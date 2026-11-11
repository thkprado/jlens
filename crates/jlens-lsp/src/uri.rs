//! File system paths of `file:` URIs, and the URIs of paths.

use std::path::{Component, Path, PathBuf, Prefix};

/// The `file:` URI of an absolute path, as [`path_of`] reads it back: `None` for a relative
/// path, one that is not Unicode, or a Windows path that is neither on a drive nor on a share.
pub fn uri_of(path: &Path) -> Option<String> {
    let mut uri = String::from("file://");
    let mut rooted = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(drive) => {
                    uri.push('/');
                    uri.push(char::from(drive));
                    uri.push(':');
                }
                Prefix::UNC(server, share) => {
                    encode(server.to_str()?, &mut uri);
                    uri.push('/');
                    encode(share.to_str()?, &mut uri);
                }
                _ => return None,
            },
            Component::RootDir => rooted = true,
            Component::Normal(name) => {
                uri.push('/');
                encode(name.to_str()?, &mut uri);
            }
            Component::CurDir | Component::ParentDir => return None,
        }
    }
    if !rooted {
        return None;
    }
    if uri == "file://" {
        uri.push('/');
    }
    Some(uri)
}

/// Percent-encodes what is not an unreserved character of a URI.
fn encode(text: &str, out: &mut String) {
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
}

/// The path of a `file:` URI: `file:///home/a/b.jsonata`, `file:///c%3A/a/b.jsonata`, or a UNC
/// path such as `file://server/share/b.jsonata` on Windows. Other URIs (`untitled:`) have none.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = &uri[5..];
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let (host, path) = match rest.strip_prefix("//") {
        Some(authority) => match authority.find('/') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => (authority, "/"),
        },
        None => ("", rest),
    };
    let host = if host.eq_ignore_ascii_case("localhost") {
        ""
    } else {
        host
    };
    to_path(&decode(host)?, &decode(path)?)
}

/// Percent-decodes, failing on malformed escapes and on text that is not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(windows)]
fn to_path(host: &str, path: &str) -> Option<PathBuf> {
    let path = path.replace('/', "\\");
    if !host.is_empty() {
        return Some(PathBuf::from(format!(r"\\{host}{path}")));
    }
    // `\c:\a` is `c:\a`.
    let bytes = path.as_bytes();
    match bytes {
        [b'\\', drive, b':', ..] if drive.is_ascii_alphabetic() => Some(PathBuf::from(&path[1..])),
        _ => None,
    }
}

#[cfg(not(windows))]
fn to_path(host: &str, path: &str) -> Option<PathBuf> {
    (host.is_empty() && path.starts_with('/')).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_schemes_and_malformed_uris_have_no_path() {
        assert_eq!(path_of("untitled:Untitled-1"), None);
        assert_eq!(path_of("http://example.com/a"), None);
        assert_eq!(path_of("file"), None);
        assert_eq!(path_of("file:///a%2"), None);
        assert_eq!(path_of("file:///a%zz"), None);
        assert_eq!(path_of("file:///%ff"), None, "not UTF-8");
    }

    /// The paths are those that the `url` crate encodes in its URIs.
    #[test]
    fn paths_agree_with_the_url_crate() {
        let paths: &[&str] = if cfg!(windows) {
            &[
                r"C:\work\rules\a.jsonata",
                r"d:\a b\é 𝄞.jsonata",
                r"\\server\share\x.jsonata",
            ]
        } else {
            &["/work/rules/a.jsonata", "/a b/é 𝄞#?%.jsonata", "/"]
        };
        for path in paths {
            let uri = url::Url::from_file_path(path).unwrap();
            assert_eq!(
                path_of(uri.as_str()).as_deref(),
                Some(std::path::Path::new(path)),
                "{uri}"
            );
            // The URI of a path reads back as the path, and resolves as the `url` crate's.
            let own = uri_of(Path::new(path)).expect(path);
            assert_eq!(path_of(&own).as_deref(), Some(Path::new(path)), "{own}");
            assert_eq!(
                url::Url::parse(&own)
                    .unwrap()
                    .to_file_path()
                    .ok()
                    .as_deref(),
                Some(Path::new(path)),
                "{own}"
            );
        }
        assert_eq!(uri_of(Path::new("rules/a.jsonata")), None);
        assert_eq!(uri_of(Path::new("")), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_uris_as_editors_send_them() {
        // VS Code encodes the drive's colon and lowercases the letter.
        assert_eq!(
            path_of("file:///d%3A/work/a.jsonata"),
            Some(PathBuf::from(r"d:\work\a.jsonata"))
        );
        assert_eq!(
            path_of("file://localhost/C:/a"),
            Some(PathBuf::from(r"C:\a"))
        );
        assert_eq!(path_of("file:///work/a"), None, "no drive");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_uris_as_editors_send_them() {
        assert_eq!(
            path_of("file://localhost/home/a"),
            Some(PathBuf::from("/home/a"))
        );
        assert_eq!(
            path_of("file:///home/a?query#fragment"),
            Some(PathBuf::from("/home/a"))
        );
        assert_eq!(path_of("file://server/share/a"), None, "a remote host");
    }
}
