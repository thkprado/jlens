//! jlens-regress: matches that an interrupt stops.

use regress::{INTERRUPT_INTERVAL, Interrupted, Regex};
use std::cell::Cell;

/// `n` a's followed by `!`: the patterns below backtrack catastrophically on it.
fn catastrophic_text(n: usize) -> String {
    format!("{}!", "a".repeat(n))
}

/// Runs `find_from_interruptible` with an interrupt that stops at its `stop_at`-th call, and
/// returns the result with the number of calls.
fn run(
    pattern: &str,
    text: &str,
    stop_at: u32,
) -> (Result<Option<regress::Match>, Interrupted>, u32) {
    let re = Regex::new(pattern).unwrap();
    let calls = Cell::new(0);
    let result = re.find_from_interruptible(text, 0, || {
        calls.set(calls.get() + 1);
        calls.get() >= stop_at
    });
    (result, calls.get())
}

#[test]
fn nested_quantifiers_stop() {
    for pattern in [
        r"^(a+)+$",
        r"(a|aa)+$",
        r"^(a|a)*$",
        r"^(\w+\s?)*$",
        r"(a*)*b",
    ] {
        let (result, calls) = run(pattern, &catastrophic_text(64), 3);
        assert_eq!(result.unwrap_err(), Interrupted, "{pattern}");
        assert_eq!(calls, 3, "{pattern}");
    }
}

#[test]
fn backreferences_stop() {
    let (result, calls) = run(r"^(a+)+\1$", &catastrophic_text(64), 2);
    assert!(result.is_err());
    assert_eq!(calls, 2);
}

#[test]
fn negative_lookahead_does_not_succeed_when_interrupted() {
    // Uninterrupted, the inner pattern fails and the negative lookahead succeeds: the match
    // must not be reported as found.
    let (result, _) = run(r"^(?!(a+)+$)", &catastrophic_text(64), 1);
    assert_eq!(result.unwrap_err(), Interrupted);
    let (result, _) = run(r"^(?!(a+)+$)", &catastrophic_text(8), u32::MAX);
    assert_eq!(result.unwrap().unwrap().range(), 0..0);
}

#[test]
fn positive_lookahead_and_lookbehind_stop() {
    let (result, _) = run(r"(?=(a+)+$)a", &catastrophic_text(64), 1);
    assert!(result.is_err());
    let (result, _) = run(r"(?<=b(a+)+)!", &catastrophic_text(64), 1);
    assert!(result.is_err());
}

#[test]
fn loops_without_failures_are_counted() {
    // Every iteration matches; only the last one fails.
    let iterations = 20 * INTERRUPT_INTERVAL as usize;
    let text = "ab".repeat(iterations);
    let (result, calls) = run(r"^(?:(a)b)*$", &text, u32::MAX);
    assert_eq!(result.unwrap().unwrap().range(), 0..text.len());
    assert!(calls >= 19, "{calls} calls");
}

#[test]
fn short_matches_never_call_the_interrupt() {
    let (result, calls) = run(r"(\d+)-(\d+)", "tel 555-0100", 1);
    let m = result.unwrap().unwrap();
    assert_eq!(m.range(), 4..12);
    assert_eq!(m.group(2), Some(8..12));
    assert_eq!(calls, 0);
}

#[test]
fn matches_like_find_from() {
    let cases: &[(&str, &str, &str, usize)] = &[
        (r"(\w)\1", "", "Frankly, Miss Piggy", 0),
        (r"(?<=x)y", "", "xyxy", 1),
        (r"^abc", "m", "x\nabc", 0),
        (r"a*", "", "baaa", 1),
        (r"", "", "", 0),
        (r"é+", "i", "xÉé", 0),
        (r"(?<year>\d{4})-(?<month>\d\d)", "", "on 2026-10", 0),
        (r"\bfoo\b", "", "a foo b", 3),
        (r"x", "", "abc", 4),
    ];
    for &(pattern, flags, text, start) in cases {
        let re = Regex::with_flags(pattern, flags).unwrap();
        let expected = re.find_from(text, start).next();
        let found = re.find_from_interruptible(text, start, || false).unwrap();
        assert_eq!(
            found.as_ref().map(|m| (m.range(), m.captures.clone())),
            expected.as_ref().map(|m| (m.range(), m.captures.clone())),
            "/{pattern}/{flags} on {text:?} from {start}"
        );
    }
}

#[test]
fn a_searcher_matches_like_find_from() {
    // Each search of a searcher reuses the storage the one before left, also when it failed;
    // the matches must be those of a search of its own.
    let cases: &[(&str, &str, &str)] = &[
        (r"\w+", "", "ação coração λόγος 日本語 plain"),
        (r"(\w)(\w*)", "", "one two three"),
        (r"(\w)\1", "", "Frankly, Miss Piggy all good"),
        (r"(?<=x)y", "", "xyxy zy xy"),
        (r"(?<!a)b", "", "ab cb ab db"),
        (r"(?=(\w+))\w", "", "ab cd"),
        (r"(a)|(b)", "", "abba"),
        (r"(?:(a)|b)+", "", "ab ba b"),
        (r"^\w", "m", "x\nabc\n\ndef"),
        (r"é+", "i", "xÉé É"),
        (
            r"(?<year>\d{4})-(?<month>\d\d)",
            "",
            "on 2026-10 and 1999-01",
        ),
        (r"\bfoo\b", "", "a foo b foo"),
        (r"\p{Lu}\p{Ll}*", "u", "Ωμέγα Ünd Text"),
    ];
    for &(pattern, flags, text) in cases {
        let re = Regex::with_flags(pattern, flags).unwrap();
        let mut searcher = re.searcher(text);
        // Every start, so that searches that fail are followed by others.
        let starts = (0..=text.len() + 1).filter(|&i| i > text.len() || text.is_char_boundary(i));
        for start in starts {
            let expected = re.find_from(text, start).next();
            let found = searcher.find_from(start, || false).unwrap();
            assert_eq!(
                found.as_ref().map(|m| (m.range(), m.captures.clone())),
                expected.as_ref().map(|m| (m.range(), m.captures.clone())),
                "/{pattern}/{flags} on {text:?} from {start}"
            );
        }
    }
}

#[test]
fn a_searcher_matches_after_an_interrupt() {
    let re = Regex::new(r"(a+)+$|(b)").unwrap();
    let text = format!("{}!b", "a".repeat(30));
    let mut searcher = re.searcher(&text);
    assert_eq!(searcher.find_from(0, || true).unwrap_err(), Interrupted);
    // From the `!`, where the first branch fails at once and the second takes its group.
    let found = searcher
        .find_from(30, regress::NoInterrupt)
        .unwrap()
        .unwrap();
    assert_eq!(found.range(), 31..32);
    assert_eq!(found.captures, [None, Some(31..32)]);
}
