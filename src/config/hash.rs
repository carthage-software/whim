use crate::config::Error;
use crate::config::Manifest;

impl Manifest {
    pub(crate) fn resolution_hash(&self) -> Result<String, Error> {
        let mut hasher = blake3::Hasher::new();
        hash_text(&mut hasher, "root-manifest-v1");
        self.hash_common(&mut hasher)?;
        hash_text(&mut hasher, "development");
        for requirement in self.development_requirements()? {
            hash_text(&mut hasher, requirement.source.identity());
            hash_text(&mut hasher, &requirement.requirement.to_string());
        }

        hash_text(&mut hasher, "overrides");
        for (source, replacement) in self.normalized_overrides()? {
            hash_text(&mut hasher, source.identity());
            hash_text(&mut hasher, replacement.identity());
        }

        Ok(format!("blake3:{}", hasher.finalize().to_hex()))
    }

    pub(crate) fn consumed_resolution_hash(&self) -> Result<String, Error> {
        let mut hasher = blake3::Hasher::new();
        hash_text(&mut hasher, "consumed-manifest-v1");
        self.hash_common(&mut hasher)?;
        Ok(format!("blake3:{}", hasher.finalize().to_hex()))
    }

    fn hash_common(&self, hasher: &mut blake3::Hasher) -> Result<(), Error> {
        hash_text(hasher, "requirements");
        hash_optional(hasher, self.requirements.whim.as_deref());
        hash_text(hasher, "autoload");
        for (prefix, path) in &self.autoload.namespaces {
            hash_text(hasher, prefix);
            hash_text(hasher, path);
        }

        hash_text(hasher, "runtime");
        for requirement in self.runtime_requirements()? {
            hash_text(hasher, requirement.source.identity());
            hash_text(hasher, &requirement.requirement.to_string());
        }

        hash_text(hasher, "conflicts");
        for requirement in self.conflict_requirements()? {
            hash_text(hasher, requirement.source.identity());
            hash_text(hasher, &requirement.requirement.to_string());
        }

        Ok(())
    }
}

fn hash_optional(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hash_text(hasher, value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_text(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}
