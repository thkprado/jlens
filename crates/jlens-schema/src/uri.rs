//! URI references, as far as a JSON Schema's `$id` and `$ref` need them: a reference resolved
//! against a base (RFC 3986, section 5.2), and the fragment read as a JSON Pointer or an anchor.
//!
//! Two URIs name the same document when their texts are equal once resolved: nothing is
//! normalized but the dot segments of a path. A base may be relative, or empty, as that of a
//! document given with no URI is; a reference is then resolved against its path alone.

/// The components of a URI reference.
struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn parse(text: &str) -> Parts<'_> {
    let (text, fragment) = match text.split_once('#') {
        Some((text, fragment)) => (text, Some(fragment)),
        None => (text, None),
    };
    let (text, query) = match text.split_once('?') {
        Some((text, query)) => (text, Some(query)),
        None => (text, None),
    };
    let (scheme, text) = match text.split_once(':') {
        Some((scheme, rest))
            if scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            (Some(scheme), rest)
        }
        _ => (None, text),
    };
    let (authority, path) = match text.strip_prefix("//") {
        Some(rest) => {
            let end = rest.find('/').unwrap_or(rest.len());
            (Some(&rest[..end]), &rest[end..])
        }
        None => (None, text),
    };
    Parts {
        scheme,
        authority,
        path,
        query,
        fragment,
    }
}

/// A path without its `.` and `..` segments (RFC 3986, section 5.2.4).
fn remove_dot_segments(path: &str) -> String {
    let mut input = path;
    let mut out = String::with_capacity(path.len());
    while !input.is_empty() {
        if let Some(rest) = input
            .strip_prefix("../")
            .or_else(|| input.strip_prefix("./"))
        {
            input = rest;
        } else if input.starts_with("/./") {
            input = &input[2..];
        } else if input == "/." {
            input = "/";
        } else if input.starts_with("/../") || input == "/.." {
            input = if input == "/.." { "/" } else { &input[3..] };
            out.truncate(out.rfind('/').unwrap_or(0));
        } else if input == "." || input == ".." {
            input = "";
        } else {
            // The first segment, with its leading slash if it has one.
            let start = usize::from(input.starts_with('/'));
            let end = input[start..].find('/').map_or(input.len(), |i| i + start);
            out.push_str(&input[..end]);
            input = &input[end..];
        }
    }
    out
}

/// `reference` resolved against `base`, with the fragment of the reference.
pub(crate) fn resolve(base: &str, reference: &str) -> String {
    let r = parse(reference);
    let b = parse(base);
    let (scheme, authority, path, query);
    if r.scheme.is_some() {
        (scheme, authority, query) = (r.scheme, r.authority, r.query);
        path = remove_dot_segments(r.path);
    } else {
        scheme = b.scheme;
        if r.authority.is_some() {
            (authority, query) = (r.authority, r.query);
            path = remove_dot_segments(r.path);
        } else {
            authority = b.authority;
            if r.path.is_empty() {
                path = b.path.to_owned();
                query = r.query.or(b.query);
            } else {
                query = r.query;
                path = if r.path.starts_with('/') {
                    remove_dot_segments(r.path)
                } else if b.authority.is_some() && b.path.is_empty() {
                    remove_dot_segments(&format!("/{}", r.path))
                } else {
                    let folder = b.path.rfind('/').map_or("", |i| &b.path[..=i]);
                    let merged = remove_dot_segments(&format!("{folder}{}", r.path));
                    // A relative base stays relative when `..` empties its folders.
                    match merged.strip_prefix('/') {
                        Some(relative) if !folder.starts_with('/') => relative.to_owned(),
                        _ => merged,
                    }
                };
            }
        }
    }
    let mut out = String::new();
    if let Some(scheme) = scheme {
        out.push_str(scheme);
        out.push(':');
    }
    if let Some(authority) = authority {
        out.push_str("//");
        out.push_str(authority);
    }
    out.push_str(&path);
    if let Some(query) = query {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = r.fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// A URI without its fragment, and the fragment (empty when there is none).
pub(crate) fn split_fragment(uri: &str) -> (&str, &str) {
    uri.split_once('#').unwrap_or((uri, ""))
}

/// `text` with its `%XX` escapes read; `None` when what they give is not UTF-8.
pub(crate) fn percent_decode(text: &str) -> Option<String> {
    if !text.contains('%') {
        return Some(text.to_owned());
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(&high), Some(&low)) if hex(high).is_some() && hex(low).is_some() => {
                out.push((hex(high)? * 16 + hex(low)?) as u8);
                i += 3;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_as_rfc_3986_does() {
        // The examples of section 5.4, against one base.
        let base = "http://a/b/c/d;p?q";
        for (reference, expected) in [
            ("g:h", "g:h"),
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q#s"),
            ("g#s", "http://a/b/c/g#s"),
            ("g?y#s", "http://a/b/c/g?y#s"),
            (";x", "http://a/b/c/;x"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../", "http://a/"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("/../g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            ("..g", "http://a/b/c/..g"),
            ("./../g", "http://a/b/g"),
            ("./g/.", "http://a/b/c/g/"),
            ("g/./h", "http://a/b/c/g/h"),
            ("g/../h", "http://a/b/c/h"),
            ("g;x=1/../y", "http://a/b/c/y"),
            ("g?y/./x", "http://a/b/c/g?y/./x"),
            ("g#s/../x", "http://a/b/c/g#s/../x"),
            ("http:g", "http:g"),
        ] {
            assert_eq!(resolve(base, reference), expected, "{reference}");
        }
    }

    #[test]
    fn resolves_against_a_base_that_is_relative_or_empty() {
        assert_eq!(resolve("", "#/$defs/a"), "#/$defs/a");
        assert_eq!(resolve("", "common.json#/$defs/a"), "common.json#/$defs/a");
        assert_eq!(
            resolve("schemas/order.json", "common.json"),
            "schemas/common.json"
        );
        assert_eq!(
            resolve("schemas/order.json", "../common.json"),
            "common.json"
        );
        assert_eq!(resolve("schemas/order.json", "#x"), "schemas/order.json#x");
        assert_eq!(resolve("http://a", "g"), "http://a/g");
        assert_eq!(resolve("urn:example:order", "#/a"), "urn:example:order#/a");
        assert_eq!(
            resolve("urn:example:order", "urn:example:common"),
            "urn:example:common"
        );
    }

    #[test]
    fn reads_escapes() {
        assert_eq!(percent_decode("/a%20b/%C3%A9").as_deref(), Some("/a b/é"));
        assert_eq!(percent_decode("100%").as_deref(), Some("100%"));
        assert_eq!(percent_decode("%zz").as_deref(), Some("%zz"));
        assert_eq!(percent_decode("%FF"), None);
    }
}
