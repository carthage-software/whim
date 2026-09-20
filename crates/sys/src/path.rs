//! Lossless conversion between platform paths and Whim byte strings.

#![deny(clippy::nursery, clippy::pedantic)]

use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStringExt;
#[cfg(windows)]
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
#[cfg(windows)]
use std::str::from_utf8;

/// Returns the platform path's raw bytes.
#[must_use]
pub fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        let mut bytes = Vec::new();
        for character in char::decode_utf16(path.as_os_str().encode_wide()) {
            match character {
                Ok(character) => {
                    bytes.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
                Err(error) => {
                    let surrogate = error.unpaired_surrogate();
                    bytes.extend_from_slice(&[
                        0xe0 | (surrogate >> 12) as u8,
                        0x80 | ((surrogate >> 6) & 0x3f) as u8,
                        0x80 | (surrogate & 0x3f) as u8,
                    ]);
                }
            }
        }
        bytes
    }
}

/// Decodes platform path bytes, including unpaired Windows surrogates encoded as WTF-8.
///
/// # Errors
/// Returns an error for byte sequences that cannot represent a Windows path.
pub fn path_from_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    let path = PathBuf::from(decode_os_string(bytes, true)?);
    #[cfg(windows)]
    {
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next()
            && prefix.kind().is_verbatim()
        {
            let mut normalized = PathBuf::from(prefix.as_os_str());
            normalized.push(components.as_path());
            return Ok(normalized);
        }
    }

    Ok(path)
}

/// Decodes an OS string without changing path separators.
///
/// # Errors
/// Returns an error when Windows cannot represent the supplied bytes.
pub fn os_string_from_bytes(bytes: &[u8]) -> io::Result<OsString> {
    decode_os_string(bytes, false)
}

#[cfg_attr(
    unix,
    expect(
        clippy::unnecessary_wraps,
        reason = "Windows rejects invalid path encodings"
    )
)]
fn decode_os_string(bytes: &[u8], normalize_separators: bool) -> io::Result<OsString> {
    #[cfg(unix)]
    {
        let _ = normalize_separators;
        Ok(OsString::from_vec(bytes.to_vec()))
    }
    #[cfg(windows)]
    {
        let mut remaining = bytes;
        let mut wide = Vec::new();
        while !remaining.is_empty() {
            match from_utf8(remaining) {
                Ok(text) => {
                    wide.extend(text.encode_utf16());
                    break;
                }
                Err(error) => {
                    let (valid, rest) = remaining.split_at(error.valid_up_to());
                    wide.extend(
                        from_utf8(valid)
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
                            .encode_utf16(),
                    );
                    match rest {
                        [0xed, middle @ 0xa0..=0xbf, last @ 0x80..=0xbf, tail @ ..] => {
                            wide.push(
                                0xd000 | (u16::from(*middle & 0x3f) << 6) | u16::from(*last & 0x3f),
                            );
                            remaining = tail;
                        }
                        _ => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "a Windows path must use UTF-8 or WTF-8",
                            ));
                        }
                    }
                }
            }
        }
        if normalize_separators {
            for unit in &mut wide {
                if *unit == u16::from(b'/') {
                    *unit = u16::from(b'\\');
                }
            }
        }
        Ok(OsString::from_wide(&wide))
    }
}
