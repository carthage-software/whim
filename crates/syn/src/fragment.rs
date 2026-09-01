use std::error;
use std::fmt;

use crate::arena::Arena;
use crate::cst::class::ClassLikeMember;
use crate::cst::class::Property;
use crate::cst::function::Function;
use crate::cst::statement::Statement;
use crate::cst::r#type::Type;
use crate::cst::r#type::TypeAlias;
use crate::cst::r#type::TypeParameterList;
use crate::error::ParseError;
use crate::parser;

/// A fragment that could not be parsed, carrying a rendered diagnostic.
#[derive(Debug, Clone)]
pub struct FragmentError {
    message: String,
    source: Option<ParseError>,
}

impl FragmentError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// The rendered parser diagnostic(s).
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FragmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl error::Error for FragmentError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &dyn error::Error)
    }
}

impl From<ParseError> for FragmentError {
    fn from(source: ParseError) -> Self {
        Self {
            message: source.to_string(),
            source: Some(source),
        }
    }
}

/// Parses a bare type expression, e.g. `"float"`, `"vec<int>"`, or
/// `"int|string"`.
///
/// # Errors
///
/// Returns an error when `source` is not one valid type.
pub fn parse_type<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena Type<'arena>, FragmentError>
where
    A: Arena,
{
    let wrapped = format!("type __fragment = {source};");
    let program = parser::parse(arena, &wrapped)?;

    match program.statements {
        [Statement::TypeAlias(alias)] => Ok(alias.aliased),
        _ => Err(FragmentError::new("expected a single type")),
    }
}

/// Parses a function signature fragment: everything a `function` declaration
/// carries between its name and its body, e.g. `"<T>(vec<T> $items): int"` or
/// `"(): void"`.
///
/// # Errors
///
/// Returns an error when `source` is not one valid function signature.
pub fn parse_signature<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena Function<'arena>, FragmentError>
where
    A: Arena,
{
    let wrapped = format!("function __signature{source} {{}}");
    let program = parser::parse(arena, &wrapped)?;

    match program.statements {
        [Statement::Function(function)] => Ok(function),
        _ => Err(FragmentError::new("expected a function signature")),
    }
}

/// Parses a type-alias body without its name.
///
/// # Errors
///
/// Returns an error when `source` is not one valid type-alias body.
pub fn parse_type_alias<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena TypeAlias<'arena>, FragmentError>
where
    A: Arena,
{
    let wrapped = format!("type __alias{source};");
    let program = parser::parse(arena, &wrapped)?;

    match program.statements {
        [Statement::TypeAlias(alias)] => Ok(alias),
        _ => Err(FragmentError::new("expected a type alias")),
    }
}

/// Parses a type-parameter list fragment, e.g. `"<out T, in U: Bound = int>"`.
/// Used to lift the generics off a class or interface name.
///
/// # Errors
///
/// Returns an error when `source` is not one valid type-parameter list.
pub fn parse_type_parameters<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena TypeParameterList<'arena>, FragmentError>
where
    A: Arena,
{
    let wrapped = format!("type __parameters{source} = int;");
    let program = parser::parse(arena, &wrapped)?;

    match program.statements {
        [Statement::TypeAlias(alias)] => alias
            .type_parameters
            .as_ref()
            .ok_or_else(|| FragmentError::new("expected a type-parameter list")),
        _ => Err(FragmentError::new("expected a type-parameter list")),
    }
}

/// Parses a single property declaration fragment, e.g.
/// `"public readonly T $value"`. The trailing `;` is added by the parser.
///
/// # Errors
///
/// Returns an error when `source` is not one valid property declaration.
pub fn parse_property<'arena, A>(
    arena: &'arena A,
    source: &str,
) -> Result<&'arena Property<'arena>, FragmentError>
where
    A: Arena,
{
    let wrapped = format!("class __Property {{ {source}; }}");
    let program = parser::parse(arena, &wrapped)?;

    match program.statements {
        [Statement::Class(class)] => match class.members {
            [ClassLikeMember::Property(property)] => Ok(property),
            _ => Err(FragmentError::new("expected a single property declaration")),
        },
        _ => Err(FragmentError::new("expected a property declaration")),
    }
}

#[cfg(test)]
mod tests {
    use crate::arena::LocalArena;
    use crate::cst::atom::Modifier;
    use crate::cst::r#type::Type;
    use crate::fragment::*;

    #[test]
    fn parses_scalar_and_composite_types() {
        let arena = LocalArena::new();

        assert!(matches!(parse_type(&arena, "float"), Ok(Type::Float(_))));
        assert!(matches!(parse_type(&arena, "int"), Ok(Type::Int(_))));
        assert!(matches!(parse_type(&arena, "vec<int>"), Ok(Type::Vec(_))));
        assert!(matches!(
            parse_type(&arena, "int|string"),
            Ok(Type::Union(_))
        ));
        assert!(matches!(
            parse_type(&arena, "(int, string)"),
            Ok(Type::Tuple(_))
        ));
    }

    #[test]
    fn rejects_a_malformed_type() {
        let arena = LocalArena::new();
        let error = parse_type(&arena, "int|").expect_err("the type must be rejected");

        assert!(error::Error::source(&error).is_some());
    }

    #[test]
    fn accepts_negative_and_rejects_positive_numeric_types() {
        let arena = LocalArena::new();

        assert!(matches!(
            parse_type(&arena, "-1"),
            Ok(Type::NegativeLiteral(_))
        ));
        assert!(parse_type(&arena, "+1").is_err());
        assert!(matches!(
            parse_type(&arena, "-1.5"),
            Ok(Type::NegativeLiteral(_))
        ));
        assert!(parse_type(&arena, "+1.5").is_err());
    }

    #[test]
    fn parses_a_generic_signature() {
        let arena = LocalArena::new();

        let signature = parse_signature(&arena, "<T>(vec<T> $items): int")
            .expect("the generic signature is valid");
        assert!(signature.type_parameters.is_some());
        assert_eq!(signature.parameter_list.parameters.len(), 1);
        assert!(signature.return_type.is_some());
    }

    #[test]
    fn parses_a_niladic_signature() {
        let arena = LocalArena::new();

        let signature =
            parse_signature(&arena, "(): void").expect("the niladic signature is valid");
        assert!(signature.type_parameters.is_none());
        assert!(signature.parameter_list.parameters.is_empty());
        assert!(signature.return_type.is_some());
    }

    #[test]
    fn parses_a_type_alias_body() {
        let arena = LocalArena::new();

        let alias =
            parse_type_alias(&arena, "<A, B> = (A, B)").expect("the generic type alias is valid");
        assert!(alias.type_parameters.is_some());
        assert!(matches!(alias.aliased, Type::Tuple(_)));

        let plain = parse_type_alias(&arena, "= int").expect("the plain type alias is valid");
        assert!(plain.type_parameters.is_none());
        assert!(matches!(plain.aliased, Type::Int(_)));
    }

    #[test]
    fn parses_a_type_parameter_list() {
        let arena = LocalArena::new();

        let list =
            parse_type_parameters(&arena, "<out T, out E>").expect("the parameter list is valid");
        assert_eq!(list.parameters.len(), 2);

        let single = parse_type_parameters(&arena, "<T>").expect("the single parameter is valid");
        assert_eq!(single.parameters.len(), 1);
    }

    #[test]
    fn parses_a_property_declaration() {
        let arena = LocalArena::new();

        let property = parse_property(&arena, "public readonly T $value")
            .expect("the property declaration is valid");
        assert_eq!(property.variable.name, "$value");
        assert!(property.r#type.is_some());
        assert!(property.modifiers.iter().any(Modifier::is_readonly));
    }
}
