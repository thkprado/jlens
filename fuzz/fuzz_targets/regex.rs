//! Compiling a regular expression and matching it never panic, and matches lie within the text.
//!
//! The first byte of the fuzz input chooses the flags, one bit each for `i`, `m`, `s`, `u` and
//! `v`. The rest is the pattern, a NUL byte, and the text to search.

#![no_main]

use libfuzzer_sys::fuzz_target;

/// How many times a target asks to go on, across the matches of one input: about a million
/// backtracking steps (see `regress::INTERRUPT_INTERVAL`).
const POLLS: u32 = 1000;

fuzz_target!(|data: &[u8]| {
    jlens_fuzz::record(data);
    let Some((&bits, rest)) = data.split_first() else {
        return;
    };
    let (pattern, text) = match rest.iter().position(|&b| b == 0) {
        Some(nul) => (&rest[..nul], &rest[nul + 1..]),
        None => (rest, &[][..]),
    };
    let (Ok(pattern), Ok(text)) = (std::str::from_utf8(pattern), std::str::from_utf8(text)) else {
        return;
    };
    let flags: String = "imsuv"
        .chars()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, c)| c)
        .collect();
    jlens_fuzz::on_small_stack(|| {
        let Ok(re) = regress::Regex::with_flags(pattern, flags.as_str()) else {
            return;
        };
        let mut polls = 0;
        let mut stop = || {
            polls += 1;
            polls > POLLS
        };
        let mut start = 0;
        while start <= text.len() {
            let Ok(Some(m)) = re.find_from_interruptible(text, start, &mut stop) else {
                return;
            };
            for range in m.groups().flatten() {
                assert!(
                    range.start <= range.end && range.end <= text.len(),
                    "{range:?}"
                );
                assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
            }
            for (_, range) in m.named_groups() {
                assert!(range.is_none_or(|r| r.end <= text.len()));
            }
            // An empty match moves on by one character, as JavaScript's `g` flag does.
            start = match text[m.end()..].chars().next() {
                _ if m.end() > m.start() => m.end(),
                Some(c) => m.end() + c.len_utf8(),
                None => break,
            };
        }
    });
});
