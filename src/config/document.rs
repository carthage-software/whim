use std::path::Path;

use toml_edit::DocumentMut;
use toml_edit::Item;
use toml_edit::Table;
use toml_edit::value;

use crate::config::Error;
use crate::package::Source;

#[derive(Clone, Copy)]
pub(crate) enum DependencyGroup {
    Runtime,
    Development,
}

impl DependencyGroup {
    const fn name(self) -> &'static str {
        match self {
            Self::Runtime => "dependencies",
            Self::Development => "dev-dependencies",
        }
    }
}

pub(crate) struct EditableManifest {
    document: DocumentMut,
}

impl EditableManifest {
    pub(crate) fn parse(path: &Path, text: &str) -> Result<Self, Error> {
        let document = text
            .parse::<DocumentMut>()
            .map_err(|source| Error::EditManifest {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self { document })
    }

    pub(crate) fn find(
        &self,
        group: DependencyGroup,
        source: &Source,
    ) -> Result<Option<String>, Error> {
        let Some(item) = self.document.get(group.name()) else {
            return Ok(None);
        };

        let table = item
            .as_table()
            .ok_or_else(|| Error::ExpectedTable(group.name()))?;

        for (spelling, _) in table {
            let candidate =
                Source::parse(spelling).map_err(|source| Error::InvalidDependencySource {
                    group: group.name(),
                    source,
                })?;

            if candidate == *source {
                return Ok(Some(spelling.to_owned()));
            }
        }

        Ok(None)
    }

    pub(crate) fn insert(
        &mut self,
        group: DependencyGroup,
        spelling: &str,
        requirement: String,
    ) -> Result<(), Error> {
        self.table(group)?.insert(spelling, value(requirement));
        Ok(())
    }

    pub(crate) fn remove(&mut self, group: DependencyGroup, spelling: &str) -> Result<(), Error> {
        self.table(group)?.remove(spelling);
        Ok(())
    }

    pub(crate) fn render(self) -> String {
        self.document.to_string()
    }

    fn table(&mut self, group: DependencyGroup) -> Result<&mut Table, Error> {
        let name = group.name();
        if self.document.get(name).is_none() {
            self.document[name] = Item::Table(Table::new());
        }

        self.document[name]
            .as_table_mut()
            .ok_or(Error::ExpectedTable(name))
    }
}
