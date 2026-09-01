//! The variable analysis a synthesized body is built from.

use hashbrown::HashSet;

use whim_syn::cst::binding::BindingTarget as BindTarget;
use whim_syn::cst::binding::ElementBindingTarget as BindElement;
use whim_syn::cst::function::ShortClosure;
use whim_syn::cst::function::ShortClosureBody;
use whim_syn::cst::node::Node;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::pattern::Pattern;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::compiler::emit::AssignmentTarget;
use crate::compiler::emit::Block;
use crate::compiler::emit::DestructureTarget;
use crate::compiler::emit::Expression;
use crate::compiler::emit::ParameterList;
use crate::compiler::emit::Span;
use crate::compiler::emit::Statement;

#[derive(Default)]
struct Names<'arena> {
    ordered: Vec<&'arena str>,
    seen: HashSet<&'arena str>,
}

impl<'arena> Names<'arena> {
    fn insert(&mut self, name: &'arena str) {
        if self.seen.insert(name) {
            self.ordered.push(name);
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.seen.contains(name)
    }

    fn into_owned(self) -> Vec<String> {
        self.ordered.into_iter().map(str::to_string).collect()
    }
}

struct AssignedNames<'names, 'arena> {
    names: &'names mut Names<'arena>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for AssignedNames<'_, 'arena> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        match node {
            Node::Statement(Statement::Using(using)) => {
                for binding in &using.bindings {
                    collect_bind_target_names(&binding.target, self.names);
                }

                Flow::Descend
            }
            Node::Assignment(assignment) => {
                collect_target_names(&assignment.target, self.names);

                Flow::Descend
            }
            Node::UnaryPrefix(unary)
                if matches!(
                    unary.operator,
                    UnaryPrefixOperator::PreIncrement(_) | UnaryPrefixOperator::PreDecrement(_)
                ) =>
            {
                collect_incremented_name(unary.operand, self.names);
                Flow::Skip
            }
            Node::UnaryPostfix(unary) => {
                collect_incremented_name(unary.operand, self.names);
                Flow::Skip
            }
            Node::DropConstruct(drop) => {
                for variable in drop.variables {
                    self.names.insert(variable.name);
                }

                Flow::Skip
            }
            Node::Pattern(Pattern::Variable(variable)) => {
                self.names.insert(variable.name);
                Flow::Skip
            }
            Node::AssignmentTarget(_)
            | Node::BindingTarget(_)
            | Node::Closure(_)
            | Node::ShortClosure(_) => Flow::Skip,
            _ => Flow::Descend,
        }
    }
}

pub(in crate::compiler::emit) fn collect_assigned_in_expression(
    expression: &Expression<'_>,
) -> Vec<String> {
    let mut names = Names::default();
    walk(
        Node::Expression(expression),
        &mut AssignedNames { names: &mut names },
    );
    names.into_owned()
}

pub(in crate::compiler) fn collect_assigned_in_statements(
    statements: &[Statement<'_>],
) -> Vec<String> {
    let mut names = Names::default();
    for statement in statements {
        walk(
            Node::Statement(statement),
            &mut AssignedNames { names: &mut names },
        );
    }
    names.into_owned()
}

fn collect_incremented_name<'arena>(expression: &Expression<'arena>, names: &mut Names<'arena>) {
    if let Expression::Variable(variable) = expression.unparenthesized() {
        names.insert(variable.name);
    }
}

fn collect_target_names<'arena>(target: &AssignmentTarget<'arena>, names: &mut Names<'arena>) {
    match target {
        AssignmentTarget::Variable(variable) => {
            if variable.name != "$this" {
                names.insert(variable.name);
            }
        }
        AssignmentTarget::Tuple(destructure) => {
            for element in &destructure.targets {
                match element {
                    DestructureTarget::Target(inner) => collect_target_names(inner, names),
                    DestructureTarget::Default(default) => {
                        collect_target_names(&default.target, names);
                    }
                    DestructureTarget::Rest(rest) => {
                        if let Some(inner) = &rest.target {
                            collect_target_names(inner, names);
                        }
                    }
                }
            }
        }
        AssignmentTarget::Dict(destructure) => {
            for entry in &destructure.entries {
                collect_target_names(&entry.target, names);
            }
        }
        AssignmentTarget::Property(_)
        | AssignmentTarget::StaticProperty(_)
        | AssignmentTarget::ArrayIndex(_)
        | AssignmentTarget::ArrayAppend(_) => {}
    }
}

fn collect_bind_target_names<'arena>(target: &BindTarget<'arena>, names: &mut Names<'arena>) {
    match target {
        BindTarget::Variable(variable) => {
            if variable.name != "$this" {
                names.insert(variable.name);
            }
        }
        BindTarget::Tuple(tuple) => {
            for element in &tuple.targets {
                match element {
                    BindElement::Target(target) => collect_bind_target_names(target, names),
                    BindElement::Rest(rest) => {
                        if let Some(target) = &rest.target {
                            collect_bind_target_names(target, names);
                        }
                    }
                }
            }
        }
        BindTarget::Dict(dict) => {
            for entry in &dict.entries {
                collect_bind_target_names(&entry.target, names);
            }
        }
    }
}

fn collect_scoped_bindings(node: Node<'_, '_>, bindings: &mut Vec<(String, Span)>) {
    struct ScopedBindings<'bindings> {
        bindings: &'bindings mut Vec<(String, Span)>,
    }

    impl<'ast, 'arena> Visitor<'ast, 'arena> for ScopedBindings<'_> {
        fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
            match node {
                Node::Pattern(Pattern::Variable(variable)) => {
                    self.bindings
                        .push((variable.name.to_string(), variable.span));
                    Flow::Skip
                }
                Node::BindingTarget(_) | Node::Closure(_) | Node::ShortClosure(_) => Flow::Skip,
                _ => Flow::Descend,
            }
        }
    }

    walk(node, &mut ScopedBindings { bindings });
}

pub(in crate::compiler::emit) fn collect_scoped_bindings_in_statements(
    statements: &[Statement<'_>],
    bindings: &mut Vec<(String, Span)>,
) {
    for statement in statements {
        collect_scoped_bindings(Node::Statement(statement), bindings);
    }
}

pub(in crate::compiler) fn collect_scoped_bindings_in_statement(
    statement: &Statement<'_>,
    bindings: &mut Vec<(String, Span)>,
) {
    collect_scoped_bindings(Node::Statement(statement), bindings);
}

pub(in crate::compiler::emit) fn collect_scoped_bindings_in_expression(
    expression: &Expression<'_>,
    bindings: &mut Vec<(String, Span)>,
) {
    collect_scoped_bindings(Node::Expression(expression), bindings);
}

fn collect_local_names<'arena>(node: Node<'_, 'arena>, names: &mut Names<'arena>) {
    struct LocalNames<'names, 'arena> {
        names: &'names mut Names<'arena>,
    }

    impl<'ast, 'arena> Visitor<'ast, 'arena> for LocalNames<'_, 'arena> {
        fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
            match node {
                Node::Statement(Statement::Using(using)) => {
                    for binding in &using.bindings {
                        collect_bind_target_names(&binding.target, self.names);
                    }

                    Flow::Descend
                }
                Node::Assignment(assignment) if assignment.operator.is_assign() => {
                    collect_target_names(&assignment.target, self.names);

                    Flow::Descend
                }
                Node::Foreach(r#foreach) => {
                    if let Some(key) = r#foreach.target.key() {
                        collect_target_names(key, self.names);
                    }
                    collect_target_names(r#foreach.target.value(), self.names);

                    Flow::Descend
                }
                Node::FinalLocal(local) => {
                    self.names.insert(local.variable.name);

                    Flow::Descend
                }
                Node::TryCatchClause(clause) => {
                    if let Some(variable) = &clause.variable {
                        self.names.insert(variable.name);
                    }

                    Flow::Descend
                }
                Node::Pattern(Pattern::Variable(variable)) => {
                    self.names.insert(variable.name);
                    Flow::Skip
                }
                Node::Closure(_)
                | Node::ShortClosure(_)
                | Node::Function(_)
                | Node::Class(_)
                | Node::Interface(_)
                | Node::Enum(_) => Flow::Skip,
                _ => Flow::Descend,
            }
        }
    }

    walk(node, &mut LocalNames { names });
}

fn collect_short_closure_free_variables<'arena>(
    closure: &ShortClosure<'arena>,
    names: &mut Names<'arena>,
) {
    let mut inner = Names::default();
    let mut locals = Names::default();
    let body = match &closure.body {
        ShortClosureBody::Expression { expression, .. } => Node::Expression(expression),
        ShortClosureBody::Block(block) => Node::Block(block),
    };
    collect_variables(body, &mut inner);
    collect_local_names(body, &mut locals);
    inner.ordered.retain(|name| !locals.contains(name));
    merge_unbound(&closure.parameter_list, inner, names);
}

pub(in crate::compiler::emit) fn collect_free_variables_in_expression(
    expression: &Expression<'_>,
) -> Vec<String> {
    let mut names = Names::default();
    let mut locals = Names::default();
    collect_variables(Node::Expression(expression), &mut names);
    collect_local_names(Node::Expression(expression), &mut locals);
    names.ordered.retain(|name| !locals.contains(name));
    names.into_owned()
}

pub(in crate::compiler::emit) fn collect_free_variables_in_statements(
    statements: &[Statement<'_>],
) -> Vec<String> {
    let mut names = Names::default();
    let mut locals = Names::default();
    for statement in statements {
        collect_variables(Node::Statement(statement), &mut names);
        collect_local_names(Node::Statement(statement), &mut locals);
    }
    names.ordered.retain(|name| !locals.contains(name));
    names.into_owned()
}

struct ReferencedNames<'names, 'arena> {
    names: &'names mut Names<'arena>,
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for ReferencedNames<'_, 'arena> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        match node {
            Node::Statement(Statement::Using(using)) => {
                for binding in &using.bindings {
                    collect_bind_target_names(&binding.target, self.names);
                }

                Flow::Descend
            }
            Node::StaticPropertyAccess(access) => {
                collect_variables(Node::ClassReference(&access.class), self.names);

                Flow::Skip
            }
            Node::Variable(variable) => {
                self.names.insert(variable.name);

                Flow::Skip
            }
            Node::Assignment(assignment) => {
                collect_target_names(&assignment.target, self.names);

                Flow::Descend
            }
            Node::ShortClosure(closure) => {
                collect_short_closure_free_variables(closure, self.names);

                Flow::Skip
            }
            Node::Closure(closure) => {
                if let Some(use_clause) = &closure.use_clause {
                    for variable in use_clause.variables {
                        self.names.insert(variable.name);
                    }
                }
                if references_this_in_block(&closure.body) {
                    self.names.insert("$this");
                }

                Flow::Skip
            }
            Node::Pattern(Pattern::Variable(_))
            | Node::BindingTarget(_)
            | Node::Function(_)
            | Node::Class(_)
            | Node::Interface(_)
            | Node::Enum(_) => Flow::Skip,
            _ => Flow::Descend,
        }
    }
}

fn collect_variables<'arena>(node: Node<'_, 'arena>, names: &mut Names<'arena>) {
    walk(node, &mut ReferencedNames { names });
}

/// Whether a block references `$this`, for automatic capture.
pub(in crate::compiler) fn references_this_in_block(block: &Block<'_>) -> bool {
    let mut names = Names::default();
    collect_variables(Node::Block(block), &mut names);

    names.contains("$this")
}

pub(in crate::compiler::emit) fn collect_variables_in_statements(
    statements: &[Statement<'_>],
) -> Vec<String> {
    let mut names = Names::default();
    for statement in statements {
        collect_variables(Node::Statement(statement), &mut names);
    }

    names.into_owned()
}

pub(in crate::compiler::emit) fn collect_variables_in_expression(
    expression: &Expression<'_>,
) -> Vec<String> {
    let mut names = Names::default();
    collect_variables(Node::Expression(expression), &mut names);
    names.into_owned()
}

pub(in crate::compiler) fn collect_variables_in_statement(
    statement: &Statement<'_>,
) -> Vec<String> {
    let mut names = Names::default();
    collect_variables(Node::Statement(statement), &mut names);
    names.into_owned()
}

/// Merges a lambda's free variables into `names`: the names its body reads,
/// less the ones its own parameters bind.
fn merge_unbound<'arena>(
    parameter_list: &ParameterList<'arena>,
    inner: Names<'arena>,
    names: &mut Names<'arena>,
) {
    for name in inner.ordered {
        if parameter_list
            .parameters
            .iter()
            .any(|parameter| parameter.variable.name == name)
        {
            continue;
        }

        names.insert(name);
    }
}
