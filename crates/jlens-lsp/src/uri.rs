//! File system paths of `file:` URIs.

use std::path::PathBuf;

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
        }
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
