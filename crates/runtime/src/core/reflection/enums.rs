//! Native reflection enums.

use whim_macros::whim_enum;

#[whim_enum("Whim\\Reflection\\DeclarationOrigin")]
pub(crate) enum DeclarationOrigin {
    #[whim_case("Core")]
    Core,
    #[whim_case("Extension")]
    Extension,
    #[whim_case("User")]
    User,
}

#[whim_enum("Whim\\Reflection\\Attribute\\Target")]
pub(crate) enum AttributeTarget {
    #[whim_case("Class")]
    Class,
    #[whim_case("Function")]
    Function,
    #[whim_case("Method")]
    Method,
    #[whim_case("Property")]
    Property,
    #[whim_case("ClassConstant")]
    ClassConstant,
    #[whim_case("Parameter")]
    Parameter,
    #[whim_case("TypeAlias")]
    TypeAlias,
    #[whim_case("Newtype")]
    Newtype,
    #[whim_case("Constant")]
    Constant,
}

#[whim_enum("Whim\\Reflection\\Callable\\CallableKind")]
pub(crate) enum CallableKind {
    #[whim_case("Function")]
    Function,
    #[whim_case("StaticMethod")]
    StaticMethod,
    #[whim_case("InstanceMethod")]
    InstanceMethod,
    #[whim_case("Closure")]
    Closure,
    #[whim_case("ShortClosure")]
    ShortClosure,
    #[whim_case("Partial")]
    Partial,
}

#[whim_enum("Whim\\Reflection\\Generic\\Variance")]
pub(crate) enum Variance {
    #[whim_case("Invariant")]
    Invariant,
    #[whim_case("Covariant")]
    Covariant,
    #[whim_case("Contravariant")]
    Contravariant,
}

#[whim_enum("Whim\\Reflection\\Member\\Visibility")]
pub(crate) enum Visibility {
    #[whim_case("Public")]
    Public,
    #[whim_case("Protected")]
    Protected,
    #[whim_case("Private")]
    Private,
}

#[whim_enum("Whim\\Reflection\\Type\\TypeKind")]
pub(crate) enum TypeKind {
    #[whim_case("Mixed")]
    Mixed,
    #[whim_case("Never")]
    Never,
    #[whim_case("Void")]
    Void,
    #[whim_case("Null")]
    Null,
    #[whim_case("Bool")]
    Bool,
    #[whim_case("Int")]
    Int,
    #[whim_case("Float")]
    Float,
    #[whim_case("String")]
    String,
    #[whim_case("StringLength")]
    StringLength,
    #[whim_case("Object")]
    Object,
    #[whim_case("Literal")]
    Literal,
    #[whim_case("IntegerRange")]
    IntegerRange,
    #[whim_case("Named")]
    Named,
    #[whim_case("Member")]
    Member,
    #[whim_case("TypeParameter")]
    TypeParameter,
    #[whim_case("Static")]
    Static,
    #[whim_case("Union")]
    Union,
    #[whim_case("Intersection")]
    Intersection,
    #[whim_case("Negated")]
    Negated,
    #[whim_case("Function")]
    Function,
    #[whim_case("Array")]
    Array,
    #[whim_case("Vec")]
    Vec,
    #[whim_case("VecShape")]
    VecShape,
    #[whim_case("Dict")]
    Dict,
    #[whim_case("DictShape")]
    DictShape,
    #[whim_case("Classname")]
    Classname,
    #[whim_case("Tuple")]
    Tuple,
    #[whim_case("Wildcard")]
    Wildcard,
}
