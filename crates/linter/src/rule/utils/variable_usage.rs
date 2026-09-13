use hashbrown::HashMap;

use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec as ArenaVec;
use whim_syn::cst::atom::Variable;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::expression::Expression;
use whim_syn::cst::function::ClosureBody;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::node::Node;
use whim_syn::cst::operation::AssignmentOperator;
use whim_syn::cst::operation::AssignmentTarget;
use whim_syn::cst::operation::BinaryOperator;
use whim_syn::cst::operation::DestructureTarget;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::pattern::Pattern;

#[derive(Default)]
pub(crate) struct DeadStoreVarInfo {
    do_not_flag: bool,
    pending: Vec<(Span, Box<[u32]>)>,
    pub(crate) dead_stores: Vec<(Span, Span)>,
}

#[derive(Default)]
pub(crate) struct RedundantVarInfo {
    pub(crate) do_not_flag: bool,
    pub(crate) pending_write: Option<Span>,
}

#[derive(Default)]
pub(crate) struct DeadStoreRecorder<'arena> {
    pub(crate) info: HashMap<&'arena str, DeadStoreVarInfo>,
    pub(crate) redundant: HashMap<&'arena str, RedundantVarInfo>,
    arm_counter: u32,
    arm_stack: Vec<u32>,
    rescan_depth: u32,
    excluded: Vec<&'arena str>,
}

impl<'arena> DeadStoreRecorder<'arena> {
    fn declare_external(&mut self, name: &'arena str) {
        let info = self.info.entry(name).or_default();
        info.do_not_flag = true;
        info.dead_stores.clear();
        self.redundant.entry(name).or_default().do_not_flag = true;
    }

    fn protect_dead_store(&mut self, name: &'arena str) {
        if self.rescan_depth > 0 {
            return;
        }
        let info = self.info.entry(name).or_default();
        info.do_not_flag = true;
        info.dead_stores.clear();
    }

    fn record_write(&mut self, variable: Variable<'arena>, read_first: bool) {
        if self.excluded.contains(&variable.name) {
            return;
        }
        if self.rescan_depth > 0 {
            if read_first {
                self.redundant
                    .entry(variable.name)
                    .or_default()
                    .pending_write = None;
            }
            return;
        }
        let path: Box<[u32]> = self.arm_stack.iter().copied().collect();
        let info = self.info.entry(variable.name).or_default();
        if read_first {
            info.pending.clear();
        }

        let mut index = 0;
        while index < info.pending.len() {
            if info.pending[index].1.starts_with(&path) {
                let (span, _) = info.pending.swap_remove(index);
                if !info.do_not_flag {
                    info.dead_stores.push((span, variable.span));
                }
            } else {
                index += 1;
            }
        }

        info.pending.push((variable.span, path));

        self.redundant
            .entry(variable.name)
            .or_default()
            .pending_write = Some(variable.span);
    }

    fn record_read(&mut self, name: &'arena str) {
        if self.excluded.contains(&name) {
            return;
        }
        self.redundant.entry(name).or_default().pending_write = None;
        if self.rescan_depth > 0 {
            return;
        }
        self.info.entry(name).or_default().pending.clear();
    }

    fn record_final(&mut self, variable: Variable<'arena>) {
        if self.rescan_depth > 0 {
            return;
        }
        self.protect_dead_store(variable.name);
        self.redundant
            .entry(variable.name)
            .or_default()
            .pending_write = Some(variable.span);
    }

    fn enter_arm(&mut self) {
        self.arm_counter += 1;
        self.arm_stack.push(self.arm_counter);
    }

    fn exit_arm(&mut self) {
        self.arm_stack.pop();
    }

    fn record_terminator(&mut self) {
        if self.rescan_depth > 0 {
            return;
        }
        for info in self.info.values_mut() {
            info.pending
                .retain(|(_, path)| !path.starts_with(&self.arm_stack));
        }
    }

    fn enter_rescan(&mut self) {
        self.rescan_depth += 1;
    }

    fn exit_rescan(&mut self) {
        self.rescan_depth = self.rescan_depth.saturating_sub(1);
    }
}

pub(crate) struct FunctionLikeParts<'ast, 'arena> {
    pub(crate) parameter_list: &'ast ParameterList<'arena>,
    pub(crate) body: Node<'ast, 'arena>,
    pub(crate) binds_this: bool,
}

pub(crate) fn function_like_parts<'ast, 'arena>(
    node: Node<'ast, 'arena>,
) -> Option<FunctionLikeParts<'ast, 'arena>> {
    Some(match node {
        Node::Function(function) => FunctionLikeParts {
            parameter_list: &function.parameter_list,
            body: Node::Block(&function.body),
            binds_this: false,
        },
        Node::Method(method) => {
            let MethodBody::Concrete(body) = &method.body else {
                return None;
            };

            FunctionLikeParts {
                parameter_list: &method.parameter_list,
                body: Node::Block(body),
                binds_this: !method.is_static(),
            }
        }
        Node::Closure(closure) => FunctionLikeParts {
            parameter_list: &closure.parameter_list,
            body: closure_body(&closure.body),
            binds_this: true,
        },
        _ => return None,
    })
}

fn closure_body<'ast, 'arena>(body: &'ast ClosureBody<'arena>) -> Node<'ast, 'arena> {
    match body {
        ClosureBody::Block(block) => Node::Block(block),
        ClosureBody::Expression { expression, .. } => Node::Expression(expression),
    }
}

pub(crate) fn is_silenced_name(name: &str) -> bool {
    name.strip_prefix('$').unwrap_or(name).starts_with('_')
}

enum Step<'ast, 'arena> {
    Visit(Node<'ast, 'arena>),
    Target(&'ast AssignmentTarget<'arena>, bool),
    Prepare(&'ast AssignmentTarget<'arena>),
    Final(Variable<'arena>),
    EnterArm,
    ExitArm,
    Terminate,
    EnterRescan,
    ExitRescan,
    EnterExcluded(Vec<&'arena str>),
    ExitExcluded(usize),
}

pub(crate) fn analyze<'arena, A: Arena>(
    arena: &'arena A,
    parts: FunctionLikeParts<'_, 'arena>,
) -> DeadStoreRecorder<'arena> {
    let mut recorder = DeadStoreRecorder::default();
    for parameter in &parts.parameter_list.parameters {
        recorder.declare_external(parameter.variable.name);
    }

    if parts.binds_this {
        recorder.declare_external("$this");
    }

    let mut stack = ArenaVec::with_capacity_in(64, arena);
    stack.push(Step::Visit(parts.body));
    while let Some(step) = stack.pop() {
        match step {
            Step::Prepare(target) => match target {
                AssignmentTarget::Variable(_) => {}
                AssignmentTarget::Tuple(tuple) => {
                    for target in tuple.targets.iter().rev() {
                        match target {
                            DestructureTarget::Target(target) => stack.push(Step::Prepare(target)),
                            DestructureTarget::Default(default) => {
                                stack.push(Step::Prepare(&default.target))
                            }
                            DestructureTarget::Rest(rest) => {
                                if let Some(target) = &rest.target {
                                    stack.push(Step::Prepare(target));
                                }
                            }
                        }
                    }
                }
                AssignmentTarget::Dict(dict) => {
                    for entry in dict.entries.iter().rev() {
                        stack.push(Step::Prepare(&entry.target));
                        stack.push(Step::Visit(Node::Expression(entry.key)));
                    }
                }
                _ => stack.push(Step::Visit(Node::AssignmentTarget(target))),
            },
            Step::Target(target, read_first) => match target {
                AssignmentTarget::Variable(variable) => {
                    recorder.record_write(*variable, read_first)
                }
                AssignmentTarget::Tuple(tuple) => {
                    for target in tuple.targets.iter().rev() {
                        match target {
                            DestructureTarget::Target(target) => {
                                stack.push(Step::Target(target, read_first))
                            }
                            DestructureTarget::Rest(rest) => {
                                if let Some(target) = &rest.target {
                                    stack.push(Step::Target(target, read_first));
                                }
                            }
                            DestructureTarget::Default(default) => {
                                stack.push(Step::Target(&default.target, read_first));
                                stack.push(Step::ExitArm);
                                stack.push(Step::Visit(Node::Expression(default.value)));
                                stack.push(Step::EnterArm);
                            }
                        }
                    }
                }
                AssignmentTarget::Dict(dict) => {
                    for entry in dict.entries.iter().rev() {
                        stack.push(Step::Target(&entry.target, read_first));
                    }
                }
                _ => {}
            },
            Step::Final(variable) => recorder.record_final(variable),
            Step::EnterArm => recorder.enter_arm(),
            Step::ExitArm => recorder.exit_arm(),
            Step::Terminate => recorder.record_terminator(),
            Step::EnterRescan => recorder.enter_rescan(),
            Step::ExitRescan => recorder.exit_rescan(),
            Step::EnterExcluded(names) => recorder.excluded.extend(names),
            Step::ExitExcluded(count) => {
                recorder
                    .excluded
                    .truncate(recorder.excluded.len().saturating_sub(count));
            }
            Step::Visit(node) => match node {
                Node::Variable(variable) => recorder.record_read(variable.name),
                Node::Assignment(assignment) => {
                    let conditional = matches!(
                        assignment.operator,
                        AssignmentOperator::Coalesce(_)
                            | AssignmentOperator::LogicalAnd(_)
                            | AssignmentOperator::LogicalOr(_)
                    );

                    if conditional {
                        if let AssignmentTarget::Variable(variable) = &assignment.target {
                            recorder.record_read(variable.name);
                        }

                        stack.push(Step::ExitArm);
                    }

                    stack.push(Step::Target(
                        &assignment.target,
                        !assignment.operator.is_assign(),
                    ));

                    stack.push(Step::Visit(Node::Expression(assignment.value)));
                    if conditional {
                        stack.push(Step::EnterArm);
                    }

                    stack.push(Step::Prepare(&assignment.target));
                }
                Node::NullSafeMethodCall(call) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Visit(Node::ArgumentList(&call.argument_list)));
                    stack.push(Step::EnterArm);
                    stack.push(Step::Visit(Node::Expression(call.object)));
                }
                Node::UnaryPrefix(prefix)
                    if matches!(
                        prefix.operator,
                        UnaryPrefixOperator::PreIncrement(_) | UnaryPrefixOperator::PreDecrement(_)
                    ) =>
                {
                    if let Expression::Variable(variable) = prefix.operand.unparenthesized() {
                        recorder.record_write(*variable, true);
                    } else {
                        stack.push(Step::Visit(Node::Expression(prefix.operand)));
                    }
                }
                Node::UnaryPostfix(postfix) => {
                    if let Expression::Variable(variable) = postfix.operand.unparenthesized() {
                        recorder.record_write(*variable, true);
                    } else {
                        stack.push(Step::Visit(Node::Expression(postfix.operand)));
                    }
                }
                Node::If(statement) => {
                    if let Some(otherwise) = &statement.r#else {
                        stack.push(Step::ExitArm);
                        stack.push(Step::Visit(Node::Else(otherwise)));
                        stack.push(Step::EnterArm);
                    }
                    stack.push(Step::ExitArm);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterArm);
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                }
                Node::Binary(binary)
                    if matches!(
                        binary.operator,
                        BinaryOperator::And(_)
                            | BinaryOperator::Or(_)
                            | BinaryOperator::NullCoalesce(_)
                    ) =>
                {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Visit(Node::Expression(binary.rhs)));
                    stack.push(Step::EnterArm);
                    stack.push(Step::Visit(Node::Expression(binary.lhs)));
                }
                Node::While(statement) if recorder.rescan_depth > 0 => {
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                }
                Node::While(statement) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Terminate);
                    stack.push(Step::ExitRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                    stack.push(Step::EnterRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterArm);
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                }
                Node::DoWhile(statement) if recorder.rescan_depth > 0 => {
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                }
                Node::DoWhile(statement) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Terminate);
                    stack.push(Step::Visit(Node::Expression(statement.condition)));
                    stack.push(Step::ExitRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterArm);
                }
                Node::For(statement) if recorder.rescan_depth > 0 => {
                    for expression in statement.increments.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }

                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    for expression in statement.conditions.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }

                    for expression in statement.initializations.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }
                }
                Node::For(statement) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Terminate);
                    stack.push(Step::ExitRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    for expression in statement.conditions.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }

                    for expression in statement.increments.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }

                    stack.push(Step::EnterRescan);

                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterArm);
                    for expression in statement.conditions.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }

                    for expression in statement.initializations.iter().rev() {
                        stack.push(Step::Visit(Node::Expression(expression)));
                    }
                }
                Node::Foreach(statement) if recorder.rescan_depth > 0 => {
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::Target(statement.target.value(), false));
                    stack.push(Step::Prepare(statement.target.value()));
                    if let Some(key) = statement.target.key() {
                        stack.push(Step::Target(key, false));
                        stack.push(Step::Prepare(key));
                    }

                    stack.push(Step::Visit(Node::Expression(statement.expression)));
                }
                Node::Foreach(statement) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Terminate);
                    stack.push(Step::ExitRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::EnterRescan);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    stack.push(Step::Target(statement.target.value(), true));
                    stack.push(Step::Prepare(statement.target.value()));
                    if let Some(key) = statement.target.key() {
                        stack.push(Step::Target(key, true));
                        stack.push(Step::Prepare(key));
                    }
                    stack.push(Step::EnterArm);
                    stack.push(Step::Visit(Node::Expression(statement.expression)));
                }
                Node::Match(matching) => {
                    for arm in matching.arms.iter().rev() {
                        let bindings = pattern_bindings(arena, arm.pattern);
                        stack.push(Step::ExitArm);
                        stack.push(Step::ExitExcluded(bindings.len()));
                        stack.push(Step::Visit(Node::Expression(arm.expression)));
                        stack.push(Step::EnterExcluded(bindings));
                        stack.push(Step::EnterArm);
                    }
                    stack.push(Step::Visit(Node::Expression(matching.expression)));
                }
                Node::Try(statement) => {
                    for catch in statement.catch_clauses {
                        protect_variables(arena, Node::TryCatchClause(catch), &mut recorder);
                    }

                    if let Some(finally) = &statement.finally_clause {
                        protect_variables(arena, Node::Block(&finally.block), &mut recorder);
                    }

                    if let Some(finally) = &statement.finally_clause {
                        stack.push(Step::ExitArm);
                        stack.push(Step::Visit(Node::Block(&finally.block)));
                        stack.push(Step::EnterArm);
                    }

                    for catch in statement.catch_clauses.iter().rev() {
                        stack.push(Step::ExitArm);
                        stack.push(Step::Visit(Node::TryCatchClause(catch)));
                        stack.push(Step::EnterArm);
                    }

                    if let Some(otherwise) = &statement.else_clause {
                        stack.push(Step::ExitArm);
                        stack.push(Step::Visit(Node::Block(&otherwise.block)));
                        stack.push(Step::EnterArm);
                    }

                    stack.push(Step::ExitArm);
                    stack.push(Step::Visit(Node::Block(&statement.block)));
                    stack.push(Step::EnterArm);
                }
                Node::TryCatchClause(clause) => {
                    stack.push(Step::Visit(Node::Block(&clause.block)));
                    if let Some(guard) = &clause.guard {
                        stack.push(Step::Visit(Node::TryCatchGuard(guard)));
                    }

                    if let Some(variable) = clause.variable {
                        recorder.protect_dead_store(variable.name);
                        if recorder.rescan_depth == 0 {
                            recorder
                                .redundant
                                .entry(variable.name)
                                .or_default()
                                .pending_write = Some(variable.span);
                        }
                    }

                    stack.push(Step::Visit(Node::Type(clause.r#type)));
                }
                Node::Using(statement) => {
                    stack.push(Step::ExitArm);
                    stack.push(Step::Terminate);
                    stack.push(Step::Visit(Node::Block(&statement.body)));
                    for binding in statement.bindings.iter().rev() {
                        stack.push(Step::Visit(Node::UsingBinding(binding)));
                    }

                    stack.push(Step::EnterArm);
                }
                Node::UsingBinding(binding) => {
                    protect_variables(arena, Node::BindingTarget(&binding.target), &mut recorder);
                    stack.push(Step::Visit(Node::Expression(binding.value)));
                }
                Node::Pattern(pattern) => {
                    protect_variables(arena, Node::Pattern(pattern), &mut recorder)
                }
                Node::FinalLocal(local) => {
                    stack.push(Step::Final(local.variable));
                    stack.push(Step::Visit(Node::Expression(local.value)));
                }
                Node::Closure(closure) => {
                    read_captures(
                        arena,
                        closure_body(&closure.body),
                        &closure.parameter_list,
                        &mut recorder,
                    );
                }
                Node::Return(_)
                | Node::Throw(_)
                | Node::Break(_)
                | Node::Continue(_)
                | Node::ExitConstruct(_)
                | Node::PanicConstruct(_) => {
                    stack.push(Step::Terminate);
                    push_children(node, &mut stack);
                }
                Node::Function(_)
                | Node::Method(_)
                | Node::Class(_)
                | Node::Interface(_)
                | Node::Enum(_) => {}
                _ => push_children(node, &mut stack),
            },
        }
    }

    recorder
}

fn push_children<'ast, 'arena, A: Arena>(
    node: Node<'ast, 'arena>,
    stack: &mut ArenaVec<'arena, Step<'ast, 'arena>, A>,
) {
    let start = stack.len();
    node.visit_children(&mut |child| stack.push(Step::Visit(child)));
    stack[start..].reverse();
}

fn read_captures<'arena, A: Arena>(
    arena: &'arena A,
    body: Node<'_, 'arena>,
    parameters: &ParameterList<'arena>,
    recorder: &mut DeadStoreRecorder<'arena>,
) {
    let mut stack = ArenaVec::new_in(arena);
    stack.push(body);
    while let Some(node) = stack.pop() {
        match node {
            Node::Variable(variable)
                if !parameters
                    .parameters
                    .iter()
                    .any(|parameter| parameter.variable.name == variable.name) =>
            {
                recorder.record_read(variable.name)
            }
            Node::Function(_)
            | Node::Method(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Enum(_) => {}
            _ => node.visit_children(&mut |child| stack.push(child)),
        }
    }
}

fn protect_variables<'arena, A: Arena>(
    arena: &'arena A,
    root: Node<'_, 'arena>,
    recorder: &mut DeadStoreRecorder<'arena>,
) {
    let mut stack = ArenaVec::new_in(arena);
    stack.push(root);
    while let Some(node) = stack.pop() {
        match node {
            Node::Variable(variable) => recorder.protect_dead_store(variable.name),
            Node::Function(_)
            | Node::Method(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Enum(_) => {}
            _ => node.visit_children(&mut |child| stack.push(child)),
        }
    }
}

fn pattern_bindings<'arena, A: Arena>(
    arena: &'arena A,
    pattern: &Pattern<'arena>,
) -> Vec<&'arena str> {
    let mut bindings = Vec::new();
    let mut stack = ArenaVec::new_in(arena);
    stack.push(Node::Pattern(pattern));
    while let Some(node) = stack.pop() {
        match node {
            Node::Variable(variable) => bindings.push(variable.name),
            Node::Function(_)
            | Node::Method(_)
            | Node::Closure(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Enum(_) => {}
            _ => node.visit_children(&mut |child| stack.push(child)),
        }
    }
    bindings
}
