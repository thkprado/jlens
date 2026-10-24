//! jlens-regress: cases where jlens-regress differed from V8 and from ECMAScript, found by the
//! differential test against V8 (`cargo xtask regex-differential`). The corpus records them
//! (`tests/corpus/v8.json`), with V8's answers.

pub mod common;
use common::*;

/// `v` is Unicode mode, with `u`'s syntax: no Annex B escapes, braces or octal escapes.
#[test]
fn v_flag_has_unicode_syntax() {
    for pattern in [
        "\\-", "\\z", "\\c1", "\\c_", "\\07", "\\1", "a\\2(b)", "\\k<z>", "}", "]", "a{", "a{1",
    ] {
        test_parse_fails_flags(pattern, "v");
        test_parse_fails_flags(pattern, "u");
    }
}

fn v_flag_matches_code_points_tc(tc: TestConfig) {
    // Case folding is Unicode's simple case folding, as with `u`.
    tc.compilef("\\u212A", "iv").test_succeeds("k");
    tc.compilef("\\u017F", "iv").test_succeeds("S");
    tc.compilef("^.$", "v").test_succeeds("\u{1F600}");
}

#[test]
fn v_flag_matches_code_points() {
    test_with_configs(v_flag_matches_code_points_tc)
}

/// An assertion takes no quantifier, in any mode; Annex B quantifies only lookaheads.
#[test]
fn quantified_word_boundaries_fail() {
    for flags in ["", "u", "v", "i"] {
        for pattern in ["\\b+", "\\b*", "\\b{1,}", "a\\B?", "\\B{1}"] {
            test_parse_fails_flags(pattern, flags);
        }
    }
}

fn canonicalize_without_unicode_tc(tc: TestConfig) {
    // Canonicalize never maps a non-ASCII character to an ASCII one.
    tc.compilef("\u{131}", "i").test_fails("I");
    tc.compilef("\u{131}", "i").test_fails("i");
    tc.compilef("\u{131}", "i").test_succeeds("\u{131}");
    tc.compilef("I", "i").test_fails("\u{131}");
    tc.compilef("\u{17F}", "i").test_fails("s");
    tc.compilef("s", "i").test_fails("\u{17F}");
    tc.compilef("[a-z]", "i").test_fails("\u{17F}");
    tc.compilef("[a-z]", "i").test_fails("\u{212A}");
    tc.compilef("\\w", "i").test_fails("\u{17F}");
    tc.compilef("\\w", "i").test_fails("\u{212A}");
    tc.compilef("[\\W]", "i").test_fails("k");
    tc.compilef("[\\W]", "i").test_succeeds("\u{212A}");
    // Nor a character whose uppercase is two characters.
    tc.compilef("\u{1F80}", "i").test_fails("\u{1F88}");
    tc.compilef("\u{1FB3}", "i").test_fails("\u{1FBC}");
    // With `u`, case folding relates them.
    tc.compilef("[a-z]", "iu").test_succeeds("\u{17F}");
    tc.compilef("\\w", "iu").test_succeeds("\u{212A}");
    tc.compilef("\u{1F80}", "iu").test_succeeds("\u{1F88}");
}

#[test]
fn canonicalize_without_unicode() {
    test_with_configs(canonicalize_without_unicode_tc)
}

fn word_complement_with_unicode_tc(tc: TestConfig) {
    // With `u` and `i`, the word characters include the Kelvin sign and the long s.
    tc.compilef("[\\W]", "iu").test_fails("\u{212A}");
    tc.compilef("[\\W]", "iu").test_fails("k");
    tc.compilef("[\\W]", "iu").test_fails("\u{17F}");
    tc.compilef("[\\W-]", "iu").test_succeeds("-");
}

#[test]
fn word_complement_with_unicode() {
    test_with_configs(word_complement_with_unicode_tc)
}

fn class_set_complements_fold_first_tc(tc: TestConfig) {
    // With `v` and `i`, a complement is taken after case folding.
    tc.compilef("\\P{Lowercase}", "iv").test_fails("A");
    tc.compilef("\\P{Lowercase}", "iv").test_succeeds(" ");
    tc.compilef("[\\P{Lowercase}]", "iv").test_fails("A");
    tc.compilef("[^\\P{Lowercase}]", "iv").test_succeeds("A");
    tc.compilef("[^\\W]", "iv").test_succeeds("\u{17F}");
    tc.compilef("[^\\W]", "iv").test_succeeds("\u{212A}");
    tc.compilef("[\\W]", "iv").test_fails("\u{212A}");
    // With `u`, it is not.
    tc.compilef("\\P{Lowercase}", "iu").test_succeeds("A");
    // `\p{ASCII}` folds like any property: the long s folds to `s`.
    tc.compilef("[\\p{ASCII}]", "iv").test_succeeds("\u{17F}");
    tc.compilef("\\P{ASCII}", "iv").test_fails("\u{17F}");
}

#[test]
fn class_set_complements_fold_first() {
    test_with_configs(class_set_complements_fold_first_tc)
}

fn class_set_operations_fold_operands_tc(tc: TestConfig) {
    tc.compilef("[\\p{Ll}&&\\u{C9}]", "iv")
        .test_succeeds("\u{C9}");
    tc.compilef("[^\\u017F&&s]", "iv").test_fails("S");
    tc.compilef("[^\\u017F&&s]", "iv").test_succeeds("c");
    tc.compilef("[[_-\u{E9}]--I]", "iv").test_fails("I");
    tc.compilef("[[_-\u{E9}]--I]", "iv").test_succeeds("a");
    tc.compilef("[\\p{L}--\\q{\u{3C3}}]", "iv")
        .test_fails("\u{3A3}");
    tc.compilef("[\\q{\u{3C3}}]", "iv").test_succeeds("\u{3A3}");
    tc.compilef("[\\q{\u{3C3}|ab}]", "iv")
        .match1f("AB")
        .test_eq("AB");
    tc.compilef("[[[\\r0]\\.-K -\u{3C2}]&&\u{1E9E}]", "iv")
        .test_succeeds("\u{DF}");
    tc.compilef("[[[\\r0]\\.-K -\u{3C2}]&&\u{1E9E}]", "iv")
        .test_succeeds("\u{1E9E}");
}

#[test]
fn class_set_operations_fold_operands() {
    test_with_configs(class_set_operations_fold_operands_tc)
}

fn class_set_backspace_tc(tc: TestConfig) {
    tc.compilef("[\\b]", "v").test_succeeds("\u{8}");
    tc.compilef("[\\b]", "v").test_fails("b");
    tc.compilef("[\\q{\\b}]", "v").test_succeeds("\u{8}");
}

#[test]
fn class_set_backspace() {
    test_with_configs(class_set_backspace_tc)
}

fn optional_lone_surrogates_tc(tc: TestConfig) {
    // A lone surrogate matches nothing in a `&str`, so a loop of it matches zero times.
    for flags in ["", "u", "v"] {
        tc.compilef("a\\uD83D?", flags).match1f("a").test_eq("a");
        tc.compilef("\\uD83D*b", flags).match1f("ab").test_eq("b");
        tc.compilef("a\\uD83D{0,2}?b", flags)
            .match1f("ab")
            .test_eq("ab");
        tc.compilef("a\\uD83D+", flags).test_fails("a");
    }
}

#[test]
fn optional_lone_surrogates() {
    test_with_configs(optional_lone_surrogates_tc)
}

fn lead_surrogate_before_code_point_escape_tc(tc: TestConfig) {
    // `\uD83D` with no `\uXXXX` after it is a lone surrogate; `\u{...}` is its own escape.
    tc.compilef("\\uD83D\\u{1F600}", "u")
        .test_fails("\u{1F600}");
    tc.compilef("\\uD83D\\u{1F600}|b", "v").test_succeeds("b");
}

#[test]
fn lead_surrogate_before_code_point_escape() {
    test_with_configs(lead_surrogate_before_code_point_escape_tc)
}

fn modifiers_scope_case_folding_tc(tc: TestConfig) {
    // `-i` holds for the whole group, quantifiers included, and only inside it.
    tc.compilef("(?-i:[^k]{1,2}?)b", "i")
        .match1f("Kb")
        .test_eq("Kb");
    tc.compilef("(?-i:x)?\\W", "iu").test_fails("\u{212A}");
}

#[test]
fn modifiers_scope_case_folding() {
    test_with_configs(modifiers_scope_case_folding_tc)
}

/// Two groups of the same name that both take part in a match, though one is in an alternative
/// of another group.
#[test]
fn duplicate_names_in_sibling_groups_fail() {
    test_parse_fails("((?<a>)){0,1}(b||(?<a>))");
    test_parse_fails("(?<a>)(?=b|(?<a>))");
    test_parse_fails("(x|(?<a>))(y|(?<a>))");
}

/// `named_group` gives the group of that name that took part in the match.
fn duplicate_names_in_alternatives_tc(tc: TestConfig) {
    tc.compilef("(x|(?<a>y))|(z|(?<a>w))", "")
        .match1_named_group("w", "a")
        .test_eq("w");
    tc.compilef("(?:(?<a>x)|(?<a>y))(?:(?<b>z)|(?<b>w))", "")
        .match1_named_group("yz", "b")
        .test_eq("z");
}

#[test]
fn duplicate_names_in_alternatives() {
    test_with_configs(duplicate_names_in_alternatives_tc)
}
