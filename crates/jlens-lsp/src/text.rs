//! Positions in documents: LSP line/character pairs and byte offsets.

use lsp_types::{Position, Range, TextDocumentContentChangeEvent};

/// How the `character` of a position counts: UTF-16 code units (the LSP default) or bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf16,
    Utf8,
}

/// The start of each line of a text, for converting between positions and byte offsets.
pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
    encoding: Encoding,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str, encoding: Encoding) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex {
            text,
            starts,
            encoding,
        }
    }

    /// The byte offset of a position. A position past the end of its line is the end of the line;
    /// a line past the end of the text is the end of the text; a position inside a character is
    /// the start of the character.
    pub fn offset(&self, position: Position) -> usize {
        let Some(&start) = self.starts.get(position.line as usize) else {
            return self.text.len();
        };
        let line = &self.text[start..self.line_end(position.line as usize)];
        let mut units = 0;
        for (i, c) in line.char_indices() {
            let width = match self.encoding {
                Encoding::Utf16 => c.len_utf16(),
                Encoding::Utf8 => c.len_utf8(),
            };
            if units + width > position.character as usize {
                return start + i;
            }
            units += width;
        }
        start + line.len()
    }

    /// The position of a byte offset (clamped to the text, and to the start of a character).
    pub fn position(&self, offset: usize) -> Position {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let prefix = &self.text[self.starts[line]..offset];
        let character = match self.encoding {
            Encoding::Utf16 => prefix.encode_utf16().count(),
            Encoding::Utf8 => prefix.len(),
        };
        Position::new(line as u32, character as u32)
    }

    pub fn range(&self, span: jlens_core::Span) -> Range {
        Range::new(self.position(span.start), self.position(span.end))
    }

    /// The end of a line's text, before its line break (`\n` or `\r\n`).
    fn line_end(&self, line: usize) -> usize {
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        if end > self.starts[line]
            && self.text.as_bytes()[end - 1] == b'\r'
            && end < self.text.len()
        {
            end - 1
        } else {
            end
        }
    }
}

/// Applies the changes of a `didChange` notification, in order.
pub fn apply_changes(
    text: &mut String,
    changes: Vec<TextDocumentContentChangeEvent>,
    encoding: Encoding,
) {
    for change in changes {
        match change.range {
            None => *text = change.text,
            Some(range) => {
                let index = LineIndex::new(text, encoding);
                let start = index.offset(range.start);
                let end = index.offset(range.end).max(start);
                text.replace_range(start..end, &change.text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_code_units_or_bytes() {
        let text = "a𝄞é\r\nb";
        let utf16 = LineIndex::new(text, Encoding::Utf16);
        assert_eq!(utf16.offset(Position::new(0, 1)), 1);
        assert_eq!(utf16.offset(Position::new(0, 3)), 5); // after 𝄞 (two code units)
        assert_eq!(utf16.offset(Position::new(0, 2)), 1); // inside 𝄞: its start
        assert_eq!(utf16.offset(Position::new(0, 4)), 7);
        assert_eq!(utf16.offset(Position::new(0, 99)), 7); // past the end of the line, before \r
        assert_eq!(utf16.offset(Position::new(1, 0)), 9);
        assert_eq!(utf16.offset(Position::new(5, 0)), text.len());
        assert_eq!(utf16.position(5), Position::new(0, 3));
        assert_eq!(utf16.position(6), Position::new(0, 3)); // inside é
        assert_eq!(utf16.position(10), Position::new(1, 1));
        assert_eq!(utf16.position(99), Position::new(1, 1));

        let utf8 = LineIndex::new(text, Encoding::Utf8);
        assert_eq!(utf8.offset(Position::new(0, 5)), 5);
        assert_eq!(utf8.position(7), Position::new(0, 7));
    }

    #[test]
    fn a_trailing_carriage_return_is_text() {
        let index = LineIndex::new("ab\r", Encoding::Utf16);
        assert_eq!(index.offset(Position::new(0, 3)), 3);
    }

    #[test]
    fn changes_apply_in_order() {
        let mut text = String::from("items.\nprice");
        let change = |range: Option<Range>, text: &str| TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.into(),
        };
        apply_changes(
            &mut text,
            vec![
                change(
                    Some(Range::new(Position::new(0, 6), Position::new(0, 6))),
                    "𝄞",
                ),
                change(
                    Some(Range::new(Position::new(1, 0), Position::new(1, 5))),
                    "tax",
                ),
                // Reversed ranges are treated as empty.
                change(
                    Some(Range::new(Position::new(0, 8), Position::new(0, 6))),
                    "!",
                ),
            ],
            Encoding::Utf16,
        );
        assert_eq!(text, "items.𝄞!\ntax");
        apply_changes(&mut text, vec![change(None, "new")], Encoding::Utf16);
        assert_eq!(text, "new");
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    /// The position of the character at `index` of `chars`, counted the slow way.
    fn position_of(chars: &[char], index: usize, encoding: Encoding) -> Position {
        let before = &chars[..index];
        let line = before.iter().filter(|c| **c == '\n').count();
        let start = before.iter().rposition(|c| *c == '\n').map_or(0, |i| i + 1);
        let character: usize = before[start..]
            .iter()
            .map(|c| match encoding {
                Encoding::Utf16 => c.len_utf16(),
                Encoding::Utf8 => c.len_utf8(),
            })
            .sum();
        Position::new(line as u32, character as u32)
    }

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(&["a", "é", "𝄞", "\n", "\r\n", " ", "$", "."][..]),
            0..30,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        #[test]
        fn edits_at_character_boundaries_splice_the_text(
            start in text(),
            edits in prop::collection::vec((any::<prop::sample::Index>(), any::<prop::sample::Index>(), text()), 1..6),
            utf8 in any::<bool>(),
        ) {
            let encoding = if utf8 { Encoding::Utf8 } else { Encoding::Utf16 };
            let mut text = start.clone();
            let mut model: Vec<char> = start.chars().collect();
            for (a, b, insert) in edits {
                // Positions between a `\r` and its `\n` are not character positions of a line.
                let boundaries: Vec<usize> = (0..=model.len())
                    .filter(|&i| !(i > 0 && model[i - 1] == '\r' && model.get(i) == Some(&'\n')))
                    .collect();
                let (mut from, mut to) = (*a.get(&boundaries), *b.get(&boundaries));
                if from > to {
                    std::mem::swap(&mut from, &mut to);
                }
                let range = Range::new(position_of(&model, from, encoding), position_of(&model, to, encoding));
                apply_changes(
                    &mut text,
                    vec![TextDocumentContentChangeEvent { range: Some(range), range_length: None, text: insert.clone() }],
                    encoding,
                );
                model.splice(from..to, insert.chars());
                prop_assert_eq!(&text, &model.iter().collect::<String>());
            }
        }

        #[test]
        fn any_position_converts_without_panicking(text in text(), line in 0u32..12, character in 0u32..40, offset in 0usize..120) {
            for encoding in [Encoding::Utf8, Encoding::Utf16] {
                let index = LineIndex::new(&text, encoding);
                let byte = index.offset(Position::new(line, character));
                prop_assert!(byte <= text.len() && text.is_char_boundary(byte));
                let position = index.position(offset);
                prop_assert!(index.offset(position) <= text.len());
            }
        }
    }
}
