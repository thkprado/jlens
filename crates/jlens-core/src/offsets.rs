//! Conversion between byte offsets and UTF-16 offsets.
//!
//! The Rust crates count offsets and spans in bytes of the UTF-8 text. JavaScript strings, browser
//! editors and the `position` of an [`Error`](crate::Error) count UTF-16 code units. [`Offsets`]
//! converts between the two for one text.

/// Converts offsets into one text between UTF-8 bytes, the unit of the Rust crates, and UTF-16
/// code units, the unit of JavaScript strings and browser editors.
///
/// Build it once per text, then convert as many offsets as needed, in either direction:
///
/// ```
/// use jlens_core::Offsets;
///
/// // `é` takes 2 bytes and 1 UTF-16 unit; `𝄞` takes 4 bytes and 2 units (a surrogate pair).
/// let offsets = Offsets::new("é𝄞x");
/// assert_eq!(offsets.to_utf16(6), 3); // the byte offset of `x`
/// assert_eq!(offsets.to_byte(3), 6);
/// ```
///
/// Each direction maps an offset that falls inside a character to the start of that character
/// (a byte inside a multi-byte character, or a unit between the halves of a surrogate pair), and
/// an offset past the end of the text to the end. For ASCII text, both units are the same and no
/// table is built.
#[derive(Clone, Debug)]
pub struct Offsets {
    table: Table,
    len: usize,
}

#[derive(Clone, Debug)]
enum Table {
    /// ASCII text: a byte offset is a UTF-16 offset.
    Ascii,
    /// The UTF-16 offset of every byte offset (`len + 1` entries). Bytes inside a character map to
    /// the offset of the character.
    Units(Vec<u32>),
    /// Text too long for `u32` offsets: each conversion walks the text.
    Long(Box<str>),
}

impl Offsets {
    /// Prepares the conversion of offsets into `text`. Outside ASCII, this builds a table of
    /// four bytes per byte of text, so each conversion takes constant or logarithmic time.
    pub fn new(text: &str) -> Self {
        let table = if text.is_ascii() {
            Table::Ascii
        } else if u32::try_from(text.len()).is_ok() {
            // A text has no more UTF-16 units than bytes, so every offset fits in `u32`.
            let mut table = Vec::with_capacity(text.len() + 1);
            let mut units = 0u32;
            for c in text.chars() {
                table.extend(std::iter::repeat_n(units, c.len_utf8()));
                units += c.len_utf16() as u32;
            }
            table.push(units);
            Table::Units(table)
        } else {
            Table::Long(text.into())
        };
        Offsets {
            table,
            len: text.len(),
        }
    }

    /// Whether the text is ASCII, where a byte offset and a UTF-16 offset are the same number.
    pub fn is_ascii(&self) -> bool {
        matches!(self.table, Table::Ascii)
    }

    /// The UTF-16 offset of a byte offset.
    pub fn to_utf16(&self, byte: usize) -> usize {
        let byte = byte.min(self.len);
        match &self.table {
            Table::Ascii => byte,
            Table::Units(table) => table[byte] as usize,
            Table::Long(text) => {
                let mut end = byte;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text[..end].chars().map(char::len_utf16).sum()
            }
        }
    }

    /// The byte offset of a UTF-16 offset.
    pub fn to_byte(&self, unit: usize) -> usize {
        match &self.table {
            Table::Ascii => unit.min(self.len),
            Table::Units(table) => {
                // The first byte whose UTF-16 offset reaches `unit`.
                let mut byte = table
                    .partition_point(|&u| (u as usize) < unit)
                    .min(self.len);
                if table[byte] as usize > unit {
                    // `unit` falls inside a surrogate pair: go back to the character's start.
                    byte -= 1;
                    while byte > 0 && table[byte - 1] == table[byte] {
                        byte -= 1;
                    }
                }
                byte
            }
            Table::Long(text) => {
                let mut units = 0;
                for (byte, c) in text.char_indices() {
                    units += c.len_utf16();
                    if units > unit {
                        return byte;
                    }
                }
                self.len
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Offsets, Table};

    /// Bytes: a=0, é=1..3, 𝄞=3..7, b=7, end=8. UTF-16: a=0, é=1, 𝄞=2..4, b=4, end=5.
    const TEXT: &str = "aé𝄞b";

    #[test]
    fn ascii_offsets_are_the_same() {
        let o = Offsets::new("abc");
        assert!(o.is_ascii());
        assert_eq!([0, 1, 3].map(|b| o.to_utf16(b)), [0, 1, 3]);
        assert_eq!([0, 1, 3].map(|u| o.to_byte(u)), [0, 1, 3]);
        let empty = Offsets::new("");
        assert_eq!((empty.to_utf16(0), empty.to_byte(0)), (0, 0));
    }

    #[test]
    fn offsets_outside_ascii_convert_both_ways() {
        let o = Offsets::new(TEXT);
        assert!(!o.is_ascii());
        assert_eq!([0, 1, 3, 7, 8].map(|b| o.to_utf16(b)), [0, 1, 2, 4, 5]);
        assert_eq!([0, 1, 2, 4, 5].map(|u| o.to_byte(u)), [0, 1, 3, 7, 8]);
        // A byte inside a character: the start of the character.
        assert_eq!([2, 4, 5, 6].map(|b| o.to_utf16(b)), [1, 2, 2, 2]);
    }

    #[test]
    fn a_unit_inside_a_surrogate_pair_maps_to_the_character() {
        let o = Offsets::new(TEXT);
        assert_eq!(o.to_byte(3), 3);
        let o = Offsets::new("𝄞");
        assert_eq!([0, 1, 2].map(|u| o.to_byte(u)), [0, 0, 4]);
    }

    #[test]
    fn offsets_past_the_end_map_to_the_end() {
        let o = Offsets::new(TEXT);
        assert_eq!((o.to_utf16(99), o.to_byte(99)), (5, 8));
        let ascii = Offsets::new("abc");
        assert_eq!((ascii.to_utf16(9), ascii.to_byte(9)), (3, 3));
    }

    #[test]
    fn the_walk_for_long_texts_agrees_with_the_table() {
        let table = Offsets::new(TEXT);
        let long = Offsets {
            table: Table::Long(TEXT.into()),
            len: TEXT.len(),
        };
        for n in 0..=10 {
            assert_eq!(long.to_utf16(n), table.to_utf16(n), "byte {n}");
            assert_eq!(long.to_byte(n), table.to_byte(n), "unit {n}");
        }
    }
}
