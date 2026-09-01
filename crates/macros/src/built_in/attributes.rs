//! Parsing the structured attribute arguments the signature-string path uses:
//! leading positional literals (a name, a type), bare flags (`final`,
//! `readonly`, `abstract`, `static`), and `key = literal` pairs (`visibility =
//! "public"`, `literal = 5`).

use std::collections::BTreeMap;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Expr;
use syn::Lit;
use syn::LitStr;
use syn::Token;
use syn::ext::IdentExt;
use syn::parse::Parse;
use syn::parse::ParseStream;
use syn::token::Comma;

pub(super) struct AttributeArguments {
    positional: Vec<Lit>,
    named: BTreeMap<String, NamedArgument>,
}

enum NamedArgument {
    Flag(Span),
    Value(Expr, Span),
}

impl Parse for AttributeArguments {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut positional = Vec::new();
        let mut named = BTreeMap::new();

        while !input.is_empty() {
            if input.peek(Lit) {
                positional.push(input.parse::<Lit>()?);
            } else {
                let identifier = input.call(syn::Ident::parse_any)?;
                let span = identifier.span();
                let key = identifier.to_string();
                if named.contains_key(&key) {
                    return Err(syn::Error::new(span, format!("duplicate option `{key}`")));
                }
                if input.peek(Token![=]) {
                    input.parse::<Token![=]>()?;
                    named.insert(key, NamedArgument::Value(input.parse::<Expr>()?, span));
                } else {
                    named.insert(key, NamedArgument::Flag(span));
                }
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Comma>()?;
        }

        Ok(Self { positional, named })
    }
}

impl AttributeArguments {
    pub(super) fn validate(
        &self,
        positional: usize,
        flags: &[&str],
        values: &[&str],
    ) -> syn::Result<()> {
        if let Some(extra) = self.positional.get(positional) {
            return Err(syn::Error::new_spanned(
                extra,
                "unexpected positional argument",
            ));
        }
        for (name, argument) in &self.named {
            match argument {
                NamedArgument::Flag(span) if values.contains(&name.as_str()) => {
                    return Err(syn::Error::new(
                        *span,
                        format!("option `{name}` needs a value"),
                    ));
                }
                NamedArgument::Flag(span) if !flags.contains(&name.as_str()) => {
                    return Err(syn::Error::new(*span, format!("unknown option `{name}`")));
                }
                NamedArgument::Value(_, span) if flags.contains(&name.as_str()) => {
                    return Err(syn::Error::new(
                        *span,
                        format!("flag `{name}` takes no value"),
                    ));
                }
                NamedArgument::Value(_, span) if !values.contains(&name.as_str()) => {
                    return Err(syn::Error::new(*span, format!("unknown option `{name}`")));
                }
                NamedArgument::Flag(_) | NamedArgument::Value(_, _) => {}
            }
        }

        Ok(())
    }

    pub(super) fn positional_string(&self, index: usize) -> syn::Result<Option<LitStr>> {
        match self.positional.get(index) {
            Some(Lit::Str(literal)) => Ok(Some(literal.clone())),
            Some(literal) => Err(syn::Error::new_spanned(
                literal,
                "expected a string literal",
            )),
            None => Ok(None),
        }
    }

    pub(super) fn has_flag(&self, name: &str) -> bool {
        matches!(self.named.get(name), Some(NamedArgument::Flag(_)))
    }

    pub(super) fn value_string(&self, key: &str) -> syn::Result<Option<String>> {
        match self.value_expr(key) {
            Some(Expr::Lit(expression)) => match &expression.lit {
                Lit::Str(literal) => Ok(Some(literal.value())),
                literal => Err(syn::Error::new_spanned(
                    literal,
                    format!("option `{key}` needs a string literal"),
                )),
            },
            Some(expression) => Err(syn::Error::new_spanned(
                expression,
                format!("option `{key}` needs a string literal"),
            )),
            None => Ok(None),
        }
    }

    pub(super) fn value_lit(&self, key: &str) -> syn::Result<Option<&Lit>> {
        match self.value_expr(key) {
            Some(Expr::Lit(expression)) => Ok(Some(&expression.lit)),
            Some(expression) => Err(syn::Error::new_spanned(
                expression,
                format!("option `{key}` needs a literal"),
            )),
            None => Ok(None),
        }
    }

    pub(super) fn value_expr(&self, key: &str) -> Option<&Expr> {
        match self.named.get(key) {
            Some(NamedArgument::Value(expression, _)) => Some(expression),
            Some(NamedArgument::Flag(_)) | None => None,
        }
    }
}

pub(super) fn visibility_tokens(visibility: Option<&str>) -> syn::Result<TokenStream> {
    let path = quote!(crate::bytecode::unit::Visibility);
    match visibility {
        None | Some("public") => Ok(quote!(#path::Public)),
        Some("protected") => Ok(quote!(#path::Protected)),
        Some("private") => Ok(quote!(#path::Private)),
        Some(other) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "visibility must be \"public\", \"protected\", or \"private\", found {other:?}"
            ),
        )),
    }
}

pub(super) fn constant_value_tokens(literal: &Lit) -> syn::Result<TokenStream> {
    let path = quote!(crate::builtin::spec::ConstantValue);
    match literal {
        Lit::Int(value) => Ok(quote!(#path::Int(#value))),
        Lit::Float(value) => Ok(quote!(#path::Float(#value))),
        Lit::Bool(value) => Ok(quote!(#path::Bool(#value))),
        Lit::Str(value) => Ok(quote!(#path::String(#value))),
        other => Err(syn::Error::new_spanned(
            other,
            "a constant value must be an int, float, bool, or string literal",
        )),
    }
}
