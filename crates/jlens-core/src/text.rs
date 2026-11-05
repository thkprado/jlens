//! The strings of an evaluation: a text, or two strings joined by `&` and not yet read.
//!
//! A loop that grows a string with `&`, such as `$reduce(strs, function($a, $b){ $a & $b })`,
//! copied the whole string at each step, so that its time grew with the square of its length. A
//! long result of `&` is now a [`Joined`] node, which holds both sides and makes their text the
//! first time something reads it, once. Nothing appends in place: the left side of `&` in such a
//! loop is never the only reference to its string, since the lambda's frame still holds `$a`
//! while its body reads it.

use std::cell::{OnceCell, RefCell};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

/// Below this length, `&` copies both sides into one text, as a node would cost more than the
/// copy it saves.
const JOIN_MIN: usize = 256;

/// A string value.
#[derive(Clone)]
pub(crate) enum Text {
    /// A text.
    Flat(Rc<str>),
    /// Two strings joined by `&`, made into a text when read.
    Joined(Rc<Joined>),
}

/// Two strings joined by `&`.
pub(crate) struct Joined {
    /// The length of the text in bytes.
    len: usize,
    /// Both sides, until the text is made.
    parts: RefCell<Option<(Text, Text)>>,
    /// The text, once read.
    text: OnceCell<Rc<str>>,
}

impl Text {
    /// The two strings one after the other: `left` and `right` themselves, without a copy, when
    /// the result is long.
    pub(crate) fn join(left: Text, right: Text) -> Text {
        let len = left.len() + right.len();
        if right.is_empty() {
            return left;
        }
        if left.is_empty() {
            return right;
        }
        if len < JOIN_MIN {
            return Text::Flat(concat(&left, &right).into());
        }
        // A short side goes into the short text next to it, which a loop that adds short pieces
        // to one side meets at each step, so that it keeps a node for each `JOIN_MIN` bytes
        // instead of one for each piece: (a & b) & c is a & (b & c).
        if right.len() < JOIN_MIN
            && let Text::Joined(j) = &left
            && let Some((a, Text::Flat(b))) = &*j.parts.borrow()
            && b.len() + right.len() < JOIN_MIN
        {
            return Joined::node(len, a.clone(), Text::Flat(concat(b, &right).into()));
        }
        if left.len() < JOIN_MIN
            && let Text::Joined(j) = &right
            && let Some((Text::Flat(c), d)) = &*j.parts.borrow()
            && left.len() + c.len() < JOIN_MIN
        {
            return Joined::node(len, Text::Flat(concat(&left, c).into()), d.clone());
        }
        Joined::node(len, left, right)
    }

    /// The length in bytes, without making the text.
    pub(crate) fn len(&self) -> usize {
        match self {
            Text::Flat(s) => s.len(),
            Text::Joined(j) => j.len,
        }
    }

    /// Whether the string is empty, without making the text.
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The text, as a shared string of its own.
    pub(crate) fn into_rc(self) -> Rc<str> {
        match self {
            Text::Flat(s) => s,
            Text::Joined(j) => j.text().clone(),
        }
    }

    /// Whether both are the same text in memory.
    pub(crate) fn ptr_eq(&self, other: &Text) -> bool {
        match (self, other) {
            (Text::Flat(a), Text::Flat(b)) => Rc::ptr_eq(a, b),
            (Text::Joined(a), Text::Joined(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

fn concat(left: &str, right: &str) -> String {
    let mut text = String::with_capacity(left.len() + right.len());
    text.push_str(left);
    text.push_str(right);
    text
}

impl Joined {
    fn node(len: usize, left: Text, right: Text) -> Text {
        Text::Joined(Rc::new(Joined {
            len,
            parts: RefCell::new(Some((left, right))),
            text: OnceCell::new(),
        }))
    }

    #[inline(never)]
    fn text(&self) -> &Rc<str> {
        if let Some(text) = self.text.get() {
            return text;
        }
        let mut out = String::with_capacity(self.len);
        // Joined strings nest as deep as the loop that made them ran, so the walk keeps its own
        // stack, right side below the left.
        let mut stack: Vec<Text> = Vec::new();
        if let Some((left, right)) = &*self.parts.borrow() {
            stack.push(right.clone());
            stack.push(left.clone());
        }
        while let Some(part) = stack.pop() {
            match part {
                Text::Flat(s) => out.push_str(&s),
                Text::Joined(j) => {
                    if let Some(text) = j.text.get() {
                        out.push_str(text);
                    } else if let Some((left, right)) = &*j.parts.borrow() {
                        stack.push(right.clone());
                        stack.push(left.clone());
                    }
                }
            }
        }
        // The parts are no longer needed: a loop that reads each string it grows would
        // otherwise keep every one of them.
        drop(self.parts.borrow_mut().take());
        self.text.get_or_init(|| out.into())
    }
}

impl Drop for Joined {
    /// Without a recursion as deep as the nodes, which a long loop would make overflow the stack.
    #[inline(never)]
    fn drop(&mut self) {
        let Some((left, right)) = self.parts.get_mut().take() else {
            return;
        };
        let mut stack = vec![left, right];
        while let Some(part) = stack.pop() {
            if let Text::Joined(j) = part
                && let Ok(mut j) = Rc::try_unwrap(j)
                && let Some((left, right)) = j.parts.get_mut().take()
            {
                stack.push(left);
                stack.push(right);
            }
        }
    }
}

impl Deref for Text {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        match self {
            Text::Flat(s) => s,
            Text::Joined(j) => j.text(),
        }
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self
    }
}

impl PartialEq for Text {
    // With `#[inline]` it stayed a call where `$distinct` compares strings: 1.75 ms, not 0.81,
    // for the 100,000 strings of `bytes_eq`.
    #[inline(always)]
    fn eq(&self, other: &Text) -> bool {
        match (self, other) {
            (Text::Flat(a), Text::Flat(b)) => bytes_eq(a.as_bytes(), b.as_bytes()),
            _ => {
                self.len() == other.len()
                    && (self.ptr_eq(other) || bytes_eq(self.as_bytes(), other.as_bytes()))
            }
        }
    }
}

/// Up to this length, two strings are compared byte by byte in place.
const SHORT: usize = 16;

/// Whether two strings hold the same bytes. `==` on slices calls `memcmp`, which costs more than
/// the comparison itself on short strings: `$distinct` over 100,000 strings of two bytes, eight
/// values, took 2.73 ms through it against a `Document`, and takes 0.81 ms with short strings
/// compared here.
#[inline]
fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && if a.len() <= SHORT {
            a.iter().zip(b).all(|(x, y)| x == y)
        } else {
            a == b
        }
}

impl Eq for Text {}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl From<Rc<str>> for Text {
    fn from(s: Rc<str>) -> Text {
        Text::Flat(s)
    }
}

impl From<&Rc<str>> for Text {
    fn from(s: &Rc<str>) -> Text {
        Text::Flat(s.clone())
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Text {
        Text::Flat(s.into())
    }
}

impl From<String> for Text {
    fn from(s: String) -> Text {
        Text::Flat(s.into())
    }
}

impl From<&String> for Text {
    fn from(s: &String) -> Text {
        Text::Flat(s.as_str().into())
    }
}

impl From<Box<str>> for Text {
    fn from(s: Box<str>) -> Text {
        Text::Flat(s.into())
    }
}

impl From<std::borrow::Cow<'_, str>> for Text {
    fn from(s: std::borrow::Cow<'_, str>) -> Text {
        Text::Flat(s.into())
    }
}

impl Default for Text {
    fn default() -> Text {
        Text::Flat(Rc::default())
    }
}

impl From<char> for Text {
    fn from(c: char) -> Text {
        Text::Flat(c.to_string().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(s: &str) -> Text {
        Text::from(s)
    }

    #[test]
    fn a_string_stays_two_words() {
        assert_eq!(size_of::<Text>(), size_of::<Rc<str>>());
        assert_eq!(size_of::<crate::value::Value<'_>>(), 3 * size_of::<usize>());
    }

    #[test]
    fn joined_strings_read_as_their_text() {
        let long = "é".repeat(200);
        let joined = Text::join(flat(&long), flat("ab"));
        assert!(matches!(joined, Text::Joined(_)));
        assert_eq!(joined.len(), 402);
        assert_eq!(&*joined, format!("{long}ab"));
        let short = Text::join(flat("ab"), flat("cd"));
        assert!(matches!(short, Text::Flat(_)));
        assert_eq!(&*short, "abcd");
        assert!(Text::join(flat(""), joined.clone()).ptr_eq(&joined));
        assert!(Text::join(joined.clone(), flat("")).ptr_eq(&joined));
    }

    #[test]
    fn strings_equal_by_their_bytes_on_both_sides_of_short() {
        for len in [0, 1, SHORT - 1, SHORT, SHORT + 1, 300] {
            let s = "a".repeat(len);
            assert_eq!(flat(&s), flat(&s.clone()), "{len}");
            for at in [0, len / 2, len.saturating_sub(1)]
                .into_iter()
                .filter(|_| len > 0)
            {
                let mut other = s.clone().into_bytes();
                other[at] = b'b';
                let other = String::from_utf8(other).unwrap();
                assert_ne!(flat(&s), flat(&other), "{len} at {at}");
            }
            assert_ne!(flat(&s), flat(&format!("{s}a")), "{len}");
        }
        let long = "x".repeat(JOIN_MIN);
        let joined = Text::join(flat(&long), flat("ab"));
        assert_eq!(joined, flat(&format!("{long}ab")));
        assert_eq!(flat(&format!("{long}ab")), joined);
        assert_ne!(joined, flat(&format!("{long}ac")));
    }

    /// The nodes of a string, and the length of its longest flat part.
    fn nodes(text: &Text) -> (usize, usize) {
        let (mut nodes, mut longest) = (0, 0);
        let mut stack = vec![text.clone()];
        while let Some(part) = stack.pop() {
            match part {
                Text::Flat(s) => longest = longest.max(s.len()),
                Text::Joined(j) => {
                    nodes += 1;
                    if let Some((left, right)) = &*j.parts.borrow() {
                        stack.push(left.clone());
                        stack.push(right.clone());
                    }
                }
            }
        }
        (nodes, longest)
    }

    #[test]
    fn short_pieces_take_a_node_for_each_few_hundred_bytes() {
        let start = Text::from("x".repeat(JOIN_MIN));
        let (mut left, mut right) = (start.clone(), start.clone());
        let mut model = String::new();
        for i in 0..10_000 {
            let piece = Text::from(["a", "é", "😀"][i % 3]);
            model.push_str(&piece);
            left = Text::join(left, piece.clone());
            right = Text::join(piece, right);
        }
        for text in [&left, &right] {
            let (nodes, longest) = nodes(text);
            assert!(nodes <= 2 * model.len() / JOIN_MIN + 2, "{nodes}");
            assert!(longest <= JOIN_MIN);
        }
        assert_eq!(&*left, format!("{start}{model}"));
        let reversed: String = model.chars().rev().collect();
        assert_eq!(&*right, format!("{reversed}{start}"));
    }

    #[test]
    fn a_deep_chain_is_read_and_dropped_without_recursion() {
        // Pieces too long to fold, both ways round, and read halfway, which drops the parts
        // read.
        let piece = Text::from("x".repeat(JOIN_MIN));
        let mark = Text::from("y".repeat(JOIN_MIN));
        let mut left = piece.clone();
        let mut right = piece.clone();
        let mut middle = None;
        for i in 0..100_000 {
            let piece = if i % 10 == 0 { &mark } else { &piece };
            left = Text::join(left, piece.clone());
            right = Text::join(piece.clone(), right);
            if i == 50_000 {
                assert_eq!(left.len(), JOIN_MIN * 50_002);
                assert!(left.ends_with('y'));
                middle = Some(left.clone());
            }
        }
        assert_eq!(nodes(&left).0, 100_000 - 50_000);
        assert_eq!(nodes(&right).0, 100_000);
        assert_eq!(left.len(), JOIN_MIN * 100_001);
        assert_eq!(left.len(), right.len());
        assert!(left[JOIN_MIN * 99_991..].starts_with('y'));
        assert!(right[..JOIN_MIN * 10].ends_with('y'));
        assert!(middle.unwrap().ends_with('y'));
    }
}
