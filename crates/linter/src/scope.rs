use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec;
use whim_syn::cst::node::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassLikeScope<'arena> {
    Class(&'arena str),
    Interface(&'arena str),
    Enum(&'arena str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionLikeScope<'arena> {
    Function(&'arena str),
    Method(&'arena str),
    Closure(Span),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'arena> {
    Namespace(&'arena str),
    ClassLike(ClassLikeScope<'arena>),
    FunctionLike(FunctionLikeScope<'arena>),
}

impl<'arena> Scope<'arena> {
    #[must_use]
    pub fn for_node(node: Node<'_, 'arena>) -> Option<Self> {
        Some(match node {
            Node::Namespace(namespace) => Self::Namespace(namespace.name.value()),
            Node::Class(class) => Self::ClassLike(ClassLikeScope::Class(class.name.value)),
            Node::Interface(interface) => {
                Self::ClassLike(ClassLikeScope::Interface(interface.name.value))
            }
            Node::Enum(enumeration) => {
                Self::ClassLike(ClassLikeScope::Enum(enumeration.name.value))
            }
            Node::Function(function) => {
                Self::FunctionLike(FunctionLikeScope::Function(function.name.value))
            }
            Node::Method(method) => {
                Self::FunctionLike(FunctionLikeScope::Method(method.name.value))
            }
            Node::Closure(closure) => {
                Self::FunctionLike(FunctionLikeScope::Closure(closure.span()))
            }
            _ => return None,
        })
    }
}

pub struct ScopeStack<'arena, A: Arena> {
    stack: Vec<'arena, Scope<'arena>, A>,
}

impl<'arena, A: Arena> ScopeStack<'arena, A> {
    pub fn new_in(arena: &'arena A) -> Self {
        Self {
            stack: Vec::with_capacity_in(4, arena),
        }
    }

    pub fn push(&mut self, scope: Scope<'arena>) {
        self.stack.push(scope);
    }

    pub fn pop(&mut self) -> Option<Scope<'arena>> {
        self.stack.pop()
    }

    #[must_use]
    pub fn get_namespace(&self) -> &'arena str {
        self.stack
            .iter()
            .rev()
            .find_map(|scope| {
                if let Scope::Namespace(name) = scope {
                    Some(*name)
                } else {
                    None
                }
            })
            .unwrap_or("")
    }

    #[must_use]
    pub fn get_class_like_scope(&self) -> Option<ClassLikeScope<'arena>> {
        self.stack.iter().rev().find_map(|scope| {
            if let Scope::ClassLike(scope) = scope {
                Some(*scope)
            } else {
                None
            }
        })
    }

    #[must_use]
    pub fn get_function_like_scope(&self) -> Option<FunctionLikeScope<'arena>> {
        self.stack.iter().rev().find_map(|scope| {
            if let Scope::FunctionLike(scope) = scope {
                Some(*scope)
            } else {
                None
            }
        })
    }
}
