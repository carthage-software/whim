//! Formatting for declarations: namespaces, imports, constants, type aliases,
//! attributes, class-likes and their members, functions, parameters, and types.

use std::vec::Vec;

use whim_span::HasSpan;
use whim_syn::arena::Arena;
use whim_syn::arena::Vec as ArenaVec;

use whim_syn::cst::atom::Identifier;
use whim_syn::cst::class::Class;
use whim_syn::cst::class::ClassLikeConstant;
use whim_syn::cst::class::ClassLikeMember;
use whim_syn::cst::class::Enum;
use whim_syn::cst::class::EnumCase;
use whim_syn::cst::class::Interface;
use whim_syn::cst::class::Method;
use whim_syn::cst::class::MethodBody;
use whim_syn::cst::class::Property;
use whim_syn::cst::declaration::Attribute;
use whim_syn::cst::declaration::AttributeList;
use whim_syn::cst::declaration::Constant;
use whim_syn::cst::declaration::Namespace;
use whim_syn::cst::declaration::NamespaceBody;
use whim_syn::cst::declaration::Use;
use whim_syn::cst::declaration::UseItem;
use whim_syn::cst::declaration::UseItems;
use whim_syn::cst::function::Function;
use whim_syn::cst::function::Parameter;
use whim_syn::cst::function::ParameterList;
use whim_syn::cst::r#type::DictShapeTypeEntry;
use whim_syn::cst::r#type::FunctionTypeParameter;
use whim_syn::cst::r#type::IntegerRangeBound;
use whim_syn::cst::r#type::IntegerRangeOperator;
use whim_syn::cst::r#type::NamedType;
use whim_syn::cst::r#type::NegativeLiteralType;
use whim_syn::cst::r#type::Newtype;
use whim_syn::cst::r#type::Type;
use whim_syn::cst::r#type::TypeAlias;
use whim_syn::cst::r#type::TypeArgument;
use whim_syn::cst::r#type::TypeParameter;
use whim_syn::cst::r#type::TypeVariance;

use crate::document::Document;
use crate::document::Group;
use crate::format::Format;
use crate::format::FormatterState;

/// Collects the members of a left-nested union into source order, stopping at
/// any non-union type (so an intersection member stays a single member).
fn flatten_union<'arena>(r#type: &'arena Type<'arena>, out: &mut Vec<&'arena Type<'arena>>) {
    if let Type::Union(union) = r#type {
        flatten_union(union.left, out);
        out.push(union.right);
    } else {
        out.push(r#type);
    }
}

/// Collects the members of a left-nested intersection into source order.
fn flatten_intersection<'arena>(r#type: &'arena Type<'arena>, out: &mut Vec<&'arena Type<'arena>>) {
    if let Type::Intersection(intersection) = r#type {
        flatten_intersection(intersection.left, out);
        out.push(intersection.right);
    } else {
        out.push(r#type);
    }
}

impl<'arena, A> FormatterState<'arena, A>
where
    A: Arena,
{
    fn format_return_type_suffix(
        &mut self,
        r#type: Option<&'arena Type<'arena>>,
    ) -> Document<'arena, A> {
        match r#type {
            Some(r#type) => {
                let r#type = r#type.format(self);
                self.concat([self.text(": "), r#type])
            }
            None => self.empty(),
        }
    }

    fn format_class_like_clause<T>(
        &mut self,
        keyword: &'arena str,
        types: &[T],
    ) -> Document<'arena, A>
    where
        T: Format<'arena, A>,
    {
        let mut tail = self.vec();
        tail.push(self.line());
        for (index, r#type) in types.iter().enumerate() {
            if index != 0 {
                tail.push(self.text(","));
                tail.push(self.line());
            }

            tail.push(r#type.format(self));
        }

        let mut contents = self.vec();
        contents.push(self.text(keyword));
        contents.push(self.indent(tail));
        Document::Array(contents)
    }

    fn finish_class_like(
        &self,
        attributes: Document<'arena, A>,
        header: ArenaVec<'arena, Document<'arena, A>, A>,
        body: Document<'arena, A>,
    ) -> Document<'arena, A> {
        self.concat([
            attributes,
            Document::Group(Group::new(header)),
            self.space(),
            body,
        ])
    }
}

impl<'arena, A> Format<'arena, A> for Namespace<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match &self.body {
            NamespaceBody::Implicit(body) => {
                let close = body
                    .statements
                    .last()
                    .map_or(body.semicolon.end.offset, |statement| {
                        statement.span().end.offset
                    });
                let items = f.format_sequence(body.statements, close);

                let mut parts = f.vec();
                parts.push(f.text("namespace"));
                parts.push(f.space());
                parts.push(f.text(self.name.value()));
                parts.push(f.text(";"));

                if !items.is_empty() {
                    parts.push(f.hard_line());
                    parts.push(f.hard_line());
                    parts.extend(items);
                }

                Document::Array(parts)
            }
            NamespaceBody::BraceDelimited(block) => {
                let body =
                    f.format_braced_sequence(block.statements, block.right_brace.start.offset);

                let mut parts = f.vec();
                parts.push(f.text("namespace"));
                parts.push(f.space());
                parts.push(f.text(self.name.value()));
                parts.push(f.space());
                parts.push(body);

                Document::Array(parts)
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for Use<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let items = match &self.items {
            UseItems::Sequence(sequence) => f.inline_token_sequence(&sequence.items),
            UseItems::List(list) => {
                let group = f.delimited(
                    "{",
                    list.items.as_slice(),
                    "}",
                    list.right_brace.start.offset,
                    false,
                );
                f.concat([f.text(list.namespace.value()), f.text("\\"), group])
            }
        };

        f.concat([f.text("use"), f.space(), items, f.text(";")])
    }
}

impl<'arena, A> Format<'arena, A> for UseItem<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            match &self.alias {
                Some(alias) => f.concat([
                    f.text(self.name.value()),
                    f.text(" as "),
                    f.text(alias.identifier.value),
                ]),
                None => f.text(self.name.value()),
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for Constant<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let value = self.value.format(f);

        f.concat([
            attributes,
            f.text("const"),
            f.space(),
            f.text(self.name.value),
            f.text(" = "),
            value,
            f.text(";"),
        ])
    }
}

impl<'arena, A> Format<'arena, A> for TypeAlias<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let type_parameters = match &self.type_parameters {
            Some(list) => f.format_type_parameter_list(list),
            None => f.empty(),
        };
        let aliased = self.aliased.format(f);

        f.concat([
            attributes,
            f.text("type"),
            f.space(),
            f.text(self.name.value),
            type_parameters,
            f.text(" = "),
            aliased,
            f.text(";"),
        ])
    }
}

impl<'arena, A> Format<'arena, A> for Newtype<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let type_parameters = match &self.type_parameters {
            Some(list) => f.format_type_parameter_list(list),
            None => f.empty(),
        };
        let backing = self.backing.format(f);

        f.concat([
            attributes,
            f.text("newtype"),
            f.space(),
            f.text(self.name.value),
            type_parameters,
            f.text(" = "),
            backing,
            f.text(";"),
        ])
    }
}

impl<'arena, A> Format<'arena, A> for AttributeList<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.inline_token_sequence(&self.attributes);
        f.concat([f.text("#["), attributes, f.text("]")])
    }
}

impl<'arena, A> Format<'arena, A> for Attribute<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match &self.argument_list {
            Some(argument_list) => {
                let arguments = f.format_argument_list(argument_list);
                f.concat([f.text(self.name.value()), arguments])
            }
            None => f.text(self.name.value()),
        }
    }
}

impl<'arena, A> Format<'arena, A> for Identifier<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.text(self.value())
    }
}

impl<'arena, A> Format<'arena, A> for Type<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            Type::Named(named) => named.format(f),
            Type::Literal(literal) => literal.format(f),
            Type::NegativeLiteral(literal) => match literal {
                NegativeLiteralType::Integer { literal, .. } => {
                    f.concat([f.text("-"), f.text(literal.raw)])
                }
                NegativeLiteralType::Float { literal, .. } => {
                    f.concat([f.text("-"), f.text(literal.raw)])
                }
            },
            Type::IntegerRange(range) => {
                let lower = match &range.lower {
                    Some(bound) => format_integer_range_bound(f, bound),
                    None => f.empty(),
                };
                let operator = match range.operator {
                    IntegerRangeOperator::Exclusive(_) => f.text(".."),
                    IntegerRangeOperator::Inclusive(_) => f.text("..="),
                };
                let upper = match &range.upper {
                    Some(bound) => format_integer_range_bound(f, bound),
                    None => f.empty(),
                };
                f.concat([lower, operator, upper])
            }
            Type::Union(union) => {
                let mut members = Vec::new();
                flatten_union(union.left, &mut members);
                members.push(union.right);
                f.format_composite_type(&members, "|", "| ")
            }
            Type::Intersection(intersection) => {
                let mut members = Vec::new();
                flatten_intersection(intersection.left, &mut members);
                members.push(intersection.right);
                f.format_composite_type(&members, "&", "& ")
            }
            Type::Negated(negated) => {
                let inner = negated.r#type.format(f);
                f.concat([f.text("!"), inner])
            }
            Type::Parenthesized(parenthesized) => {
                let inner = parenthesized.r#type.format(f);
                f.concat([f.text("("), inner, f.text(")")])
            }
            Type::Function(function) => match &function.signature {
                Some(signature) => {
                    let parameters = f.delimited(
                        "(",
                        signature.parameters.as_slice(),
                        ")",
                        signature.right_parenthesis.start.offset,
                        false,
                    );
                    let return_type = signature.return_type.format(f);
                    f.concat([f.text("fn"), parameters, f.text(": "), return_type])
                }
                None => f.text("fn"),
            },
            Type::Array(array) => {
                let arguments = if let Some(arguments) = &array.type_arguments {
                    f.format_type_argument_list(arguments)
                } else {
                    f.empty()
                };
                f.concat([f.text("array"), arguments])
            }
            Type::Vec(vec) => {
                let arguments = if let Some(arguments) = &vec.type_arguments {
                    f.format_type_argument_list(arguments)
                } else {
                    f.empty()
                };
                f.concat([f.text("vec"), arguments])
            }
            Type::VecShape(shape) => {
                let mut elements = f.inline_token_sequence(&shape.elements);
                if let Some(trailing) = &shape.trailing_type {
                    let rest = match trailing.r#type {
                        Some(r#type) => {
                            let r#type = r#type.format(f);
                            f.concat([f.text("..."), r#type])
                        }
                        None => f.text("..."),
                    };
                    elements = if shape.elements.is_empty() {
                        rest
                    } else {
                        f.concat([elements, f.text(", "), rest])
                    };
                }
                f.concat([f.text("vec["), elements, f.text("]")])
            }
            Type::Dict(dict) => {
                let arguments = if let Some(arguments) = &dict.type_arguments {
                    f.format_type_argument_list(arguments)
                } else {
                    f.empty()
                };
                f.concat([f.text("dict"), arguments])
            }
            Type::DictShape(shape) => {
                let mut entries = f.inline_token_sequence(&shape.entries);
                if let Some(rest) = &shape.rest {
                    let key = rest.type_arguments.key.format(f);
                    let value = rest.type_arguments.value.format(f);
                    let rest = f.concat([f.text("...<"), key, f.text(", "), value, f.text(">")]);
                    entries = if shape.entries.is_empty() {
                        rest
                    } else {
                        f.concat([entries, f.text(", "), rest])
                    };
                }
                f.concat([f.text("dict["), entries, f.text("]")])
            }
            Type::Classname(classname) => {
                let inner = classname.inner.format(f);
                f.concat([f.text("classname<"), inner, f.text(">")])
            }
            Type::Tuple(tuple) => f.format_tuple_type(tuple),
            Type::String(keyword)
            | Type::Int(keyword)
            | Type::Float(keyword)
            | Type::Bool(keyword)
            | Type::Void(keyword)
            | Type::Mixed(keyword)
            | Type::Never(keyword)
            | Type::Object(keyword)
            | Type::Parent(keyword)
            | Type::Static(keyword) => f.text(keyword.value),
            Type::Self_(self_type) => {
                let keyword = f.text(self_type.self_.value);
                match &self_type.member {
                    Some(member) => {
                        let name = f.text(member.name.value);
                        let name = match &member.type_arguments {
                            Some(arguments) => {
                                let arguments = f.format_type_argument_list(arguments);
                                f.concat([name, arguments])
                            }
                            None => name,
                        };
                        f.concat([keyword, f.text("::"), name])
                    }
                    None => keyword,
                }
            }
        }
    }
}

impl<'arena, A> Format<'arena, A> for DictShapeTypeEntry<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let key = self.key.format(f);
        let value = self.value.format(f);
        f.concat([key, f.text(" => "), value])
    }
}

fn format_integer_range_bound<'arena, A>(
    f: &FormatterState<'arena, A>,
    bound: &IntegerRangeBound<'arena>,
) -> Document<'arena, A>
where
    A: Arena,
{
    match bound {
        IntegerRangeBound::Positive(literal) => f.text(literal.raw),
        IntegerRangeBound::Negative { literal, .. } => f.concat([f.text("-"), f.text(literal.raw)]),
    }
}

impl<'arena, A> Format<'arena, A> for NamedType<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let identifier = f.text(self.identifier.value());
        let result = match &self.type_arguments {
            Some(type_arguments) => {
                let arguments = f.format_type_argument_list(type_arguments);
                f.concat([identifier, arguments])
            }
            None => identifier,
        };
        match &self.member {
            Some(member) => {
                let name = f.text(member.name.value);
                let name = match &member.type_arguments {
                    Some(arguments) => {
                        let arguments = f.format_type_argument_list(arguments);
                        f.concat([name, arguments])
                    }
                    None => name,
                };
                f.concat([result, f.text("::"), name])
            }
            None => result,
        }
    }
}

impl<'arena, A> Format<'arena, A> for TypeArgument<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, { self.r#type.format(f) })
    }
}

impl<'arena, A> Format<'arena, A> for TypeParameter<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            let mut parts = f.vec();
            match &self.variance {
                Some(TypeVariance::In(_)) => parts.push(f.text("in ")),
                Some(TypeVariance::Out(_)) => parts.push(f.text("out ")),
                None => {}
            }
            parts.push(f.text(self.name.value));
            if let Some(bound) = &self.bound {
                parts.push(f.text(": "));
                for (index, r#type) in bound.types.iter().enumerate() {
                    if index > 0 {
                        parts.push(f.text(" + "));
                    }
                    let rendered = r#type.format(f);
                    parts.push(rendered);
                }
            }
            if let Some(default) = &self.default {
                let r#type = default.r#type.format(f);
                parts.push(f.text(" = "));
                parts.push(r#type);
            }

            Document::Array(parts)
        })
    }
}

impl<'arena, A> Format<'arena, A> for FunctionTypeParameter<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            let r#type = self.r#type.format(f);
            if self.equals.is_some() {
                f.concat([f.text("="), r#type])
            } else {
                r#type
            }
        })
    }
}

impl<'arena, A> Format<'arena, A> for Class<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let modifiers = f.modifiers_prefix(self.modifiers);

        let mut header = f.vec();
        header.push(modifiers);
        header.push(f.text("class"));
        header.push(f.space());
        header.push(f.text(self.name.value));

        if let Some(type_parameters) = &self.type_parameters {
            let list = f.format_type_parameter_list(type_parameters);
            header.push(list);
        }

        if let Some(extends) = &self.extends {
            let types = f.inline_token_sequence(&extends.types);
            let clause = f.concat([f.text(" extends "), types]);
            let clause = f.never_break(clause);
            header.push(clause);
        }

        if let Some(implements) = &self.implements {
            let clause = f.format_class_like_clause(" implements", implements.types.as_slice());
            header.push(clause);
        }

        if let Some(permissions) = &self.permissions {
            let clause = f.format_class_like_clause(" for", permissions.types.as_slice());
            header.push(clause);
        }

        let body = f.format_braced_sequence(self.members, self.right_brace.start.offset);
        f.finish_class_like(attributes, header, body)
    }
}

impl<'arena, A> Format<'arena, A> for Interface<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);

        let mut header = f.vec();
        header.push(f.text("interface"));
        header.push(f.space());
        header.push(f.text(self.name.value));

        if let Some(type_parameters) = &self.type_parameters {
            let list = f.format_type_parameter_list(type_parameters);
            header.push(list);
        }

        if let Some(extends) = &self.extends {
            let clause = f.format_class_like_clause(" extends", extends.types.as_slice());
            header.push(clause);
        }

        if let Some(permissions) = &self.permissions {
            let clause = f.format_class_like_clause(" for", permissions.types.as_slice());
            header.push(clause);
        }

        let body = f.format_braced_sequence(self.members, self.right_brace.start.offset);
        f.finish_class_like(attributes, header, body)
    }
}

impl<'arena, A> Format<'arena, A> for Enum<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);

        let mut header = f.vec();
        header.push(f.text("enum"));
        header.push(f.space());
        header.push(f.text(self.name.value));

        if let Some(type_parameters) = &self.type_parameters {
            let list = f.format_type_parameter_list(type_parameters);
            header.push(list);
        }

        if let Some(backing) = &self.backing_type {
            let r#type = backing.r#type.format(f);
            header.push(f.text(": "));
            header.push(r#type);
        }

        if let Some(implements) = &self.implements {
            let clause = f.format_class_like_clause(" implements", implements.types.as_slice());
            header.push(clause);
        }

        let body = f.format_braced_sequence(self.members, self.right_brace.start.offset);
        f.finish_class_like(attributes, header, body)
    }
}

impl<'arena, A> Format<'arena, A> for ClassLikeMember<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        match self {
            ClassLikeMember::Constant(member) => member.format(f),
            ClassLikeMember::EnumCase(member) => member.format(f),
            ClassLikeMember::Method(member) => member.format(f),
            ClassLikeMember::Property(member) => member.format(f),
        }
    }

    #[inline]
    fn blank_line_before(&self, next: &Self) -> bool {
        matches!(self, Self::Method(_)) && matches!(next, Self::Method(_))
    }
}

impl<'arena, A> Format<'arena, A> for ClassLikeConstant<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let modifiers = f.modifiers_prefix(self.modifiers);
        let value = self.value.format(f);

        let mut parts = f.vec();
        parts.push(attributes);
        parts.push(modifiers);
        parts.push(f.text("const"));
        parts.push(f.space());
        if let Some(r#type) = self.r#type {
            let r#type = r#type.format(f);
            parts.push(r#type);
            parts.push(f.space());
        }
        parts.push(f.text(self.name.value));
        parts.push(f.text(" = "));
        parts.push(value);
        parts.push(f.text(";"));

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for EnumCase<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);

        let mut parts = f.vec();
        parts.push(attributes);
        parts.push(f.text("case"));
        parts.push(f.space());
        parts.push(f.text(self.name.value));
        if let Some(value) = &self.value {
            let expression = value.expression.format(f);
            parts.push(f.text(" = "));
            parts.push(expression);
        }
        parts.push(f.text(";"));

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Method<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let modifiers = f.modifiers_prefix(self.modifiers);
        let type_parameters = self
            .type_parameters
            .as_ref()
            .map(|list| f.format_type_parameter_list(list));
        let parameters = self.parameter_list.format(f);
        let return_type =
            f.format_return_type_suffix(self.return_type.as_ref().map(|r#type| r#type.r#type));

        let mut signature = f.vec();
        signature.push(modifiers);
        signature.push(f.text("function"));
        signature.push(f.space());
        signature.push(f.text(self.name.value));
        if let Some(type_parameters) = type_parameters {
            signature.push(type_parameters);
        }
        signature.push(parameters);
        signature.push(return_type);

        let declaration = match &self.body {
            MethodBody::Abstract(_) => {
                signature.push(f.text(";"));
                Document::Group(Group::new(signature))
            }
            MethodBody::Concrete(block) => {
                let body = block.format(f);
                f.concat([Document::Group(Group::new(signature)), f.space(), body])
            }
        };

        f.concat([attributes, declaration])
    }
}

impl<'arena, A> Format<'arena, A> for Property<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let modifiers = f.modifiers_prefix(self.modifiers);

        let mut parts = f.vec();
        parts.push(attributes);
        parts.push(modifiers);
        if let Some(r#type) = self.r#type {
            let r#type = r#type.format(f);
            parts.push(r#type);
            parts.push(f.space());
        }
        parts.push(f.text(self.variable.name));
        if let Some(default) = &self.default {
            let value = default.value.format(f);
            parts.push(f.text(" = "));
            parts.push(value);
        }
        parts.push(f.text(";"));

        Document::Array(parts)
    }
}

impl<'arena, A> Format<'arena, A> for Function<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        let attributes = f.attribute_lists_prefix(self.attribute_lists);
        let type_parameters = match &self.type_parameters {
            Some(list) => f.format_type_parameter_list(list),
            None => f.empty(),
        };
        let parameters = self.parameter_list.format(f);
        let return_type =
            f.format_return_type_suffix(self.return_type.as_ref().map(|r#type| r#type.r#type));
        let body = self.body.format(f);

        let mut grouped = f.vec();
        grouped.push(f.text("function"));
        grouped.push(f.space());
        grouped.push(f.text(self.name.value));
        grouped.push(type_parameters);
        grouped.push(parameters);
        grouped.push(return_type);

        f.concat([
            attributes,
            Document::Group(Group::new(grouped)),
            f.space(),
            body,
        ])
    }
}

impl<'arena, A> Format<'arena, A> for ParameterList<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        f.signature_parameters(
            self.parameters.as_slice(),
            self.right_parenthesis.start.offset,
        )
    }
}

impl<'arena, A> Format<'arena, A> for Parameter<'arena>
where
    A: Arena,
{
    fn format(&self, f: &mut FormatterState<'arena, A>) -> Document<'arena, A> {
        wrap!(f, self, {
            let modifiers = f.modifiers_prefix(self.modifiers);

            let mut parts = f.vec();
            for attribute_list in self.attribute_lists {
                parts.push(attribute_list.format(f));
                parts.push(f.line());
            }
            parts.push(modifiers);
            if let Some(r#type) = self.r#type {
                let r#type = r#type.format(f);
                parts.push(r#type);
                parts.push(f.space());
            }
            parts.push(f.text(self.variable.name));
            if let Some(default) = &self.default {
                let value = default.value.format(f);
                parts.push(f.text(" = "));
                parts.push(value);
            }

            Document::Group(Group::new(parts))
        })
    }
}
