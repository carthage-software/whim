//! Type lowering: source types to complete runtime descriptors, and canonical
//! rendering.

use hashbrown::HashMap;
use whim_base::limits::MAX_TYPE_DEPTH;
use whim_bytecode::aliases::expand_aliases;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::unit::CompiledTypeAlias;
use whim_bytecode::unit::Variance;
use whim_optimizer::descriptors_equal;
use whim_span::HasSpan;
use whim_span::Span;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeParameterList;
use whim_value::heap::Heap;

use crate::error::CompileError;
use crate::error::CompileErrorKind;
use crate::names::Resolver;

pub(crate) mod aliases;
pub(crate) mod bounds;
pub(crate) mod lowering;
pub(crate) mod rendering;

pub(crate) fn descriptor_is_never(
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

pub(crate) fn descriptor_is_top(descriptor: &TypeDescriptor, depth: usize) -> bool {
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
pub(crate) enum DeclaredTypeKind {
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
    pub(crate) const fn name(self) -> &'static str {
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
    pub(crate) kind: DeclaredTypeKind,
    pub(crate) required: usize,
    pub(crate) total: usize,
    pub(crate) variances: Vec<Variance>,
    pub(crate) is_alias: bool,
    pub(crate) is_callable: bool,
    pub(crate) alias: Option<AliasExpansion<'arena>>,
}

pub(crate) struct AliasExpansion<'arena> {
    pub(crate) type_parameters: Option<&'arena TypeParameterList<'arena>>,
    pub(crate) aliased: &'arena Type<'arena>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct AliasEdge {
    pub target: String,
    pub through_union: bool,
    pub through_structure: bool,
}

#[derive(Default)]
pub(crate) struct AliasGraph {
    nodes: HashMap<String, AliasNode>,
    order: Vec<String>,
}

pub(crate) struct AliasNode {
    pub(crate) edges: Vec<AliasEdge>,
    pub(crate) span: Span,
    pub(crate) order: usize,
}

impl AliasGraph {
    pub(crate) fn insert(&mut self, name: String, edges: Vec<AliasEdge>, span: Span) {
        if let Some(node) = self.nodes.get_mut(&name) {
            node.edges = edges;
            node.span = span;
            return;
        }

        let order = self.order.len();
        self.order.push(name.clone());
        self.nodes.insert(name, AliasNode { edges, span, order });
    }

    pub(crate) fn get(&self, name: &str) -> Option<&AliasNode> {
        self.nodes.get(name)
    }

    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }
}

pub(crate) struct ClassContext {
    pub(crate) name: String,
    pub(crate) type_parameters: Vec<String>,
    pub(crate) parent: Option<String>,
    pub(crate) parent_arguments: Option<Vec<TypeDescriptor>>,
}

pub(crate) struct TypeScope<'compilation> {
    pub(crate) heap: &'compilation Heap,
    pub(crate) resolver: &'compilation Resolver,
    pub(crate) class: Option<&'compilation ClassContext>,
    pub(crate) aliases: &'compilation [CompiledTypeAlias],
    pub(crate) binders: &'compilation [String],
    pub(crate) forbidden_binders: &'compilation [String],
    pub(crate) generics: &'compilation GenericTable<'compilation>,
}

impl TypeScope<'_> {
    pub(crate) fn is_binder(&self, identifier: &Identifier<'_>) -> bool {
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
