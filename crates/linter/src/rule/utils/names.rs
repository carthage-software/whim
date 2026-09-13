use hashbrown::HashMap;

use whim_span::Span;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::call::Callee;
use whim_syn::cst::declaration::Use;
use whim_syn::cst::declaration::UseItem;
use whim_syn::cst::declaration::UseItems;
use whim_syn::cst::node::Node;
use whim_syn::cst::r#type::TypeParameterList;

#[derive(Debug, Clone, Default)]
pub(crate) struct Resolver {
    namespace: String,
    aliases: HashMap<String, String>,
}

impl Resolver {
    pub(crate) fn for_namespace(namespace: &str) -> Self {
        Self {
            namespace: namespace.to_owned(),
            aliases: HashMap::new(),
        }
    }

    pub(crate) fn collect_use(&mut self, declaration: &Use<'_>) {
        for_each_use_item(declaration, |_, target, alias| {
            self.aliases.insert(alias, target);
        });
    }

    pub(crate) fn resolve(&self, identifier: &Identifier<'_>) -> String {
        match identifier {
            Identifier::FullyQualified(name) => strip_leading_separator(name.value).to_owned(),
            Identifier::Local(name) => self
                .aliases
                .get(name.value)
                .cloned()
                .unwrap_or_else(|| self.qualify(name.value)),
            Identifier::Qualified(name) => {
                let (first, rest) = name
                    .value
                    .split_once('\\')
                    .expect("qualified identifiers contain a separator");
                self.aliases.get(first).map_or_else(
                    || self.qualify(name.value),
                    |target| format!("{target}\\{rest}"),
                )
            }
        }
    }

    pub(crate) fn referenced_alias<'a>(&'a self, identifier: &Identifier<'a>) -> Option<&'a str> {
        let first = match identifier {
            Identifier::Local(name) => name.value,
            Identifier::Qualified(name) => name.value.split_once('\\')?.0,
            Identifier::FullyQualified(_) => return None,
        };

        self.aliases.contains_key(first).then_some(first)
    }

    pub(crate) fn qualify(&self, name: &str) -> String {
        if self.namespace.is_empty() {
            name.to_owned()
        } else {
            format!("{}\\{name}", self.namespace)
        }
    }
}

pub(crate) fn strip_leading_separator(name: &str) -> &str {
    name.strip_prefix('\\').unwrap_or(name)
}

pub(crate) fn for_each_use_item<'ast, 'arena>(
    declaration: &'ast Use<'arena>,
    mut callback: impl FnMut(&'ast UseItem<'arena>, String, String),
) {
    let (prefix, items) = match &declaration.items {
        UseItems::Sequence(sequence) => ("", &sequence.items),
        UseItems::List(list) => (strip_leading_separator(list.namespace.value()), &list.items),
    };

    for item in items {
        let name = strip_leading_separator(item.name.value());
        let target = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}\\{name}")
        };

        let alias = item.alias.as_ref().map_or_else(
            || last_segment(&target).to_owned(),
            |alias| alias.identifier.value.to_owned(),
        );
        callback(item, target, alias);
    }
}

pub(crate) fn symbol_identifier<'a>(node: Node<'_, 'a>) -> Option<Identifier<'a>> {
    match node {
        Node::Attribute(attribute) => Some(attribute.name),
        Node::NamedType(named) => Some(named.identifier),
        Node::NamedShapeType(named) => Some(named.identifier),
        Node::NamedClassReference(named) => Some(named.identifier),
        Node::ConstantAccess(access) => Some(access.name),
        Node::FunctionCall(call) => match call.function {
            Callee::Identifier(identifier) => Some(identifier),
            Callee::Expression(_) => None,
        },
        Node::FunctionPartialApplication(call) => match call.function {
            Callee::Identifier(identifier) => Some(identifier),
            Callee::Expression(_) => None,
        },
        _ => None,
    }
}

pub(crate) fn type_parameters<'ast, 'arena>(
    node: Node<'ast, 'arena>,
) -> Option<&'ast TypeParameterList<'arena>> {
    match node {
        Node::Class(declaration) => declaration.type_parameters.as_ref(),
        Node::Interface(declaration) => declaration.type_parameters.as_ref(),
        Node::Enum(declaration) => declaration.type_parameters.as_ref(),
        Node::Function(declaration) => declaration.type_parameters.as_ref(),
        Node::Method(declaration) => declaration.type_parameters.as_ref(),
        Node::Closure(declaration) => declaration.type_parameters.as_ref(),
        Node::TypeAlias(declaration) => declaration.type_parameters.as_ref(),
        Node::Newtype(declaration) => declaration.type_parameters.as_ref(),
        _ => None,
    }
}

pub(crate) fn is_type_parameter(
    identifier: &Identifier<'_>,
    parameters: &HashMap<&str, usize>,
) -> bool {
    matches!(identifier, Identifier::Local(name) if parameters.contains_key(name.value))
}

pub(crate) fn is_type_parameter_reference(
    node: Node<'_, '_>,
    identifier: &Identifier<'_>,
    parameters: &HashMap<&str, usize>,
) -> bool {
    matches!(
        node,
        Node::NamedType(_) | Node::NamedShapeType(_) | Node::NamedClassReference(_)
    ) && is_type_parameter(identifier, parameters)
}

fn last_segment(name: &str) -> &str {
    name.rfind('\\')
        .map_or(name, |position| &name[position + 1..])
}

pub(crate) fn declaration_name<'a>(node: Node<'_, 'a>) -> Option<(&'a str, Span)> {
    match node {
        Node::Class(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::Interface(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::Enum(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::Function(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::Constant(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::TypeAlias(declaration) => Some((declaration.name.value, declaration.name.span)),
        Node::Newtype(declaration) => Some((declaration.name.value, declaration.name.span)),
        _ => None,
    }
}
