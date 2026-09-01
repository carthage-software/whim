use std::fmt;

use thiserror::Error as ThisError;
use url::ParseError as UrlParseError;
use url::Url;

#[derive(Debug, ThisError)]
pub(crate) enum Error {
    #[error("`path:` dependencies are not supported; use an absolute `git+file://` Git source")]
    PathDependency,
    #[error("only HTTPS, SSH, and explicit `git+file://` Git repositories are supported")]
    UnsupportedTransport,
    #[error("external Git transport helpers are not supported")]
    ExternalTransportHelper,
    #[error("invalid SCP-style SSH repository")]
    InvalidScp,
    #[error("invalid Git source URL: {0}")]
    InvalidUrl(#[source] UrlParseError),
    #[error("a Git source URL may not contain credentials")]
    Credentials,
    #[error("a Git source URL may not contain a query or fragment")]
    QueryOrFragment,
    #[error("local Git source URLs may not contain a host")]
    LocalHost,
    #[error("local Git source URLs must contain an absolute path")]
    RelativeLocalPath,
    #[error("a Git source URL must contain a host")]
    MissingHost,
    #[error("invalid Git source host: {0}")]
    InvalidHost(#[source] UrlParseError),
    #[error("invalid Git source port")]
    InvalidPort,
    #[error("a Git source path may not contain `.` or `..`")]
    RelativePathSegment,
    #[error("a Git source URL must name a repository")]
    MissingRepository,
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct Source {
    identity: String,
    fetch: String,
}

impl Source {
    pub(crate) fn parse(value: &str) -> Result<Self, Error> {
        if value.starts_with("path:") {
            return Err(Error::PathDependency);
        }

        if let Some(value) = value.strip_prefix("git+") {
            return Self::from_url(value);
        }

        if value.starts_with("https://") || value.starts_with("ssh://") {
            return Self::from_url(value);
        }

        if value.contains("::") {
            return Err(Error::ExternalTransportHelper);
        }

        Self::from_scp(value)
    }

    fn from_scp(value: &str) -> Result<Self, Error> {
        if value.contains("://") || value.starts_with('/') || value.starts_with('.') {
            return Err(Error::UnsupportedTransport);
        }

        let Some((authority, path)) = value.split_once(':') else {
            return Err(Error::UnsupportedTransport);
        };

        if authority.is_empty() || path.is_empty() || authority.contains('/') {
            return Err(Error::InvalidScp);
        }

        let url = format!("ssh://{authority}/{path}");
        Self::from_url(&url)
    }

    fn from_url(value: &str) -> Result<Self, Error> {
        if raw_path(value).split('/').any(relative_segment) {
            return Err(Error::RelativePathSegment);
        }

        let mut url = Url::parse(value).map_err(Error::InvalidUrl)?;
        match url.scheme() {
            "https" | "ssh" | "file" => {}
            _ => {
                return Err(Error::UnsupportedTransport);
            }
        }

        if url.password().is_some() || (url.scheme() != "ssh" && !url.username().is_empty()) {
            return Err(Error::Credentials);
        }

        if url.query().is_some() || url.fragment().is_some() {
            return Err(Error::QueryOrFragment);
        }

        if url.scheme() == "file" {
            if url.host_str().is_some() {
                return Err(Error::LocalHost);
            }

            if !url.path().starts_with('/') {
                return Err(Error::RelativeLocalPath);
            }
        } else {
            let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
                return Err(Error::MissingHost);
            };

            url.set_host(Some(&host)).map_err(Error::InvalidHost)?;
        }

        let default_port = match url.scheme() {
            "https" => Some(443),
            "ssh" => Some(22),
            _ => None,
        };

        if default_port.is_some() && url.port() == default_port {
            url.set_port(None).map_err(|()| Error::InvalidPort)?;
        }

        let mut components = Vec::new();
        for component in url.path().split('/') {
            if component.is_empty() {
                continue;
            }

            if component == "." || component == ".." {
                return Err(Error::RelativePathSegment);
            }

            components.push(component);
        }

        let Some(last) = components.last_mut() else {
            return Err(Error::MissingRepository);
        };
        if url.scheme() != "file"
            && let Some(stripped) = last.strip_suffix(".git")
        {
            if stripped.is_empty() {
                return Err(Error::MissingRepository);
            }
            *last = stripped;
        }

        let path = format!("/{}", components.join("/"));
        url.set_path(&path);
        let fetch = url.to_string().trim_end_matches('/').to_owned();
        let identity = format!("git+{fetch}");

        Ok(Self { identity, fetch })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn fetch(&self) -> &str {
        &self.fetch
    }

    pub(crate) fn digest(&self) -> String {
        blake3::hash(self.identity.as_bytes()).to_hex().to_string()
    }
}

fn raw_path(value: &str) -> &str {
    let Some((_, remainder)) = value.split_once("://") else {
        return "";
    };
    let Some(start) = remainder.find('/') else {
        return "";
    };
    let path = &remainder[start..];
    path.find(['?', '#']).map_or(path, |end| &path[..end])
}

fn relative_segment(segment: &str) -> bool {
    let mut bytes = segment.as_bytes();
    let mut dots = 0_usize;
    while !bytes.is_empty() {
        if bytes[0] == b'.' {
            bytes = &bytes[1..];
        } else if bytes.len() >= 3
            && bytes[0] == b'%'
            && bytes[1] == b'2'
            && matches!(bytes[2], b'e' | b'E')
        {
            bytes = &bytes[3..];
        } else {
            return false;
        }
        dots += 1;
    }

    matches!(dots, 1 | 2)
}

impl fmt::Debug for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Source")
            .field(&self.identity)
            .finish()
    }
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.identity)
    }
}

#[cfg(test)]
mod tests {
    use crate::package::source::Error;
    use crate::package::source::Source;

    #[test]
    fn equivalent_https_spellings_have_one_identity() {
        let first =
            Source::parse("https://GitHub.com/acme/library.git/").expect("the source is valid");
        let second =
            Source::parse("git+https://github.com/acme/library").expect("the source is valid");

        assert_eq!(first, second);
        assert_eq!(first.identity(), "git+https://github.com/acme/library");
    }

    #[test]
    fn scp_ssh_is_normalized() {
        let source = Source::parse("git@github.com:acme/library.git").expect("the source is valid");

        assert_eq!(source.identity(), "git+ssh://git@github.com/acme/library");
    }

    #[test]
    fn credentials_and_implicit_paths_are_rejected() {
        assert!(Source::parse("https://token@github.com/acme/library").is_err());
        assert!(Source::parse("ssh://git:secret@github.com/acme/library").is_err());
        assert!(Source::parse("https://github.com/acme/library?token=secret").is_err());
        assert!(Source::parse("http://github.com/acme/library").is_err());
        assert!(Source::parse("git://github.com/acme/library").is_err());
        assert!(Source::parse("ext::command").is_err());
        assert!(Source::parse("path:../library").is_err());
        assert!(Source::parse("file:///tmp/library").is_err());
        assert!(Source::parse("git+file://host/tmp/library").is_err());
    }

    #[test]
    fn relative_segments_are_rejected_before_url_normalization() {
        for source in [
            "https://example.com/acme/../library",
            "ssh://git@example.com/acme/./library",
            "git@example.com:acme/%2e%2e/library",
            "git+file:///tmp/acme/.%2E/library",
        ] {
            assert!(
                matches!(Source::parse(source), Err(Error::RelativePathSegment)),
                "accepted {source}",
            );
        }
    }

    #[test]
    fn explicit_absolute_file_sources_are_supported() {
        let source = Source::parse("git+file:///tmp//Acme///Library.git/")
            .expect("the local Git source is valid");

        assert_eq!(source.identity(), "git+file:///tmp/Acme/Library.git");
        assert_eq!(source.fetch(), "file:///tmp/Acme/Library.git");
    }

    #[test]
    fn default_ports_and_redundant_separators_are_removed() {
        let source = Source::parse("https://GitHub.com:443//Acme///Library.git//")
            .expect("the source is valid");
        assert_eq!(source.identity(), "git+https://github.com/Acme/Library");
    }

    #[test]
    fn https_and_ssh_are_distinct() {
        let https =
            Source::parse("https://github.com/acme/library").expect("the HTTPS source is valid");
        let ssh =
            Source::parse("ssh://git@github.com:22/acme/library").expect("the SSH source is valid");
        assert_ne!(https, ssh);
        assert_eq!(ssh.identity(), "git+ssh://git@github.com/acme/library");
    }
}
