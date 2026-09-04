//! Each node's direct children, in source order.

use crate::cst::access::Access;
use crate::cst::access::ClassReference;
use crate::cst::array::DictEntry;
use crate::cst::array::TupleElement;
use crate::cst::atom::Identifier;
use crate::cst::atom::Literal;
use crate::cst::binding::BindingTarget;
use crate::cst::binding::ElementBindingTarget;
use crate::cst::call::Argument;
use crate::cst::call::Call;
use crate::cst::call::Callee;
use crate::cst::call::PartialApplication;
use crate::cst::call::PartialArgument;
use crate::cst::class::ClassLikeMember;
use crate::cst::class::MethodBody;
use crate::cst::construct::Construct;
use crate::cst::control_flow::ElseBody;
use crate::cst::control_flow::ForeachTarget;
use crate::cst::declaration::NamespaceBody;
use crate::cst::declaration::UseItems;
use crate::cst::expression::Expression;
use crate::cst::expression::InterpolatedStringPart;
use crate::cst::function::ShortClosureBody;
use crate::cst::node::Node;
use crate::cst::operation::AssignmentTarget;
use crate::cst::operation::DestructureTarget;
use crate::cst::pattern::DictPatternKey;
use crate::cst::pattern::Pattern;
use crate::cst::statement::Statement;
use crate::cst::r#type::IntegerRangeBound;
use crate::cst::r#type::NegativeLiteralType;
use crate::cst::r#type::StringLength;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeVariance;

impl Node<'_, '_> {
    /// Calls `f` once per direct child, in source order.
    pub fn visit_children<F>(&self, f: &mut F)
    where
        F: FnMut(Self),
    {
        match self {
            Node::Program(node) => {
                for statement in node.statements {
                    f(Node::Statement(statement));
                }
            }
            Node::Statement(node) => match node {
                Statement::Namespace(inner) => f(Node::Namespace(inner)),
                Statement::Use(inner) => f(Node::Use(inner)),
                Statement::Class(inner) => f(Node::Class(inner)),
                Statement::Interface(inner) => f(Node::Interface(inner)),
                Statement::Enum(inner) => f(Node::Enum(inner)),
                Statement::Function(inner) => f(Node::Function(inner)),
                Statement::Constant(inner) => f(Node::Constant(inner)),
                Statement::TypeAlias(inner) => f(Node::TypeAlias(inner)),
                Statement::Newtype(inner) => f(Node::Newtype(inner)),
                Statement::Block(inner) => f(Node::Block(inner)),
                Statement::If(inner) => f(Node::If(inner)),
                Statement::While(inner) => f(Node::While(inner)),
                Statement::DoWhile(inner) => f(Node::DoWhile(inner)),
                Statement::For(inner) => f(Node::For(inner)),
                Statement::Foreach(inner) => f(Node::Foreach(inner)),
                Statement::Try(inner) => f(Node::Try(inner)),
                Statement::Using(inner) => {
                    f(Node::Keyword(&inner.using));
                    for binding in &inner.bindings {
                        f(Node::BindingTarget(&binding.target));
                        f(Node::Expression(binding.value));
                    }
                    f(Node::Block(&inner.body));
                }
                Statement::FinalLocal(inner) => f(Node::FinalLocal(inner)),
                Statement::Expression(inner) => f(Node::ExpressionStatement(inner)),
                Statement::Noop(_) => {}
            },
            Node::ExpressionStatement(node) => f(Node::Expression(node.expression)),
            Node::FinalLocal(node) => {
                f(Node::Keyword(&node.r#final));
                f(Node::Variable(&node.variable));
                f(Node::Expression(node.value));
            }
            Node::Block(node) => {
                for statement in node.statements {
                    f(Node::Statement(statement));
                }
            }
            Node::Return(node) => {
                f(Node::Keyword(&node.r#return));
                if let Some(value) = node.value {
                    f(Node::Expression(value));
                }
            }
            Node::Namespace(node) => {
                f(Node::Keyword(&node.namespace));
                f(Node::Identifier(&node.name));
                f(Node::NamespaceBody(&node.body));
            }
            Node::NamespaceBody(node) => match node {
                NamespaceBody::Implicit(inner) => f(Node::NamespaceImplicitBody(inner)),
                NamespaceBody::BraceDelimited(inner) => f(Node::Block(inner)),
            },
            Node::NamespaceImplicitBody(node) => {
                for statement in node.statements {
                    f(Node::Statement(statement));
                }
            }
            Node::Use(node) => {
                f(Node::Keyword(&node.r#use));
                f(Node::UseItems(&node.items));
            }
            Node::UseItems(node) => match node {
                UseItems::Sequence(inner) => f(Node::UseItemSequence(inner)),
                UseItems::List(inner) => f(Node::UseItemList(inner)),
            },
            Node::UseItemSequence(node) => {
                for item in &node.items {
                    f(Node::UseItem(item));
                }
            }
            Node::UseItemList(node) => {
                f(Node::Identifier(&node.namespace));
                for item in &node.items {
                    f(Node::UseItem(item));
                }
            }
            Node::UseItem(node) => {
                f(Node::Identifier(&node.name));
                if let Some(alias) = &node.alias {
                    f(Node::UseItemAlias(alias));
                }
            }
            Node::UseItemAlias(node) => {
                f(Node::Keyword(&node.r#as));
                f(Node::LocalIdentifier(&node.identifier));
            }
            Node::Constant(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.r#const));
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::TypeAlias(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.r#type));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::Type(node.aliased));
            }
            Node::Newtype(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.newtype));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::Type(node.backing));
            }
            Node::AttributeList(node) => {
                for attribute in &node.attributes {
                    f(Node::Attribute(attribute));
                }
            }
            Node::Attribute(node) => {
                f(Node::Identifier(&node.name));
                if let Some(argument_list) = &node.argument_list {
                    f(Node::ArgumentList(argument_list));
                }
            }
            Node::Class(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                for modifier in node.modifiers {
                    f(Node::Modifier(modifier));
                }
                f(Node::Keyword(&node.class));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                if let Some(extends) = &node.extends {
                    f(Node::Extends(extends));
                }
                if let Some(implements) = &node.implements {
                    f(Node::Implements(implements));
                }
                if let Some(permissions) = &node.permissions {
                    f(Node::SealedPermissions(permissions));
                }
                for member in node.members {
                    f(Node::ClassLikeMember(member));
                }
            }
            Node::Interface(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.interface));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                if let Some(extends) = &node.extends {
                    f(Node::Extends(extends));
                }
                if let Some(permissions) = &node.permissions {
                    f(Node::SealedPermissions(permissions));
                }
                for member in node.members {
                    f(Node::ClassLikeMember(member));
                }
            }
            Node::Enum(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.r#enum));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                if let Some(backing) = &node.backing_type {
                    f(Node::EnumBackingType(backing));
                }
                if let Some(implements) = &node.implements {
                    f(Node::Implements(implements));
                }
                for member in node.members {
                    f(Node::ClassLikeMember(member));
                }
            }
            Node::EnumBackingType(node) => f(Node::Type(node.r#type)),
            Node::Extends(node) => {
                f(Node::Keyword(&node.extends));
                for name in node.types {
                    f(Node::NamedType(name));
                }
            }
            Node::SealedPermissions(node) => {
                f(Node::Keyword(&node.r#for));
                for name in node.types {
                    f(Node::Identifier(name));
                }
            }
            Node::Implements(node) => {
                f(Node::Keyword(&node.implements));
                for name in node.types {
                    f(Node::NamedType(name));
                }
            }
            Node::ClassLikeMember(node) => match node {
                ClassLikeMember::Constant(inner) => f(Node::ClassLikeConstant(inner)),
                ClassLikeMember::EnumCase(inner) => f(Node::EnumCase(inner)),
                ClassLikeMember::Method(inner) => f(Node::Method(inner)),
                ClassLikeMember::Property(inner) => f(Node::Property(inner)),
            },
            Node::ClassLikeConstant(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                for modifier in node.modifiers {
                    f(Node::Modifier(modifier));
                }
                f(Node::Keyword(&node.r#const));
                if let Some(r#type) = node.r#type {
                    f(Node::Type(r#type));
                }
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::EnumCase(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.case));
                f(Node::LocalIdentifier(&node.name));
                if let Some(value) = &node.value {
                    f(Node::EnumCaseValue(value));
                }
            }
            Node::EnumCaseValue(node) => f(Node::Expression(node.expression)),
            Node::Method(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                for modifier in node.modifiers {
                    f(Node::Modifier(modifier));
                }
                f(Node::Keyword(&node.function));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::ParameterList(&node.parameter_list));
                if let Some(return_type) = &node.return_type {
                    f(Node::ReturnType(return_type));
                }
                f(Node::MethodBody(&node.body));
            }
            Node::MethodBody(node) => {
                if let MethodBody::Concrete(block) = node {
                    f(Node::Block(block));
                }
            }
            Node::Property(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                for modifier in node.modifiers {
                    f(Node::Modifier(modifier));
                }
                if let Some(r#type) = node.r#type {
                    f(Node::Type(r#type));
                }
                f(Node::Variable(&node.variable));
                if let Some(default) = &node.default {
                    f(Node::PropertyDefault(default));
                }
            }
            Node::PropertyDefault(node) => f(Node::Expression(node.value)),
            Node::Function(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.function));
                f(Node::LocalIdentifier(&node.name));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::ParameterList(&node.parameter_list));
                if let Some(return_type) = &node.return_type {
                    f(Node::ReturnType(return_type));
                }
                f(Node::Block(&node.body));
            }
            Node::Closure(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.function));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::ParameterList(&node.parameter_list));
                if let Some(use_clause) = &node.use_clause {
                    f(Node::ClosureUseClause(use_clause));
                }
                if let Some(return_type) = &node.return_type {
                    f(Node::ReturnType(return_type));
                }
                f(Node::Block(&node.body));
            }
            Node::ClosureUseClause(node) => {
                f(Node::Keyword(&node.r#use));
                for variable in node.variables {
                    f(Node::Variable(variable));
                }
            }
            Node::ShortClosure(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                f(Node::Keyword(&node.r#fn));
                if let Some(type_parameters) = &node.type_parameters {
                    f(Node::TypeParameterList(type_parameters));
                }
                f(Node::ParameterList(&node.parameter_list));
                if let Some(return_type) = &node.return_type {
                    f(Node::ReturnType(return_type));
                }
                f(Node::ShortClosureBody(&node.body));
            }
            Node::ShortClosureBody(node) => match node {
                ShortClosureBody::Expression { expression, .. } => {
                    f(Node::Expression(expression));
                }
                ShortClosureBody::Block(block) => f(Node::Block(block)),
            },
            Node::ParameterList(node) => {
                for parameter in &node.parameters {
                    f(Node::Parameter(parameter));
                }
            }
            Node::Parameter(node) => {
                for list in node.attribute_lists {
                    f(Node::AttributeList(list));
                }
                for modifier in node.modifiers {
                    f(Node::Modifier(modifier));
                }
                if let Some(r#type) = node.r#type {
                    f(Node::Type(r#type));
                }
                f(Node::Variable(&node.variable));
                if let Some(default) = &node.default {
                    f(Node::ParameterDefault(default));
                }
            }
            Node::ParameterDefault(node) => f(Node::Expression(node.value)),
            Node::ReturnType(node) => f(Node::Type(node.r#type)),
            Node::If(node) => {
                f(Node::Keyword(&node.r#if));
                f(Node::Expression(node.condition));
                f(Node::Block(&node.body));
                if let Some(r#else) = &node.r#else {
                    f(Node::Else(r#else));
                }
            }
            Node::Else(node) => {
                f(Node::Keyword(&node.r#else));
                f(Node::ElseBody(&node.body));
            }
            Node::ElseBody(node) => match node {
                ElseBody::If(inner) => f(Node::If(inner)),
                ElseBody::Block(inner) => f(Node::Block(inner)),
            },
            Node::While(node) => {
                f(Node::Keyword(&node.r#while));
                f(Node::Expression(node.condition));
                f(Node::Block(&node.body));
            }
            Node::DoWhile(node) => {
                f(Node::Keyword(&node.r#do));
                f(Node::Block(&node.body));
                f(Node::Keyword(&node.r#while));
                f(Node::Expression(node.condition));
            }
            Node::For(node) => {
                f(Node::Keyword(&node.r#for));
                for expression in &node.initializations {
                    f(Node::Expression(expression));
                }
                for expression in &node.conditions {
                    f(Node::Expression(expression));
                }
                for expression in &node.increments {
                    f(Node::Expression(expression));
                }
                f(Node::Block(&node.body));
            }
            Node::Foreach(node) => {
                f(Node::Keyword(&node.foreach));
                f(Node::Expression(node.expression));
                f(Node::Keyword(&node.r#as));
                f(Node::ForeachTarget(&node.target));
                f(Node::Block(&node.body));
            }
            Node::ForeachTarget(node) => match node {
                ForeachTarget::Value(inner) => f(Node::ForeachValueTarget(inner)),
                ForeachTarget::KeyValue(inner) => f(Node::ForeachKeyValueTarget(inner)),
            },
            Node::ForeachValueTarget(node) => f(Node::AssignmentTarget(node.value)),
            Node::ForeachKeyValueTarget(node) => {
                f(Node::AssignmentTarget(node.key));
                f(Node::AssignmentTarget(node.value));
            }
            Node::Break(node) => {
                f(Node::Keyword(&node.r#break));
                if let Some(level) = &node.level {
                    f(Node::LiteralInteger(level));
                }
            }
            Node::Continue(node) => {
                f(Node::Keyword(&node.r#continue));
                if let Some(level) = &node.level {
                    f(Node::LiteralInteger(level));
                }
            }
            Node::Try(node) => {
                f(Node::Keyword(&node.r#try));
                f(Node::Block(&node.block));
                for clause in node.catch_clauses {
                    f(Node::TryCatchClause(clause));
                }
                if let Some(clause) = &node.else_clause {
                    f(Node::TryElseClause(clause));
                }
                if let Some(clause) = &node.finally_clause {
                    f(Node::TryFinallyClause(clause));
                }
            }
            Node::TryCatchClause(node) => {
                f(Node::Keyword(&node.r#catch));
                f(Node::Type(node.r#type));
                if let Some(variable) = &node.variable {
                    f(Node::Variable(variable));
                }
                if let Some(guard) = &node.guard {
                    f(Node::Keyword(&guard.r#if));
                    f(Node::Expression(guard.condition));
                }
                f(Node::Block(&node.block));
            }
            Node::TryElseClause(node) => {
                f(Node::Keyword(&node.r#else));
                f(Node::Block(&node.block));
            }
            Node::TryFinallyClause(node) => {
                f(Node::Keyword(&node.r#finally));
                f(Node::Block(&node.block));
            }
            Node::Match(node) => {
                f(Node::Keyword(&node.r#match));
                f(Node::Expression(node.expression));
                for arm in &node.arms {
                    f(Node::MatchArm(arm));
                }
            }
            Node::MatchArm(node) => {
                f(Node::Pattern(node.pattern));
                f(Node::Expression(node.expression));
            }
            Node::Pattern(node) => match node {
                Pattern::Variable(variable) => f(Node::Variable(variable)),
                Pattern::Parenthesized(pattern) => {
                    f(Node::Pattern(pattern.pattern));
                }
                Pattern::As(pattern) => {
                    f(Node::Pattern(pattern.left));
                    f(Node::Pattern(pattern.right));
                }
                Pattern::Union(pattern) => {
                    f(Node::Pattern(pattern.left));
                    f(Node::Pattern(pattern.right));
                }
                Pattern::Vec(pattern) => {
                    for element in &pattern.elements {
                        f(Node::Pattern(element));
                    }
                    if let Some(trailing) = &pattern.trailing
                        && let Some(pattern) = trailing.pattern
                    {
                        f(Node::Pattern(pattern));
                    }
                }
                Pattern::Dict(pattern) => {
                    for entry in &pattern.entries {
                        f(Node::DictPatternKey(&entry.key));
                        f(Node::Pattern(entry.pattern));
                    }
                    if let Some(trailing) = &pattern.trailing
                        && let Some(pattern) = trailing.pattern
                    {
                        f(Node::Pattern(pattern));
                    }
                }
                Pattern::Tuple(pattern) => {
                    for element in &pattern.elements {
                        f(Node::Pattern(element));
                    }
                    if let Some(trailing) = &pattern.trailing
                        && let Some(pattern) = trailing.pattern
                    {
                        f(Node::Pattern(pattern));
                    }
                }
                Pattern::Type(r#type) => f(Node::Type(r#type)),
            },
            Node::DictPatternKey(node) => match node {
                DictPatternKey::String(literal) => f(Node::LiteralString(literal)),
                DictPatternKey::Integer { literal, .. } => f(Node::LiteralInteger(literal)),
            },
            Node::BindingTarget(node) => match node {
                BindingTarget::Variable(variable) => {
                    f(Node::Variable(variable));
                }
                BindingTarget::Tuple(tuple) => {
                    for target in &tuple.targets {
                        match target {
                            ElementBindingTarget::Target(target) => {
                                f(Node::BindingTarget(target));
                            }
                            ElementBindingTarget::Rest(rest) => {
                                if let Some(target) = &rest.target {
                                    f(Node::BindingTarget(target));
                                }
                            }
                        }
                    }
                }
                BindingTarget::Dict(dict) => {
                    for entry in &dict.entries {
                        f(Node::Expression(entry.key));
                        f(Node::BindingTarget(&entry.target));
                    }
                }
            },
            Node::Expression(node) => match node {
                Expression::Binary(inner) => f(Node::Binary(inner)),
                Expression::UnaryPrefix(inner) => f(Node::UnaryPrefix(inner)),
                Expression::UnaryPostfix(inner) => f(Node::UnaryPostfix(inner)),
                Expression::TypeOperation(inner) => f(Node::TypeOperation(inner)),
                Expression::Assignment(inner) => f(Node::Assignment(inner)),
                Expression::Parenthesized(inner) => f(Node::Parenthesized(inner)),
                Expression::Literal(inner) => f(Node::Literal(inner)),
                Expression::InterpolatedString(inner) => f(Node::InterpolatedString(inner)),
                Expression::Vec(inner) => f(Node::VecExpression(inner)),
                Expression::VecFill(inner) => f(Node::VecFillExpression(inner)),
                Expression::Dict(inner) => f(Node::DictExpression(inner)),
                Expression::Tuple(inner) => f(Node::TupleExpression(inner)),
                Expression::ArrayAccess(inner) => f(Node::ArrayAccess(inner)),
                Expression::ArrayAppend(inner) => f(Node::ArrayAppend(inner)),
                Expression::Variable(inner) => f(Node::Variable(inner)),
                Expression::Access(inner) => f(Node::Access(inner)),
                Expression::Call(inner) => f(Node::Call(inner)),
                Expression::PartialApplication(inner) => f(Node::PartialApplication(inner)),
                Expression::Closure(inner) => f(Node::Closure(inner)),
                Expression::ShortClosure(inner) => f(Node::ShortClosure(inner)),
                Expression::Match(inner) => f(Node::Match(inner)),
                Expression::Instantiation(inner) => f(Node::Instantiation(inner)),
                Expression::Break(inner) => f(Node::Break(inner)),
                Expression::Continue(inner) => f(Node::Continue(inner)),
                Expression::Return(inner) => f(Node::Return(inner)),
                Expression::Throw(inner) => f(Node::Throw(inner)),
                Expression::Construct(inner) => f(Node::Construct(inner)),
            },
            Node::Parenthesized(node) => f(Node::Expression(node.expression)),
            Node::InterpolatedString(node) => {
                for part in node.parts {
                    f(Node::InterpolatedStringPart(part));
                }
            }
            Node::InterpolatedStringPart(node) => match node {
                InterpolatedStringPart::Literal(inner) => {
                    f(Node::InterpolatedStringLiteral(inner));
                }
                InterpolatedStringPart::Variable(inner) => f(Node::Variable(inner)),
                InterpolatedStringPart::Expression(inner) => {
                    f(Node::InterpolatedStringExpression(inner));
                }
            },
            Node::InterpolatedStringExpression(node) => {
                f(Node::Expression(node.expression));
            }
            Node::Throw(node) => {
                f(Node::Keyword(&node.throw));
                f(Node::Expression(node.exception));
            }
            Node::Construct(node) => match node {
                Construct::Require(inner) => f(Node::RequireConstruct(inner)),
                Construct::RequireOnce(inner) => f(Node::RequireOnceConstruct(inner)),
                Construct::Length(inner) => f(Node::LengthConstruct(inner)),
                Construct::Contains(inner) => f(Node::ContainsConstruct(inner)),
                Construct::ContainsKey(inner) => f(Node::ContainsKeyConstruct(inner)),
                Construct::Clone(inner) => f(Node::CloneConstruct(inner)),
                Construct::Remove(inner) => f(Node::RemoveConstruct(inner)),
                Construct::SwapRemove(inner) => f(Node::SwapRemoveConstruct(inner)),
                Construct::RemoveFirst(inner) => f(Node::RemoveFirstConstruct(inner)),
                Construct::RemoveLast(inner) => f(Node::RemoveLastConstruct(inner)),
                Construct::Assert(inner) => f(Node::AssertConstruct(inner)),
                Construct::Exit(inner) => f(Node::ExitConstruct(inner)),
                Construct::Panic(inner) => f(Node::PanicConstruct(inner)),
                Construct::Write(inner) => f(Node::WriteConstruct(inner)),
                Construct::WriteLine(inner) => f(Node::WriteLineConstruct(inner)),
                Construct::WriteError(inner) => f(Node::WriteErrorConstruct(inner)),
                Construct::WriteErrorLine(inner) => f(Node::WriteErrorLineConstruct(inner)),
                Construct::Debug(inner) => f(Node::DebugConstruct(inner)),
                Construct::Discard(inner) => f(Node::DiscardConstruct(inner)),
                Construct::Drop(inner) => f(Node::DropConstruct(inner)),
                Construct::File(inner) => f(Node::FileConstruct(inner)),
                Construct::Directory(inner) => f(Node::DirectoryConstruct(inner)),
                Construct::Embed(inner) => f(Node::EmbedConstruct(inner)),
            },
            Node::RequireConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::RequireOnceConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::LengthConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::ContainsConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.array));
                f(Node::Expression(node.value));
            }
            Node::ContainsKeyConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.array));
                f(Node::Expression(node.key));
            }
            Node::CloneConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.object));
                for field in node.fields {
                    f(Node::CloneField(field));
                }
            }
            Node::CloneField(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::RemoveConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.array));
                f(Node::Expression(node.key));
            }
            Node::SwapRemoveConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.vector));
                f(Node::Expression(node.index));
            }
            Node::RemoveFirstConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.array));
            }
            Node::RemoveLastConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.array));
            }
            Node::AssertConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.condition));
                if let Some(message) = &node.message {
                    f(Node::AssertMessage(message));
                }
            }
            Node::AssertMessage(node) => f(Node::Expression(node.value)),
            Node::ExitConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                if let Some(code) = node.code {
                    f(Node::Expression(code));
                }
            }
            Node::PanicConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::LiteralString(&node.message));
            }
            Node::WriteConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for argument in node.arguments {
                    f(Node::ConstructArgument(argument));
                }
            }
            Node::WriteLineConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for argument in node.arguments {
                    f(Node::ConstructArgument(argument));
                }
            }
            Node::WriteErrorConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for argument in node.arguments {
                    f(Node::ConstructArgument(argument));
                }
            }
            Node::WriteErrorLineConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for argument in node.arguments {
                    f(Node::ConstructArgument(argument));
                }
            }
            Node::DebugConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for argument in node.arguments {
                    f(Node::ConstructArgument(argument));
                }
            }
            Node::DiscardConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::DropConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                for variable in node.variables {
                    f(Node::Variable(variable));
                }
            }
            Node::FileConstruct(node) => f(Node::LocalIdentifier(&node.name)),
            Node::DirectoryConstruct(node) => f(Node::LocalIdentifier(&node.name)),
            Node::EmbedConstruct(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::LiteralString(&node.path));
            }
            Node::ConstructArgument(node) => f(Node::Expression(node.value)),
            Node::Instantiation(node) => {
                f(Node::Keyword(&node.new));
                f(Node::ClassReference(&node.class));
                if let Some(argument_list) = &node.argument_list {
                    f(Node::ArgumentList(argument_list));
                }
            }
            Node::Binary(node) => {
                f(Node::Expression(node.lhs));
                f(Node::BinaryOperator(&node.operator));
                f(Node::Expression(node.rhs));
            }
            Node::UnaryPrefix(node) => {
                f(Node::UnaryPrefixOperator(&node.operator));
                f(Node::Expression(node.operand));
            }
            Node::UnaryPostfix(node) => {
                f(Node::Expression(node.operand));
                f(Node::UnaryPostfixOperator(&node.operator));
            }
            Node::TypeOperation(node) => {
                f(Node::Expression(node.operand));
                f(Node::TypeOperator(&node.operator));
                f(Node::Type(node.r#type));
            }
            Node::Assignment(node) => {
                f(Node::AssignmentTarget(&node.target));
                f(Node::AssignmentOperator(&node.operator));
                f(Node::Expression(node.value));
            }
            Node::AssignmentTarget(node) => match node {
                AssignmentTarget::Variable(inner) => f(Node::Variable(inner)),
                AssignmentTarget::Property(inner) => f(Node::PropertyAccess(inner)),
                AssignmentTarget::StaticProperty(inner) => f(Node::StaticPropertyAccess(inner)),
                AssignmentTarget::ArrayIndex(inner) => f(Node::ArrayAccess(inner)),
                AssignmentTarget::ArrayAppend(inner) => f(Node::ArrayAppend(inner)),
                AssignmentTarget::Tuple(inner) => f(Node::TupleDestructure(inner)),
                AssignmentTarget::Dict(inner) => f(Node::DictDestructure(inner)),
            },
            Node::TupleDestructure(node) => {
                for target in &node.targets {
                    f(Node::DestructureTarget(target));
                }
            }
            Node::DictDestructure(node) => {
                for entry in &node.entries {
                    f(Node::DictDestructureEntry(entry));
                }
            }
            Node::DictDestructureEntry(node) => {
                f(Node::Expression(node.key));
                f(Node::AssignmentTarget(&node.target));
            }
            Node::DestructureTarget(node) => match node {
                DestructureTarget::Target(inner) => f(Node::AssignmentTarget(inner)),
                DestructureTarget::Default(inner) => f(Node::DestructureDefault(inner)),
                DestructureTarget::Rest(inner) => f(Node::DestructureRest(inner)),
            },
            Node::DestructureDefault(node) => {
                f(Node::AssignmentTarget(&node.target));
                f(Node::Expression(node.value));
            }
            Node::DestructureRest(node) => {
                if let Some(target) = &node.target {
                    f(Node::AssignmentTarget(target));
                }
            }
            Node::Access(node) => match node {
                Access::Constant(inner) => f(Node::ConstantAccess(inner)),
                Access::Property(inner) => f(Node::PropertyAccess(inner)),
                Access::NullSafeProperty(inner) => f(Node::NullSafePropertyAccess(inner)),
                Access::StaticProperty(inner) => f(Node::StaticPropertyAccess(inner)),
                Access::ClassConstant(inner) => f(Node::ClassConstantAccess(inner)),
            },
            Node::ClassReference(node) => match node {
                ClassReference::Named(inner) => f(Node::NamedClassReference(inner)),
                ClassReference::Self_(inner)
                | ClassReference::Parent(inner)
                | ClassReference::Static(inner) => f(Node::Keyword(inner)),
                ClassReference::Expression(inner) => f(Node::Expression(inner)),
            },
            Node::NamedClassReference(node) => {
                f(Node::Identifier(&node.identifier));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
            }
            Node::ConstantAccess(node) => f(Node::Identifier(&node.name)),
            Node::PropertyAccess(node) => {
                f(Node::Expression(node.object));
                f(Node::LocalIdentifier(&node.property));
            }
            Node::NullSafePropertyAccess(node) => {
                f(Node::Expression(node.object));
                f(Node::LocalIdentifier(&node.property));
            }
            Node::StaticPropertyAccess(node) => {
                f(Node::ClassReference(&node.class));
                f(Node::Variable(&node.property));
            }
            Node::ClassConstantAccess(node) => {
                f(Node::ClassReference(&node.class));
                f(Node::LocalIdentifier(&node.constant));
            }
            Node::VecExpression(node) => {
                f(Node::Keyword(&node.vec));
                for element in node.elements {
                    f(Node::VecElement(element));
                }
            }
            Node::VecFillExpression(node) => {
                f(Node::Keyword(&node.vec));
                f(Node::Expression(node.value));
                f(Node::Expression(node.size));
            }
            Node::VecElement(node) => f(Node::Expression(node.value)),
            Node::DictExpression(node) => {
                f(Node::Keyword(&node.dict));
                for entry in node.entries {
                    f(Node::DictEntry(entry));
                }
            }
            Node::DictEntry(node) => match node {
                DictEntry::Pair(inner) => f(Node::DictPair(inner)),
                DictEntry::Spread(inner) => f(Node::DictSpread(inner)),
            },
            Node::DictPair(node) => {
                f(Node::Expression(node.key));
                f(Node::Expression(node.value));
            }
            Node::DictSpread(node) => f(Node::Expression(node.value)),
            Node::TupleExpression(node) => {
                for element in node.elements {
                    f(Node::TupleElement(element));
                }
            }
            Node::TupleElement(node) => match node {
                TupleElement::Value(inner) => f(Node::Expression(inner)),
                TupleElement::Rest(inner) => f(Node::TupleRest(inner)),
            },
            Node::TupleRest(node) => {
                if let Some(value) = node.value {
                    f(Node::Expression(value));
                }
            }
            Node::ArrayAccess(node) => {
                f(Node::Expression(node.array));
                f(Node::Expression(node.index));
            }
            Node::ArrayAppend(node) => f(Node::Expression(node.array)),
            Node::Callee(node) => match node {
                Callee::Identifier(inner) => f(Node::Identifier(inner)),
                Callee::Expression(inner) => f(Node::Expression(inner)),
            },
            Node::Call(node) => match node {
                Call::Function(inner) => f(Node::FunctionCall(inner)),
                Call::Method(inner) => f(Node::MethodCall(inner)),
                Call::NullSafeMethod(inner) => f(Node::NullSafeMethodCall(inner)),
                Call::StaticMethod(inner) => f(Node::StaticMethodCall(inner)),
            },
            Node::FunctionCall(node) => {
                f(Node::Callee(&node.function));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::ArgumentList(&node.argument_list));
            }
            Node::MethodCall(node) => {
                f(Node::Expression(node.object));
                f(Node::LocalIdentifier(&node.method));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::ArgumentList(&node.argument_list));
            }
            Node::NullSafeMethodCall(node) => {
                f(Node::Expression(node.object));
                f(Node::LocalIdentifier(&node.method));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::ArgumentList(&node.argument_list));
            }
            Node::StaticMethodCall(node) => {
                f(Node::ClassReference(&node.class));
                f(Node::LocalIdentifier(&node.method));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::ArgumentList(&node.argument_list));
            }
            Node::ArgumentList(node) => {
                for argument in &node.arguments {
                    f(Node::Argument(argument));
                }
            }
            Node::PartialArgumentList(node) => {
                for argument in &node.arguments {
                    f(Node::PartialArgument(argument));
                }
            }
            Node::Argument(node) => match node {
                Argument::Positional(inner) => f(Node::PositionalArgument(inner)),
                Argument::Named(inner) => f(Node::NamedArgument(inner)),
            },
            Node::PartialArgument(node) => match node {
                PartialArgument::Positional(inner) => f(Node::PositionalArgument(inner)),
                PartialArgument::Named(inner) => f(Node::NamedArgument(inner)),
                PartialArgument::NamedPlaceholder(inner) => {
                    f(Node::NamedPlaceholderArgument(inner));
                }
                PartialArgument::Placeholder(inner) => f(Node::PlaceholderArgument(inner)),
                PartialArgument::VariadicPlaceholder(inner) => {
                    f(Node::VariadicPlaceholderArgument(inner));
                }
            },
            Node::PositionalArgument(node) => f(Node::Expression(node.value)),
            Node::NamedArgument(node) => {
                f(Node::LocalIdentifier(&node.name));
                f(Node::Expression(node.value));
            }
            Node::NamedPlaceholderArgument(node) => f(Node::LocalIdentifier(&node.name)),
            Node::PartialApplication(node) => match node {
                PartialApplication::Function(inner) => f(Node::FunctionPartialApplication(inner)),
                PartialApplication::Method(inner) => f(Node::MethodPartialApplication(inner)),
                PartialApplication::StaticMethod(inner) => {
                    f(Node::StaticMethodPartialApplication(inner));
                }
            },
            Node::FunctionPartialApplication(node) => {
                f(Node::Callee(&node.function));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::PartialArgumentList(&node.argument_list));
            }
            Node::MethodPartialApplication(node) => {
                f(Node::Expression(node.object));
                f(Node::LocalIdentifier(&node.method));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::PartialArgumentList(&node.argument_list));
            }
            Node::StaticMethodPartialApplication(node) => {
                f(Node::ClassReference(&node.class));
                f(Node::LocalIdentifier(&node.method));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                f(Node::PartialArgumentList(&node.argument_list));
            }
            Node::Type(node) => match node {
                Type::Named(inner) => f(Node::NamedType(inner)),
                Type::Literal(inner) => f(Node::Literal(inner)),
                Type::NegativeLiteral(inner) => f(Node::NegativeLiteralType(inner)),
                Type::IntegerRange(inner) => f(Node::IntegerRangeType(inner)),
                Type::StringLength(inner) => f(Node::StringLengthType(inner)),
                Type::Union(inner) => f(Node::UnionType(inner)),
                Type::Intersection(inner) => f(Node::IntersectionType(inner)),
                Type::Negated(inner) => f(Node::NegatedType(inner)),
                Type::Parenthesized(inner) => f(Node::ParenthesizedType(inner)),
                Type::Function(inner) => f(Node::FunctionType(inner)),
                Type::Array(inner) => f(Node::ArrayType(inner)),
                Type::Vec(inner) => f(Node::VecType(inner)),
                Type::VecShape(inner) => {
                    for element in &inner.elements {
                        f(Node::Type(element));
                    }
                    if let Some(trailing) = &inner.trailing_type
                        && let Some(r#type) = trailing.r#type
                    {
                        f(Node::Type(r#type));
                    }
                }
                Type::Dict(inner) => f(Node::DictType(inner)),
                Type::DictShape(inner) => {
                    for entry in &inner.entries {
                        f(Node::Literal(&entry.key));
                        f(Node::Type(entry.value));
                    }
                    if let Some(rest) = &inner.rest {
                        f(Node::Type(rest.type_arguments.key));
                        f(Node::Type(rest.type_arguments.value));
                    }
                }
                Type::Classname(inner) => f(Node::ClassnameType(inner)),
                Type::Tuple(inner) => f(Node::TupleType(inner)),
                Type::String(inner)
                | Type::Int(inner)
                | Type::Float(inner)
                | Type::Bool(inner)
                | Type::Void(inner)
                | Type::Mixed(inner)
                | Type::Never(inner)
                | Type::Object(inner)
                | Type::Parent(inner)
                | Type::Static(inner) => f(Node::Keyword(inner)),
                Type::Self_(inner) => {
                    f(Node::Keyword(&inner.self_));
                    if let Some(member) = &inner.member {
                        f(Node::LocalIdentifier(&member.name));
                        if let Some(type_arguments) = &member.type_arguments {
                            f(Node::TypeArgumentList(type_arguments));
                        }
                    }
                }
            },
            Node::NamedType(node) => {
                f(Node::Identifier(&node.identifier));
                if let Some(type_arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(type_arguments));
                }
                if let Some(member) = &node.member {
                    f(Node::LocalIdentifier(&member.name));
                    if let Some(type_arguments) = &member.type_arguments {
                        f(Node::TypeArgumentList(type_arguments));
                    }
                }
            }
            Node::TypeArgumentList(node) => {
                for argument in node.arguments {
                    f(Node::TypeArgument(argument));
                }
            }
            Node::TypeArgument(node) => f(Node::Type(node.r#type)),
            Node::TypeParameterList(node) => {
                for parameter in node.parameters {
                    f(Node::TypeParameter(parameter));
                }
            }
            Node::TypeParameter(node) => {
                if let Some(variance) = &node.variance {
                    f(Node::TypeVariance(variance));
                }
                f(Node::LocalIdentifier(&node.name));
                if let Some(bound) = &node.bound {
                    f(Node::TypeParameterBound(bound));
                }
                if let Some(default) = &node.default {
                    f(Node::TypeParameterDefault(default));
                }
            }
            Node::TypeVariance(node) => match node {
                TypeVariance::In(inner) | TypeVariance::Out(inner) => f(Node::Keyword(inner)),
            },
            Node::TypeParameterBound(node) => {
                for r#type in node.types {
                    f(Node::Type(r#type));
                }
            }
            Node::TypeParameterDefault(node) => f(Node::Type(node.r#type)),
            Node::UnionType(node) => {
                f(Node::Type(node.left));
                f(Node::Type(node.right));
            }
            Node::IntersectionType(node) => {
                f(Node::Type(node.left));
                f(Node::Type(node.right));
            }
            Node::NegatedType(node) => f(Node::Type(node.r#type)),
            Node::NegativeLiteralType(node) => match node {
                NegativeLiteralType::Integer { literal, .. } => {
                    f(Node::LiteralInteger(literal));
                }
                NegativeLiteralType::Float { literal, .. } => {
                    f(Node::LiteralFloat(literal));
                }
            },
            Node::IntegerRangeType(node) => {
                if let Some(lower) = &node.lower {
                    f(Node::IntegerRangeBound(lower));
                }
                f(Node::IntegerRangeOperator(&node.operator));
                if let Some(upper) = &node.upper {
                    f(Node::IntegerRangeBound(upper));
                }
            }
            Node::StringLengthType(node) => {
                f(Node::Keyword(&node.string));
                f(Node::StringLength(&node.length));
            }
            Node::StringLength(node) => match node {
                StringLength::Exact(length) => f(Node::LiteralInteger(length)),
                StringLength::Range(range) => f(Node::IntegerRangeType(range)),
            },
            Node::IntegerRangeBound(node) => match node {
                IntegerRangeBound::Positive(literal)
                | IntegerRangeBound::Negative { literal, .. } => {
                    f(Node::LiteralInteger(literal));
                }
            },
            Node::ParenthesizedType(node) => f(Node::Type(node.r#type)),
            Node::FunctionType(node) => {
                f(Node::Keyword(&node.r#fn));
                if let Some(signature) = &node.signature {
                    f(Node::FunctionTypeSignature(signature));
                }
            }
            Node::FunctionTypeSignature(node) => {
                for parameter in node.parameters {
                    f(Node::FunctionTypeParameter(parameter));
                }
                f(Node::Type(node.return_type));
            }
            Node::FunctionTypeParameter(node) => f(Node::Type(node.r#type)),
            Node::ArrayType(node) => {
                f(Node::Keyword(&node.array));
                if let Some(arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(arguments));
                }
            }
            Node::VecType(node) => {
                f(Node::Keyword(&node.vec));
                if let Some(arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(arguments));
                }
            }
            Node::DictType(node) => {
                f(Node::Keyword(&node.dict));
                if let Some(arguments) = &node.type_arguments {
                    f(Node::TypeArgumentList(arguments));
                }
            }
            Node::ClassnameType(node) => {
                f(Node::Keyword(&node.classname));
                f(Node::Type(node.inner));
            }
            Node::TupleType(node) => {
                for element in &node.elements {
                    f(Node::Type(element));
                }
                if let Some(trailing) = &node.trailing_type
                    && let Some(r#type) = trailing.r#type
                {
                    f(Node::Type(r#type));
                }
            }
            Node::Identifier(node) => match node {
                Identifier::Local(inner) => f(Node::LocalIdentifier(inner)),
                Identifier::Qualified(inner) => f(Node::QualifiedIdentifier(inner)),
                Identifier::FullyQualified(inner) => f(Node::FullyQualifiedIdentifier(inner)),
            },
            Node::Literal(node) => match node {
                Literal::String(inner) => f(Node::LiteralString(inner)),
                Literal::Integer(inner) => f(Node::LiteralInteger(inner)),
                Literal::Float(inner) => f(Node::LiteralFloat(inner)),
                Literal::True(inner) | Literal::False(inner) | Literal::Null(inner) => {
                    f(Node::Keyword(inner));
                }
            },
            Node::Keyword(_)
            | Node::BinaryOperator(_)
            | Node::UnaryPrefixOperator(_)
            | Node::UnaryPostfixOperator(_)
            | Node::TypeOperator(_)
            | Node::AssignmentOperator(_)
            | Node::PlaceholderArgument(_)
            | Node::VariadicPlaceholderArgument(_)
            | Node::IntegerRangeOperator(_)
            | Node::LocalIdentifier(_)
            | Node::QualifiedIdentifier(_)
            | Node::FullyQualifiedIdentifier(_)
            | Node::Variable(_)
            | Node::LiteralString(_)
            | Node::InterpolatedStringLiteral(_)
            | Node::LiteralInteger(_)
            | Node::LiteralFloat(_)
            | Node::Modifier(_) => {}
        }
    }

    /// Collects this node's direct children into an owned vector.
    #[must_use]
    pub fn children(&self) -> Vec<Self> {
        let mut children = Vec::new();
        self.visit_children(&mut |child| children.push(child));

        children
    }
}
