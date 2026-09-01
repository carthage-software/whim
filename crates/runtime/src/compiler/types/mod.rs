//! Type lowering: source types to complete runtime descriptors, and canonical
//! rendering.

use hashbrown::HashMap;

use whim_span::Span;
use whim_span::HasSpan;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeParameterList;

use crate::bytecode::aliases::expand_aliases;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::Variance;
use crate::optimizer::descriptors_equal;
use crate::value::heap::Heap;
use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::compiler::names::Resolver;
use crate::limits::MAX_TYPE_DEPTH;

pub(in crate::compiler) mod aliases;
pub(in crate::compiler) mod bounds;
pub(in crate::compiler) mod lowering;
pub(in crate::compiler) mod rendering;

pub(in crate::compiler) fn descriptor_is_never(
    descriptor: &TypeDescriptor,
    aliases: &[CompiledTypeAlias],
) -> bool {
    descriptor_is_bottom(&expand_aliases(descriptor, aliases), 0)
}

fn descriptor_is_bottom(descriptor: &TypeDescriptor, depth: usize) -> bool {
    if depth > MAX_TYPE_DEPTH {
        return false;
    }
    match descriptor {
        TypeDescriptor::Never => true,
        TypeDescriptor::Negated(inner) => descriptor_is_top(inner, depth + 1),
        TypeDescriptor::Union(members) => members
            .iter()
            .all(|member| descriptor_is_bottom(member, depth + 1)),
        TypeDescriptor::Intersection(members) => {
            members
                .iter()
                .any(|member| descriptor_is_bottom(member, depth + 1))
                || has_complementary_pair(members, depth + 1)
        }
        _ => false,
    }
}

pub(in crate::compiler) fn descriptor_is_top(descriptor: &TypeDescriptor, depth: usize) -> bool {
    if depth > MAX_TYPE_DEPTH {
        return false;
    }
    match descriptor {
        TypeDescriptor::Wildcard | TypeDescriptor::Mixed => true,
        TypeDescriptor::Negated(inner) => descriptor_is_bottom(inner, depth + 1),
        TypeDescriptor::Union(members) => {
            members
                .iter()
                .any(|member| descriptor_is_top(member, depth + 1))
                || has_complementary_pair(members, depth + 1)
        }
        TypeDescriptor::Intersection(members) => members
            .iter()
            .all(|member| descriptor_is_top(member, depth + 1)),
        _ => false,
    }
}

fn has_complementary_pair(members: &[TypeDescriptor], depth: usize) -> bool {
    members.iter().enumerate().any(|(index, member)| {
        members[index + 1..]
            .iter()
            .any(|other| match (member, other) {
                (TypeDescriptor::Negated(inner), other)
                | (other, TypeDescriptor::Negated(inner)) => {
                    descriptors_equal(inner, other, depth + 1)
                }
                _ => false,
            })
    })
}

pub(crate) type GenericTable<'arena> = HashMap<String, GenericDecl<'arena>>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum DeclaredTypeKind {
    Constant,
    Function,
    ClassLike,
    TypeAlias,
    Newtype,
    Method,
    ClassConstant,
    EnumCase,
}

impl DeclaredTypeKind {
    pub(in crate::compiler) const fn name(self) -> &'static str {
        match self {
            Self::Constant => "constant",
            Self::Function => "function",
            Self::ClassLike => "class-like",
            Self::TypeAlias => "type alias",
            Self::Newtype => "newtype",
            Self::Method => "method",
            Self::ClassConstant => "class constant",
            Self::EnumCase => "enum case",
        }
    }
}

pub(crate) struct GenericDecl<'arena> {
    pub(in crate::compiler) kind: DeclaredTypeKind,
    pub(in crate::compiler) required: usize,
    pub(in crate::compiler) total: usize,
    pub(in crate::compiler) variances: Vec<Variance>,
    pub(in crate::compiler) is_alias: bool,
    pub(in crate::compiler) is_callable: bool,
    pub(in crate::compiler) alias: Option<AliasExpansion<'arena>>,
}

pub(in crate::compiler) struct AliasExpansion<'arena> {
    pub(in crate::compiler) type_parameters: Option<&'arena TypeParameterList<'arena>>,
    pub(in crate::compiler) aliased: &'arena Type<'arena>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AliasEdge {
    pub target: String,
    pub through_union: bool,
    pub through_collection: bool,
}

#[derive(Default)]
pub(crate) struct AliasGraph {
    nodes: HashMap<String, AliasNode>,
    order: Vec<String>,
}

pub(in crate::compiler) struct AliasNode {
    pub(in crate::compiler) edges: Vec<AliasEdge>,
    pub(in crate::compiler) span: Span,
    pub(in crate::compiler) order: usize,
}

impl AliasGraph {
    pub(in crate::compiler) fn insert(&mut self, name: String, edges: Vec<AliasEdge>, span: Span) {
        if let Some(node) = self.nodes.get_mut(&name) {
            node.edges = edges;
            node.span = span;
            return;
        }

        let order = self.order.len();
        self.order.push(name.clone());
        self.nodes.insert(name, AliasNode { edges, span, order });
    }

    pub(in crate::compiler) fn get(&self, name: &str) -> Option<&AliasNode> {
        self.nodes.get(name)
    }

    pub(in crate::compiler) fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }
}

pub(in crate::compiler) struct ClassContext {
    pub(in crate::compiler) name: String,
    pub(in crate::compiler) type_parameters: Vec<String>,
    pub(in crate::compiler) parent: Option<String>,
    pub(in crate::compiler) parent_arguments: Option<Vec<TypeDescriptor>>,
}

pub(in crate::compiler) struct TypeScope<'compilation> {
    pub(in crate::compiler) heap: &'compilation Heap,
    pub(in crate::compiler) resolver: &'compilation Resolver,
    pub(in crate::compiler) class: Option<&'compilation ClassContext>,
    pub(in crate::compiler) aliases: &'compilation [CompiledTypeAlias],
    pub(in crate::compiler) binders: &'compilation [String],
    pub(in crate::compiler) forbidden_binders: &'compilation [String],
    pub(in crate::compiler) generics: &'compilation GenericTable<'compilation>,
}

impl TypeScope<'_> {
    pub(in crate::compiler) fn is_binder(&self, identifier: &Identifier<'_>) -> bool {
        matches!(
            identifier,
            Identifier::Local(local) if self.binders.iter().any(|binder| binder == local.value)
        )
    }

    fn is_forbidden_binder(&self, identifier: &Identifier<'_>) -> bool {
        matches!(
            identifier,
            Identifier::Local(local) if self.forbidden_binders.iter().any(|binder| binder == local.value)
        )
    }
}

impl TypeScope<'_> {
    fn class_name(&self, source: &Type<'_>) -> Result<String, CompileError> {
        self.class.map_or_else(
            || {
                Err(CompileError::new(
                    CompileErrorKind::ClassContextRequired,
                    "`self` refers to the enclosing class, and there is none here",
                    source.span(),
                ))
            },
            |class| Ok(class.name.clone()),
        )
    }

    fn parent_name(&self, source: &Type<'_>) -> Result<String, CompileError> {
        let Some(class) = self.class else {
            return Err(CompileError::new(
                CompileErrorKind::ClassContextRequired,
                "`parent` refers to the enclosing class's parent, and there is no class here",
                source.span(),
            ));
        };
        class.parent.as_ref().map_or_else(
            || {
                Err(CompileError::new(
                    CompileErrorKind::ClassContextRequired,
                    format!(
                        "`parent` refers to the enclosing class's parent, but {} has no parent",
                        class.name
                    ),
                    source.span(),
                ))
            },
            |parent| Ok(parent.clone()),
        )
    }
}
