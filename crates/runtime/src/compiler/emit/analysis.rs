//! The variable analysis a synthesized body is built from.

use hashbrown::HashSet;

use whim_syn::cst::binding::BindingTarget as BindTarget;
use whim_syn::cst::binding::ElementBindingTarget as BindElement;
use whim_syn::cst::function::Closure;
use whim_syn::cst::function::ClosureBody;
use whim_syn::cst::node::Node;
use whim_syn::cst::operation::UnaryPrefixOperator;
use whim_syn::cst::pattern::ObjectPatternEntry;
use whim_syn::cst::pattern::Pattern;
use whim_syn::cst::walker::Flow;
use whim_syn::cst::walker::Visitor;
use whim_syn::cst::walker::walk;

use crate::compiler::emit::AssignmentTarget;
use crate::compiler::emit::DestructureTarget;
use crate::compiler::emit::Expression;
use crate::compiler::emit::ParameterList;
use crate::compiler::emit::Span;
use crate::compiler::emit::Statement;

#[derive(Clone, Default)]
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
            Node::Pattern(Pattern::Variable(variable))
            | Node::ObjectPatternEntry(ObjectPatternEntry::Shorthand(variable)) => {
                self.names.insert(variable.name);
                Flow::Skip
            }
            Node::AssignmentTarget(_) | Node::BindingTarget(_) | Node::Closure(_) => Flow::Skip,
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
                Node::Pattern(Pattern::Variable(variable))
                | Node::ObjectPatternEntry(ObjectPatternEntry::Shorthand(variable)) => {
                    self.bindings
                        .push((variable.name.to_string(), variable.span));
                    Flow::Skip
                }
                Node::BindingTarget(_) | Node::Closure(_) => Flow::Skip,
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
                Node::Pattern(Pattern::Variable(variable))
                | Node::ObjectPatternEntry(ObjectPatternEntry::Shorthand(variable)) => {
                    self.names.insert(variable.name);
                    Flow::Skip
                }
                Node::Closure(_)
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

fn collect_closure_free_variables<'arena>(closure: &Closure<'arena>, names: &mut Names<'arena>) {
    let mut inner = Names::default();
    let mut locals = Names::default();
    let body = match &closure.body {
        ClosureBody::Expression { expression, .. } => Node::Expression(expression),
        ClosureBody::Block(block) => Node::Block(block),
    };

    collect_local_names(body, &mut locals);
    walk(
        body,
        &mut ReferencedNames {
            names: &mut inner,
            locals: Some(locals),
        },
    );

    merge_unbound(&closure.parameter_list, inner, names);
}

pub(in crate::compiler) fn closure_has_captures(closure: &Closure<'_>) -> bool {
    let mut names = Names::default();
    collect_closure_free_variables(closure, &mut names);
    !names.ordered.is_empty()
}

pub(in crate::compiler::emit) fn collect_free_variables_in_expression(
    expression: &Expression<'_>,
) -> Vec<String> {
    let mut names = Names::default();
    let mut locals = Names::default();
    collect_local_names(Node::Expression(expression), &mut locals);
    walk(
        Node::Expression(expression),
        &mut ReferencedNames {
            names: &mut names,
            locals: Some(locals),
        },
    );

    names.into_owned()
}

pub(in crate::compiler::emit) fn collect_free_variables_in_statements(
    statements: &[Statement<'_>],
) -> Vec<String> {
    let mut names = Names::default();
    let mut locals = Names::default();
    for statement in statements {
        collect_local_names(Node::Statement(statement), &mut locals);
    }

    let mut visitor = ReferencedNames {
        names: &mut names,
        locals: Some(locals),
    };

    for statement in statements {
        walk(Node::Statement(statement), &mut visitor);
    }

    names.into_owned()
}

struct ReferencedNames<'names, 'arena> {
    names: &'names mut Names<'arena>,
    locals: Option<Names<'arena>>,
}

impl<'arena> ReferencedNames<'_, 'arena> {
    fn reference(&mut self, name: &'arena str) {
        if !self
            .locals
            .as_ref()
            .is_some_and(|locals| locals.contains(name))
        {
            self.names.insert(name);
        }
    }

    fn branches<'ast>(&mut self, branches: impl IntoIterator<Item = Node<'ast, 'arena>>)
    where
        'arena: 'ast,
    {
        let before = self.locals.clone();
        let mut merged = before.clone();
        for branch in branches {
            self.locals = before.clone();
            walk(branch, self);
            if let (Some(merged), Some(locals)) = (&mut merged, &self.locals) {
                for name in &locals.ordered {
                    merged.insert(name);
                }
            }
        }

        self.locals = merged;
    }
}

impl<'ast, 'arena> Visitor<'ast, 'arena> for ReferencedNames<'_, 'arena> {
    fn enter(&mut self, node: Node<'ast, 'arena>) -> Flow {
        match node {
            Node::If(statement) if self.locals.is_some() => {
                walk(Node::Expression(statement.condition), self);
                self.branches(
                    [
                        Some(Node::Block(&statement.body)),
                        statement.r#else.as_ref().map(Node::Else),
                    ]
                    .into_iter()
                    .flatten(),
                );

                Flow::Skip
            }
            Node::Match(expression) if self.locals.is_some() => {
                walk(Node::Expression(expression.expression), self);
                self.branches(expression.arms.iter().map(Node::MatchArm));

                Flow::Skip
            }
            Node::For(statement) if self.locals.is_some() => {
                for expression in statement
                    .initializations
                    .iter()
                    .chain(statement.conditions.iter())
                {
                    walk(Node::Expression(expression), self);
                }
                walk(Node::Block(&statement.body), self);
                for expression in &statement.increments {
                    walk(Node::Expression(expression), self);
                }

                Flow::Skip
            }
            Node::Statement(Statement::Using(using)) => {
                if self.locals.is_none() {
                    for binding in &using.bindings {
                        collect_bind_target_names(&binding.target, self.names);
                    }
                }

                Flow::Descend
            }
            Node::StaticPropertyAccess(access) => {
                walk(Node::ClassReference(&access.class), self);

                Flow::Skip
            }
            Node::Variable(variable) => {
                self.reference(variable.name);

                Flow::Skip
            }
            Node::Assignment(assignment) => {
                if self.locals.is_some() && assignment.operator.is_assign() {
                    walk(Node::Expression(assignment.value), self);
                    if let Some(locals) = &mut self.locals {
                        collect_target_names(&assignment.target, locals);
                    }

                    walk(Node::AssignmentTarget(&assignment.target), self);

                    return Flow::Skip;
                }

                if self.locals.is_none() {
                    collect_target_names(&assignment.target, self.names);
                }

                Flow::Descend
            }
            Node::Closure(closure) => {
                let mut inner = Names::default();
                collect_closure_free_variables(closure, &mut inner);
                for name in inner.ordered {
                    self.reference(name);
                }

                Flow::Skip
            }
            Node::Pattern(Pattern::Variable(_))
            | Node::ObjectPatternEntry(ObjectPatternEntry::Shorthand(_))
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
    walk(
        node,
        &mut ReferencedNames {
            names,
            locals: None,
        },
    );
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
