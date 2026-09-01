use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::fmt::Write;

use thiserror::Error as ThisError;

use crate::config::Manifest;
use crate::package::source::Source;

const LOADER_BODY: &str = r"];

Autoload\register(
  new Autoload\Autoloader()->withFallback(
    function (
      SymbolKind $kind,
      string $name,
    ) use ($namespaces): bool {
      $separator = Str\search($name, '\\');
      $segment = match ($separator) {
        null => $name,
        $_ => Str\slice($name, 0, $separator),
      };
      if (!contains_key!($namespaces, $segment)) {
        return false;
      }

      foreach ($namespaces[$segment] as ($prefix, $root)) {
        if (!Str\starts_with($name, $prefix)) {
          continue;
        }

        $relative = Str\slice($name, length!($prefix));
        $exact = $root . '/' . Str\replace($relative, '\\', '/') . '.whim';
        if (Filesystem\is_file($exact)) {
          require_once!($exact);
          if (
            Whim\Symbol\exists($name, false)
            && Whim\Symbol\get_kind($name) == $kind
          ) {
            return true;
          }
        }

        $slash = Str\search_last($relative, '\\');
        $namespace = match ($slash) {
          null => $root,
          $_ => $root . '/' . Str\replace(
            Str\slice($relative, 0, $slash),
            '\\',
            '/',
          ),
        };

        $group = match ($kind) {
          SymbolKind::Class => 'classes.whim',
          SymbolKind::Interface => 'interfaces.whim',
          SymbolKind::Enum => 'enums.whim',
          SymbolKind::TypeAlias | SymbolKind::Newtype => 'types.whim',
          SymbolKind::Function => 'functions.whim',
          SymbolKind::Constant => 'constants.whim',
        };

        $group = $namespace . '/' . $group;
        if (Filesystem\is_file($group)) {
          require_once!($group);
          if (
            Whim\Symbol\exists($name, false)
            && Whim\Symbol\get_kind($name) == $kind
          ) {
            return true;
          }
        }

      }

      return false;
    },
  ),
);
";

struct Mapping {
    root: String,
    path: String,
}

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("could not render the generated autoloader")]
    Render(#[source] fmt::Error),
    #[error("autoload prefix `{0}` is declared by multiple sources")]
    DuplicatePrefix(String),
}

#[tracing::instrument(level = "debug", skip(root, packages), fields(packages = packages.len()))]
pub(crate) fn generate(
    root: &Manifest,
    packages: &BTreeMap<Source, Manifest>,
) -> Result<String, Error> {
    let mut prefixes = BTreeMap::<String, Mapping>::new();
    add_mappings(&mut prefixes, root, "directory!() . '/..'")?;
    for (source, manifest) in packages {
        let root = format!("directory!() . '/packages/{}'", source.digest());
        add_mappings(&mut prefixes, manifest, &root)?;
    }

    let mut buckets = BTreeMap::<String, Vec<(String, Mapping)>>::new();
    for (prefix, mapping) in prefixes {
        let segment = prefix
            .split_once('\\')
            .map_or(prefix.as_str(), |(segment, _)| segment);
        buckets
            .entry(segment.to_owned())
            .or_default()
            .push((prefix, mapping));
    }

    for mappings in buckets.values_mut() {
        mappings.sort_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then_with(|| left.0.cmp(&right.0))
        });
    }

    let bucket_count = buckets.len();

    let mut output = String::from(
        "use Whim\\Autoload;\nuse Whim\\Filesystem;\nuse Whim\\Str;\nuse Whim\\Symbol\\SymbolKind;\n\n$namespaces = dict[\n",
    );

    for (segment, mappings) in buckets {
        writeln!(output, "  '{}' => vec[", quote(&segment)).map_err(Error::Render)?;
        for (prefix, mapping) in mappings {
            writeln!(
                output,
                "    ('{}', {} . '/{}'),",
                quote(&prefix),
                mapping.root,
                quote(&mapping.path)
            )
            .map_err(Error::Render)?;
        }

        output.push_str("  ],\n");
    }

    output.push_str(LOADER_BODY);
    tracing::debug!(
        buckets = bucket_count,
        bytes = output.len(),
        "generated dependency autoloader"
    );

    Ok(output)
}

fn add_mappings(
    prefixes: &mut BTreeMap<String, Mapping>,
    manifest: &Manifest,
    root: &str,
) -> Result<(), Error> {
    for (prefix, directory) in &manifest.autoload.namespaces {
        match prefixes.entry(prefix.clone()) {
            Entry::Occupied(_) => return Err(Error::DuplicatePrefix(prefix.clone())),
            Entry::Vacant(entry) => {
                entry.insert(Mapping {
                    root: root.to_owned(),
                    path: directory.trim_end_matches('/').to_owned(),
                });
            }
        }
    }

    Ok(())
}

fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            character => quoted.push(character),
        }
    }

    quoted
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::config::Manifest;
    use crate::package::loader::generate;
    use crate::package::source::Source;

    #[test]
    fn nested_prefixes_are_emitted_longest_first() {
        let root = Manifest::parse(
            "manifest-version = 1\n[autoload.namespaces]\n\"App\\\\\" = \"src/\"\n",
            true,
        )
        .expect("the root manifest is valid");
        let dependency = Manifest::parse(
            "manifest-version = 1\n[autoload.namespaces]\n\"App\\\\Feature\\\\\" = \"src/\"\n",
            false,
        )
        .expect("the dependency manifest is valid");
        let source =
            Source::parse("git+https://github.com/acme/feature").expect("the source is valid");
        let loader = generate(&root, &BTreeMap::from([(source, dependency)]))
            .expect("the loader is generated");

        let nested = loader
            .find("('App\\\\Feature\\\\',")
            .expect("the nested prefix is emitted");
        let parent = loader
            .find("('App\\\\',")
            .expect("the parent prefix is emitted");
        assert!(nested < parent);
    }

    #[test]
    fn duplicate_prefixes_are_rejected() {
        let text = "manifest-version = 1\n[autoload.namespaces]\n\"App\\\\\" = \"src/\"\n";
        let root = Manifest::parse(text, true).expect("the root manifest is valid");
        let dependency = Manifest::parse(text, false).expect("the dependency manifest is valid");
        let source =
            Source::parse("git+https://github.com/acme/application").expect("the source is valid");

        assert!(generate(&root, &BTreeMap::from([(source, dependency)])).is_err());
    }
}
