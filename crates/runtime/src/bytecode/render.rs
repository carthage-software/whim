//! Canonical rendering of bytecode type descriptors.

use crate::bytecode::chunk::descriptors::ShapeKey;
use crate::bytecode::chunk::descriptors::TypeDescriptor;

pub(crate) fn type_descriptor(
    descriptor: &TypeDescriptor,
    render_float: &impl Fn(f64) -> String,
) -> String {
    Renderer { render_float }.render(descriptor)
}

struct Renderer<'renderer, F> {
    render_float: &'renderer F,
}

impl<F> Renderer<'_, F>
where
    F: Fn(f64) -> String,
{
    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive match keeps every descriptor spelling together"
    )]
    fn render(&self, descriptor: &TypeDescriptor) -> String {
        let render = |descriptor: &TypeDescriptor| self.render(descriptor);
        let joined = |members: &[TypeDescriptor], separator: &str| {
            members
                .iter()
                .map(|member| self.render(member))
                .collect::<Vec<_>>()
                .join(separator)
        };

        match descriptor {
            TypeDescriptor::Wildcard => "_".to_string(),
            TypeDescriptor::Mixed => "mixed".to_string(),
            TypeDescriptor::Void => "void".to_string(),
            TypeDescriptor::Never => "never".to_string(),
            TypeDescriptor::Null => "null".to_string(),
            TypeDescriptor::Bool => "bool".to_string(),
            TypeDescriptor::Int => "int".to_string(),
            TypeDescriptor::Float => "float".to_string(),
            TypeDescriptor::String => "string".to_string(),
            TypeDescriptor::StringLength { min, max } => match max {
                Some(max) if min == max => format!("string[{min}]"),
                Some(max) => format!("string[{min}..={max}]"),
                None => format!("string[{min}..]"),
            },
            TypeDescriptor::Object => "object".to_string(),
            TypeDescriptor::TrueLiteral => "true".to_string(),
            TypeDescriptor::FalseLiteral => "false".to_string(),
            TypeDescriptor::IntLiteral(value) => value.to_string(),
            TypeDescriptor::IntRange { min, max } => match (min, max) {
                (Some(min), Some(max)) => format!("{min}..={max}"),
                (Some(min), None) => format!("{min}.."),
                (None, Some(max)) => format!("..={max}"),
                (None, None) => "int".to_string(),
            },
            TypeDescriptor::FloatLiteral(value) => (self.render_float)(*value),
            TypeDescriptor::StringLiteral(atom) => {
                format!("'{}'", atom.to_string_lossy())
            }
            TypeDescriptor::Named {
                name, arguments, ..
            } => arguments.as_ref().map_or_else(
                || name.to_string(),
                |arguments| format!("{name}<{}>", joined(arguments, ", ")),
            ),
            TypeDescriptor::Member {
                class,
                class_arguments,
                member,
                member_arguments,
            } => {
                let class = class_arguments.as_ref().map_or_else(
                    || class.to_string(),
                    |arguments| format!("{class}<{}>", joined(arguments, ", ")),
                );
                member_arguments.as_ref().map_or_else(
                    || format!("{class}::{member}"),
                    |arguments| format!("{class}::{member}<{}>", joined(arguments, ", ")),
                )
            }
            TypeDescriptor::Parameter(name) => name.to_string(),
            TypeDescriptor::StaticClass => "static".to_string(),
            TypeDescriptor::Array(Some((key, value))) => {
                format!("array<{}, {}>", render(key), render(value))
            }
            TypeDescriptor::Array(None) => "array".to_string(),
            TypeDescriptor::Vector(Some(element)) => format!("vec<{}>", render(element)),
            TypeDescriptor::Vector(None) => "vec".to_string(),
            TypeDescriptor::VectorShape { elements, rest } => {
                let mut parts = elements.iter().map(render).collect::<Vec<_>>();
                if let Some(rest) = rest {
                    parts.push(format!("...{}", render(rest)));
                } else if parts.is_empty() {
                    return "vec[...]".to_string();
                }

                format!("vec[{}]", parts.join(", "))
            }
            TypeDescriptor::Dictionary(Some((key, value))) => {
                format!("dict<{}, {}>", render(key), render(value))
            }
            TypeDescriptor::Dictionary(None) => "dict".to_string(),
            TypeDescriptor::DictionaryShape { entries, rest } => {
                let mut parts = entries
                    .iter()
                    .map(|(key, value)| {
                        let key = match key {
                            ShapeKey::Int(key) => key.to_string(),
                            ShapeKey::String(key) => {
                                format!("'{}'", key.to_string_lossy())
                            }
                        };
                        format!("{key} => {}", render(value))
                    })
                    .collect::<Vec<_>>();
                if let Some((key, value)) = rest {
                    parts.push(format!("...<{}, {}>", render(key), render(value)));
                } else if parts.is_empty() {
                    return "dict[...]".to_string();
                }

                format!("dict[{}]", parts.join(", "))
            }
            TypeDescriptor::Callable(Some(signature)) => {
                let parameters = signature
                    .parameters
                    .iter()
                    .map(|parameter| {
                        format!(
                            "{}{}",
                            if parameter.optional { "=" } else { "" },
                            render(&parameter.r#type)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                format!("fn({parameters}): {}", render(&signature.return_type))
            }
            TypeDescriptor::Callable(None) => "fn".to_string(),
            TypeDescriptor::Classname(inner) => format!("classname<{}>", render(inner)),
            TypeDescriptor::Tuple(members) if members.len() == 1 => {
                format!("({},)", render(&members[0]))
            }
            TypeDescriptor::Tuple(members) => format!("({})", joined(members, ", ")),
            TypeDescriptor::TupleRest { elements, rest } => {
                let mut members = elements.iter().map(render).collect::<Vec<_>>();
                members.push(format!("...{}", render(rest)));
                format!("({})", members.join(", "))
            }
            TypeDescriptor::TupleAny => "tuple".to_string(),
            TypeDescriptor::Union(members) => joined(members, "|"),
            TypeDescriptor::Intersection(members) => members
                .iter()
                .map(|member| {
                    let rendered = render(member);
                    if matches!(member, TypeDescriptor::Union(_)) {
                        format!("({rendered})")
                    } else {
                        rendered
                    }
                })
                .collect::<Vec<_>>()
                .join("&"),
            TypeDescriptor::Negated(inner) => self.render_unary("!", inner),
        }
    }

    fn render_unary(&self, operator: &str, inner: &TypeDescriptor) -> String {
        let rendered = self.render(inner);
        if matches!(
            inner,
            TypeDescriptor::Union(_) | TypeDescriptor::Intersection(_)
        ) {
            format!("{operator}({rendered})")
        } else {
            format!("{operator}{rendered}")
        }
    }
}
