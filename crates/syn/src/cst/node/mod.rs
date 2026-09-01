//! A uniform, borrowed view over any CST node.

mod children;

use whim_span::HasSpan;
use whim_span::Span;

use crate::cst::Program;
use crate::cst::access::Access;
use crate::cst::access::ClassConstantAccess;
use crate::cst::access::ClassReference;
use crate::cst::access::ConstantAccess;
use crate::cst::access::NamedClassReference;
use crate::cst::access::NullSafePropertyAccess;
use crate::cst::access::PropertyAccess;
use crate::cst::access::StaticPropertyAccess;
use crate::cst::array::ArrayAccess;
use crate::cst::array::ArrayAppend;
use crate::cst::array::DictEntry;
use crate::cst::array::DictExpression;
use crate::cst::array::DictPair;
use crate::cst::array::DictSpread;
use crate::cst::array::TupleElement;
use crate::cst::array::TupleExpression;
use crate::cst::array::TupleRest;
use crate::cst::array::VecElement;
use crate::cst::array::VecExpression;
use crate::cst::array::VecFillExpression;
use crate::cst::atom::FullyQualifiedIdentifier;
use crate::cst::atom::Identifier;
use crate::cst::atom::Keyword;
use crate::cst::atom::Literal;
use crate::cst::atom::LiteralFloat;
use crate::cst::atom::LiteralInteger;
use crate::cst::atom::LiteralString;
use crate::cst::atom::LocalIdentifier;
use crate::cst::atom::Modifier;
use crate::cst::atom::QualifiedIdentifier;
use crate::cst::atom::Variable;
use crate::cst::binding::BindingTarget;
use crate::cst::call::Argument;
use crate::cst::call::ArgumentList;
use crate::cst::call::Call;
use crate::cst::call::Callee;
use crate::cst::call::FunctionCall;
use crate::cst::call::FunctionPartialApplication;
use crate::cst::call::MethodCall;
use crate::cst::call::MethodPartialApplication;
use crate::cst::call::NamedArgument;
use crate::cst::call::NamedPlaceholderArgument;
use crate::cst::call::NullSafeMethodCall;
use crate::cst::call::PartialApplication;
use crate::cst::call::PartialArgument;
use crate::cst::call::PartialArgumentList;
use crate::cst::call::PlaceholderArgument;
use crate::cst::call::PositionalArgument;
use crate::cst::call::StaticMethodCall;
use crate::cst::call::StaticMethodPartialApplication;
use crate::cst::call::VariadicPlaceholderArgument;
use crate::cst::class::Class;
use crate::cst::class::ClassLikeConstant;
use crate::cst::class::ClassLikeMember;
use crate::cst::class::Enum;
use crate::cst::class::EnumBackingType;
use crate::cst::class::EnumCase;
use crate::cst::class::EnumCaseValue;
use crate::cst::class::Extends;
use crate::cst::class::Implements;
use crate::cst::class::Interface;
use crate::cst::class::Method;
use crate::cst::class::MethodBody;
use crate::cst::class::Property;
use crate::cst::class::PropertyDefault;
use crate::cst::class::SealedPermissions;
use crate::cst::construct::AssertConstruct;
use crate::cst::construct::AssertMessage;
use crate::cst::construct::CloneConstruct;
use crate::cst::construct::CloneField;
use crate::cst::construct::Construct;
use crate::cst::construct::ConstructArgument;
use crate::cst::construct::ContainsConstruct;
use crate::cst::construct::ContainsKeyConstruct;
use crate::cst::construct::DebugConstruct;
use crate::cst::construct::DirectoryConstruct;
use crate::cst::construct::DiscardConstruct;
use crate::cst::construct::DropConstruct;
use crate::cst::construct::EmbedConstruct;
use crate::cst::construct::ExitConstruct;
use crate::cst::construct::FileConstruct;
use crate::cst::construct::LengthConstruct;
use crate::cst::construct::PanicConstruct;
use crate::cst::construct::RemoveConstruct;
use crate::cst::construct::RemoveFirstConstruct;
use crate::cst::construct::RemoveLastConstruct;
use crate::cst::construct::RequireConstruct;
use crate::cst::construct::RequireOnceConstruct;
use crate::cst::construct::SwapRemoveConstruct;
use crate::cst::construct::WriteConstruct;
use crate::cst::construct::WriteErrorConstruct;
use crate::cst::construct::WriteErrorLineConstruct;
use crate::cst::construct::WriteLineConstruct;
use crate::cst::control_flow::DoWhile;
use crate::cst::control_flow::Else;
use crate::cst::control_flow::ElseBody;
use crate::cst::control_flow::For;
use crate::cst::control_flow::Foreach;
use crate::cst::control_flow::ForeachKeyValueTarget;
use crate::cst::control_flow::ForeachTarget;
use crate::cst::control_flow::ForeachValueTarget;
use crate::cst::control_flow::If;
use crate::cst::control_flow::Match;
use crate::cst::control_flow::MatchArm;
use crate::cst::control_flow::Try;
use crate::cst::control_flow::TryCatchClause;
use crate::cst::control_flow::TryElseClause;
use crate::cst::control_flow::TryFinallyClause;
use crate::cst::control_flow::While;
use crate::cst::declaration::Attribute;
use crate::cst::declaration::AttributeList;
use crate::cst::declaration::Constant;
use crate::cst::declaration::Namespace;
use crate::cst::declaration::NamespaceBody;
use crate::cst::declaration::NamespaceImplicitBody;
use crate::cst::declaration::Use;
use crate::cst::declaration::UseItem;
use crate::cst::declaration::UseItemAlias;
use crate::cst::declaration::UseItemList;
use crate::cst::declaration::UseItemSequence;
use crate::cst::declaration::UseItems;
use crate::cst::expression::Break;
use crate::cst::expression::Continue;
use crate::cst::expression::Expression;
use crate::cst::expression::Instantiation;
use crate::cst::expression::InterpolatedString;
use crate::cst::expression::InterpolatedStringExpression;
use crate::cst::expression::InterpolatedStringLiteral;
use crate::cst::expression::InterpolatedStringPart;
use crate::cst::expression::Parenthesized;
use crate::cst::expression::Return;
use crate::cst::expression::Throw;
use crate::cst::function::Closure;
use crate::cst::function::ClosureUseClause;
use crate::cst::function::Function;
use crate::cst::function::Parameter;
use crate::cst::function::ParameterDefault;
use crate::cst::function::ParameterList;
use crate::cst::function::ReturnType;
use crate::cst::function::ShortClosure;
use crate::cst::function::ShortClosureBody;
use crate::cst::operation::Assignment;
use crate::cst::operation::AssignmentOperator;
use crate::cst::operation::AssignmentTarget;
use crate::cst::operation::Binary;
use crate::cst::operation::BinaryOperator;
use crate::cst::operation::DestructureDefault;
use crate::cst::operation::DestructureRest;
use crate::cst::operation::DestructureTarget;
use crate::cst::operation::DictDestructure;
use crate::cst::operation::DictDestructureEntry;
use crate::cst::operation::TupleDestructure;
use crate::cst::operation::TypeOperation;
use crate::cst::operation::TypeOperator;
use crate::cst::operation::UnaryPostfix;
use crate::cst::operation::UnaryPostfixOperator;
use crate::cst::operation::UnaryPrefix;
use crate::cst::operation::UnaryPrefixOperator;
use crate::cst::pattern::DictPatternKey;
use crate::cst::pattern::Pattern;
use crate::cst::statement::Block;
use crate::cst::statement::ExpressionStatement;
use crate::cst::statement::FinalLocal;
use crate::cst::statement::Statement;
use crate::cst::r#type::ArrayType;
use crate::cst::r#type::ClassnameType;
use crate::cst::r#type::DictType;
use crate::cst::r#type::FunctionType;
use crate::cst::r#type::FunctionTypeParameter;
use crate::cst::r#type::FunctionTypeSignature;
use crate::cst::r#type::IntegerRangeBound;
use crate::cst::r#type::IntegerRangeOperator;
use crate::cst::r#type::IntegerRangeType;
use crate::cst::r#type::IntersectionType;
use crate::cst::r#type::NamedType;
use crate::cst::r#type::NegatedType;
use crate::cst::r#type::NegativeLiteralType;
use crate::cst::r#type::Newtype;
use crate::cst::r#type::ParenthesizedType;
use crate::cst::r#type::TupleType;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeAlias;
use crate::cst::r#type::TypeArgument;
use crate::cst::r#type::TypeArgumentList;
use crate::cst::r#type::TypeParameter;
use crate::cst::r#type::TypeParameterBound;
use crate::cst::r#type::TypeParameterDefault;
use crate::cst::r#type::TypeParameterList;
use crate::cst::r#type::TypeVariance;
use crate::cst::r#type::UnionType;
use crate::cst::r#type::VecType;

macro_rules! define_nodes {
    ($($variant:ident($node:ty),)*) => {
        /// The kind of a [`Node`]: a cheap `Copy` tag, one variant per node type.
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum NodeKind {
            $($variant,)*
        }

        /// A borrowed reference to any node in the tree, tagged by its type.
        #[derive(Debug, Clone, Copy)]
        pub enum Node<'ast, 'arena> {
            $($variant(&'ast $node),)*
        }

        impl Node<'_, '_> {
            #[must_use]
            pub const fn kind(&self) -> NodeKind {
                match self {
                    $(Node::$variant(_) => NodeKind::$variant,)*
                }
            }
        }

        impl HasSpan for Node<'_, '_> {
            fn span(&self) -> Span {
                match self {
                    $(Node::$variant(node) => node.span(),)*
                }
            }
        }
    };
}

define_nodes! {
    Program(Program<'arena>),
    Statement(Statement<'arena>),
    ExpressionStatement(ExpressionStatement<'arena>),
    FinalLocal(FinalLocal<'arena>),
    Block(Block<'arena>),
    Return(Return<'arena>),
    Namespace(Namespace<'arena>),
    NamespaceBody(NamespaceBody<'arena>),
    NamespaceImplicitBody(NamespaceImplicitBody<'arena>),
    Use(Use<'arena>),
    UseItems(UseItems<'arena>),
    UseItemSequence(UseItemSequence<'arena>),
    UseItemList(UseItemList<'arena>),
    UseItem(UseItem<'arena>),
    UseItemAlias(UseItemAlias<'arena>),
    Constant(Constant<'arena>),
    TypeAlias(TypeAlias<'arena>),
    Newtype(Newtype<'arena>),
    AttributeList(AttributeList<'arena>),
    Attribute(Attribute<'arena>),
    Class(Class<'arena>),
    Interface(Interface<'arena>),
    Enum(Enum<'arena>),
    EnumBackingType(EnumBackingType<'arena>),
    Extends(Extends<'arena>),
    SealedPermissions(SealedPermissions<'arena>),
    Implements(Implements<'arena>),
    ClassLikeMember(ClassLikeMember<'arena>),
    ClassLikeConstant(ClassLikeConstant<'arena>),
    EnumCase(EnumCase<'arena>),
    EnumCaseValue(EnumCaseValue<'arena>),
    Method(Method<'arena>),
    MethodBody(MethodBody<'arena>),
    Property(Property<'arena>),
    PropertyDefault(PropertyDefault<'arena>),
    Modifier(Modifier<'arena>),
    Function(Function<'arena>),
    Closure(Closure<'arena>),
    ClosureUseClause(ClosureUseClause<'arena>),
    ShortClosure(ShortClosure<'arena>),
    ShortClosureBody(ShortClosureBody<'arena>),
    ParameterList(ParameterList<'arena>),
    Parameter(Parameter<'arena>),
    ParameterDefault(ParameterDefault<'arena>),
    ReturnType(ReturnType<'arena>),
    If(If<'arena>),
    Else(Else<'arena>),
    ElseBody(ElseBody<'arena>),
    While(While<'arena>),
    DoWhile(DoWhile<'arena>),
    For(For<'arena>),
    Foreach(Foreach<'arena>),
    ForeachTarget(ForeachTarget<'arena>),
    ForeachValueTarget(ForeachValueTarget<'arena>),
    ForeachKeyValueTarget(ForeachKeyValueTarget<'arena>),
    Break(Break<'arena>),
    Continue(Continue<'arena>),
    Try(Try<'arena>),
    TryCatchClause(TryCatchClause<'arena>),
    TryElseClause(TryElseClause<'arena>),
    TryFinallyClause(TryFinallyClause<'arena>),
    Match(Match<'arena>),
    MatchArm(MatchArm<'arena>),
    Pattern(Pattern<'arena>),
    DictPatternKey(DictPatternKey<'arena>),
    BindingTarget(BindingTarget<'arena>),
    Expression(Expression<'arena>),
    Parenthesized(Parenthesized<'arena>),
    InterpolatedString(InterpolatedString<'arena>),
    InterpolatedStringPart(InterpolatedStringPart<'arena>),
    InterpolatedStringLiteral(InterpolatedStringLiteral<'arena>),
    InterpolatedStringExpression(InterpolatedStringExpression<'arena>),
    Throw(Throw<'arena>),
    Construct(Construct<'arena>),
    RequireConstruct(RequireConstruct<'arena>),
    RequireOnceConstruct(RequireOnceConstruct<'arena>),
    LengthConstruct(LengthConstruct<'arena>),
    ContainsConstruct(ContainsConstruct<'arena>),
    ContainsKeyConstruct(ContainsKeyConstruct<'arena>),
    CloneConstruct(CloneConstruct<'arena>),
    CloneField(CloneField<'arena>),
    RemoveConstruct(RemoveConstruct<'arena>),
    SwapRemoveConstruct(SwapRemoveConstruct<'arena>),
    RemoveFirstConstruct(RemoveFirstConstruct<'arena>),
    RemoveLastConstruct(RemoveLastConstruct<'arena>),
    AssertConstruct(AssertConstruct<'arena>),
    AssertMessage(AssertMessage<'arena>),
    ExitConstruct(ExitConstruct<'arena>),
    PanicConstruct(PanicConstruct<'arena>),
    WriteConstruct(WriteConstruct<'arena>),
    WriteLineConstruct(WriteLineConstruct<'arena>),
    WriteErrorConstruct(WriteErrorConstruct<'arena>),
    WriteErrorLineConstruct(WriteErrorLineConstruct<'arena>),
    DebugConstruct(DebugConstruct<'arena>),
    DiscardConstruct(DiscardConstruct<'arena>),
    DropConstruct(DropConstruct<'arena>),
    FileConstruct(FileConstruct<'arena>),
    DirectoryConstruct(DirectoryConstruct<'arena>),
    EmbedConstruct(EmbedConstruct<'arena>),
    ConstructArgument(ConstructArgument<'arena>),
    Instantiation(Instantiation<'arena>),
    Binary(Binary<'arena>),
    BinaryOperator(BinaryOperator),
    UnaryPrefix(UnaryPrefix<'arena>),
    UnaryPrefixOperator(UnaryPrefixOperator),
    UnaryPostfix(UnaryPostfix<'arena>),
    UnaryPostfixOperator(UnaryPostfixOperator),
    TypeOperation(TypeOperation<'arena>),
    TypeOperator(TypeOperator<'arena>),
    Assignment(Assignment<'arena>),
    AssignmentOperator(AssignmentOperator),
    AssignmentTarget(AssignmentTarget<'arena>),
    TupleDestructure(TupleDestructure<'arena>),
    DictDestructure(DictDestructure<'arena>),
    DictDestructureEntry(DictDestructureEntry<'arena>),
    DestructureTarget(DestructureTarget<'arena>),
    DestructureDefault(DestructureDefault<'arena>),
    DestructureRest(DestructureRest<'arena>),
    Access(Access<'arena>),
    ClassReference(ClassReference<'arena>),
    NamedClassReference(NamedClassReference<'arena>),
    ConstantAccess(ConstantAccess<'arena>),
    PropertyAccess(PropertyAccess<'arena>),
    NullSafePropertyAccess(NullSafePropertyAccess<'arena>),
    StaticPropertyAccess(StaticPropertyAccess<'arena>),
    ClassConstantAccess(ClassConstantAccess<'arena>),
    VecExpression(VecExpression<'arena>),
    VecFillExpression(VecFillExpression<'arena>),
    VecElement(VecElement<'arena>),
    DictExpression(DictExpression<'arena>),
    DictEntry(DictEntry<'arena>),
    DictPair(DictPair<'arena>),
    DictSpread(DictSpread<'arena>),
    TupleExpression(TupleExpression<'arena>),
    TupleElement(TupleElement<'arena>),
    TupleRest(TupleRest<'arena>),
    ArrayAccess(ArrayAccess<'arena>),
    ArrayAppend(ArrayAppend<'arena>),
    Callee(Callee<'arena>),
    Call(Call<'arena>),
    FunctionCall(FunctionCall<'arena>),
    MethodCall(MethodCall<'arena>),
    NullSafeMethodCall(NullSafeMethodCall<'arena>),
    StaticMethodCall(StaticMethodCall<'arena>),
    ArgumentList(ArgumentList<'arena>),
    PartialArgumentList(PartialArgumentList<'arena>),
    Argument(Argument<'arena>),
    PartialArgument(PartialArgument<'arena>),
    PositionalArgument(PositionalArgument<'arena>),
    NamedArgument(NamedArgument<'arena>),
    NamedPlaceholderArgument(NamedPlaceholderArgument<'arena>),
    PlaceholderArgument(PlaceholderArgument),
    VariadicPlaceholderArgument(VariadicPlaceholderArgument),
    PartialApplication(PartialApplication<'arena>),
    FunctionPartialApplication(FunctionPartialApplication<'arena>),
    MethodPartialApplication(MethodPartialApplication<'arena>),
    StaticMethodPartialApplication(StaticMethodPartialApplication<'arena>),
    Type(Type<'arena>),
    NamedType(NamedType<'arena>),
    TypeArgumentList(TypeArgumentList<'arena>),
    TypeArgument(TypeArgument<'arena>),
    TypeParameterList(TypeParameterList<'arena>),
    TypeParameter(TypeParameter<'arena>),
    TypeVariance(TypeVariance<'arena>),
    TypeParameterBound(TypeParameterBound<'arena>),
    TypeParameterDefault(TypeParameterDefault<'arena>),
    UnionType(UnionType<'arena>),
    IntersectionType(IntersectionType<'arena>),
    NegatedType(NegatedType<'arena>),
    NegativeLiteralType(NegativeLiteralType<'arena>),
    IntegerRangeType(IntegerRangeType<'arena>),
    IntegerRangeBound(IntegerRangeBound<'arena>),
    IntegerRangeOperator(IntegerRangeOperator),
    ParenthesizedType(ParenthesizedType<'arena>),
    FunctionType(FunctionType<'arena>),
    FunctionTypeSignature(FunctionTypeSignature<'arena>),
    FunctionTypeParameter(FunctionTypeParameter<'arena>),
    ArrayType(ArrayType<'arena>),
    VecType(VecType<'arena>),
    DictType(DictType<'arena>),
    ClassnameType(ClassnameType<'arena>),
    TupleType(TupleType<'arena>),
    Keyword(Keyword<'arena>),
    Identifier(Identifier<'arena>),
    LocalIdentifier(LocalIdentifier<'arena>),
    QualifiedIdentifier(QualifiedIdentifier<'arena>),
    FullyQualifiedIdentifier(FullyQualifiedIdentifier<'arena>),
    Variable(Variable<'arena>),
    Literal(Literal<'arena>),
    LiteralString(LiteralString<'arena>),
    LiteralInteger(LiteralInteger<'arena>),
    LiteralFloat(LiteralFloat<'arena>),
}
