//! Name resolution: namespaces and `use` imports to fully qualified atoms.

use std::rc::Rc;

use hashbrown::HashMap;
use hashbrown::HashSet;

use whim_span::HasSpan;
use whim_syn::cst::atom::Identifier;
use whim_syn::cst::declaration::Use;
use whim_syn::cst::declaration::UseItem;
use whim_syn::cst::declaration::UseItems;

use crate::compiler::error::CompileError;
use crate::compiler::error::CompileErrorKind;
use crate::unreachable_invariant;
use crate::value::atom::Atom;
use crate::value::heap::Heap;

#[derive(Default, Clone)]
pub(in crate::compiler) struct Resolver {
    state: Rc<ResolverState>,
}

#[derive(Default, Clone)]
struct ResolverState {
    /// The current namespace, without a trailing separator; empty at the
    /// global level.
    namespace: String,
    aliases: HashMap<String, String>,
}

impl Resolver {
    pub(in crate::compiler) fn for_namespace(namespace: &str) -> Self {
        Self {
            state: Rc::new(ResolverState {
                namespace: namespace.to_string(),
                aliases: HashMap::new(),
            }),
        }
    }

    pub(in crate::compiler) fn collect_use(
        &mut self,
        declaration: &Use<'_>,
        declared: &HashSet<String>,
    ) -> Result<(), CompileError> {
        match &declaration.items {
            UseItems::Sequence(sequence) => {
                for item in &sequence.items {
                    self.collect_item(item, "", declared)?;
                }
            }
            UseItems::List(list) => {
                let prefix = strip_leading_separator(list.namespace.value());
                for item in &list.items {
                    self.collect_item(item, prefix, declared)?;
                }
            }
        }

        Ok(())
    }

    pub(in crate::compiler) fn has_alias(&self, name: &str) -> bool {
        self.state.aliases.contains_key(name)
    }

    fn collect_item(
        &mut self,
        item: &UseItem<'_>,
        prefix: &str,
        declared: &HashSet<String>,
    ) -> Result<(), CompileError> {
        let name = strip_leading_separator(item.name.value());
        let qualified = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}\\{name}")
        };

        let alias = item.alias.as_ref().map_or_else(
            || last_segment(&qualified).to_string(),
            |alias| alias.identifier.value.to_string(),
        );

        if self.state.aliases.contains_key(&alias) {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateImportAlias,
                format!("`{alias}` is already imported in this namespace"),
                item.span(),
            ));
        }

        if declared.contains(&alias) {
            return Err(CompileError::new(
                CompileErrorKind::DuplicateImportAlias,
                format!("the import `{alias}` collides with a declaration of the same name"),
                item.span(),
            ));
        }

        // Earlier statements retain their resolver snapshot when an import changes.
        Rc::make_mut(&mut self.state)
            .aliases
            .insert(alias, qualified);

        Ok(())
    }

    pub(in crate::compiler) fn resolve(&self, heap: &Heap, identifier: &Identifier<'_>) -> Atom {
        heap.intern(self.resolve_text(identifier).as_bytes())
    }

    pub(in crate::compiler) fn resolve_text(&self, identifier: &Identifier<'_>) -> String {
        match identifier {
            Identifier::FullyQualified(name) => strip_leading_separator(name.value).to_string(),
            Identifier::Local(name) => self
                .state
                .aliases
                .get(name.value)
                .map_or_else(|| self.qualify(name.value), Clone::clone),
            Identifier::Qualified(name) => {
                let Some((first, rest)) = name.value.split_once('\\') else {
                    // SAFETY: the parser marks only names with a separator as qualified.
                    unsafe { unreachable_invariant("a qualified identifier contains a separator") }
                };
                self.state.aliases.get(first).map_or_else(
                    || self.qualify(name.value),
                    |imported| format!("{imported}\\{rest}"),
                )
            }
        }
    }

    pub(in crate::compiler) fn qualify(&self, name: &str) -> String {
        if self.state.namespace.is_empty() {
            name.to_string()
        } else {
            format!("{}\\{name}", self.state.namespace)
        }
    }
}

fn strip_leading_separator(name: &str) -> &str {
    name.strip_prefix('\\').unwrap_or(name)
}

fn last_segment(name: &str) -> &str {
    name.rfind('\\')
        .map_or(name, |position| &name[position + 1..])
}
