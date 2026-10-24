//! `cargo xtask regex-differential`: generated regular expressions run in jlens-regress and in
//! V8 (Node.js), which must agree on whether each pattern compiles and on every match.
//!
//! The generator is seeded, so a run is reproduced by its seed. Each case is a pattern, its
//! flags and a few inputs. The cases go to `target/regex-differential/cases.json`;
//! `crates/jlens-regress/tests/differential.mjs` runs them in Node.js and writes V8's outcomes
//! next to them; the divergences are written to `target/regex-differential/divergences.json`.
//!
//! Each input is searched as `String.prototype.matchAll` does: from the start, then from the end
//! of each match, one character on after an empty one, up to `MAX_MATCHES` matches. A match is
//! the UTF-16 ranges of the whole match, of each group and of each named group.
//!
//! Four kinds of disagreement are not divergences, and are counted apart: a search V8 did not
//! finish within the timeout; a search jlens-regress did not finish within `POLLS`; without the
//! `u` or `v` flag, a case with a character outside the BMP, which V8 matches as two UTF-16 code
//! units and jlens-regress as one character (see `tests/corpus/differences.txt`); and a case that
//! may meet one of V8's bugs (`v8_bug`). A search that V8 would go on inside a surrogate pair is
//! compared up to there.

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use regress::Regex;
use serde_json::{Value, json};

use crate::differential::{Rng, flag};
use crate::root;

/// How long V8 may run the searches of one case.
const TIMEOUT_MS: u64 = 1000;

/// How many times jlens-regress may ask to go on in the search of one input: about ten million
/// backtracking steps (see `regress::INTERRUPT_INTERVAL`).
const POLLS: u32 = 10_000;

/// Matches compared per input.
const MAX_MATCHES: usize = 20;

/// Inputs searched per pattern.
const INPUTS: usize = 3;

/// How many divergences are printed; all of them are written to the file.
const PRINTED: usize = 10;

/// Characters of the generated patterns and inputs: letters whose case folding differs between
/// the Unicode and non-Unicode modes (`K` and the Kelvin sign, `s` and the long s, `ß`, the
/// sigmas, the dotted and dotless i), digits, `_`, the spaces and line terminators of `\s`, `.`
/// and `$`, and characters outside the BMP, one of them with a case pair.
const ALPHABET: &[char] = &[
    'a',
    'b',
    'c',
    'A',
    'B',
    'C',
    'k',
    'K',
    '\u{212A}',
    's',
    'S',
    '\u{17F}',
    'i',
    'I',
    '\u{130}',
    '\u{131}',
    'é',
    'É',
    'ß',
    '\u{1E9E}',
    'σ',
    'ς',
    'Σ',
    '0',
    '1',
    '9',
    '_',
    '-',
    '.',
    ' ',
    '\t',
    '\u{A0}',
    '\u{FEFF}',
    '\n',
    '\r',
    '\u{2028}',
    '\u{1D11E}',
    '\u{1F600}',
    '\u{10400}',
    '\u{10428}',
];

/// Pieces of pattern that the grammar does not build: Annex B's leniencies (a lone brace, `\c`
/// without a letter, octal and identity escapes, `\p` without `u`), escapes valid only with `u`,
/// surrogates written as escapes, and syntax errors.
const ODD: &[&str] = &[
    "{",
    "}",
    "]",
    "{1}",
    "a{",
    "a{1",
    "a{,2}",
    "\\c",
    "\\cJ",
    "\\cj",
    "\\c1",
    "\\c_",
    "\\k",
    "\\8",
    "\\9",
    "\\0",
    "\\00",
    "\\07",
    "\\1",
    "\\-",
    "\\/",
    "\\u",
    "\\u12",
    "\\x",
    "\\x4",
    "\\u{1F600}",
    "\\u{110000}",
    "\\uD83D\\uDE00",
    "\\uD83D",
    "\\uDE00",
    "\\p{L}",
    "\\P{Ll}",
    "\\p",
    "\\e",
    "\\z",
    "\\_",
    "(",
    ")",
    "[",
    "*",
    "+",
    "?",
    "(?",
    "(?<",
    "(?<a",
    "(?:",
    "\\q{a}",
    "[]",
    "[^]",
    "$^",
    "(?=)",
    "(?!)",
];

/// Property escapes whose characters are the same in any recent Unicode version, for the
/// characters of `ALPHABET`.
const PROPERTIES: &[&str] = &[
    "L",
    "Lu",
    "Ll",
    "Nd",
    "P",
    "Zs",
    "ASCII",
    "Any",
    "White_Space",
    "Script=Latin",
    "Script=Greek",
    "sc=Deseret",
    "Lowercase",
    "Changes_When_Casefolded",
];

/// One generated case: a pattern, its flags, and the inputs it searches.
struct Case {
    pattern: String,
    flags: String,
    inputs: Vec<String>,
}

pub fn run(args: &[String]) -> Result<()> {
    let count = flag(args, "--cases")?.unwrap_or(1000);
    let seed = match flag(args, "--seed")? {
        Some(seed) => seed,
        None => SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64,
    };

    let mut rng = Rng(seed);
    let cases: Vec<Case> = (0..count).map(|_| case(&mut rng)).collect();

    let dir = root().join("target/regex-differential");
    fs::create_dir_all(&dir)?;
    // A file left by an earlier run would be mistaken for this run's.
    let _ = fs::remove_file(dir.join("divergences.json"));
    let cases_path = dir.join("cases.json");
    let outcomes_path = dir.join("outcomes.json");
    let file = json!({
        "seed": seed,
        "timeout": TIMEOUT_MS,
        "max": MAX_MATCHES,
        "cases": cases
            .iter()
            .map(|c| json!({"pattern": c.pattern, "flags": c.flags, "inputs": c.inputs}))
            .collect::<Vec<_>>(),
    });
    fs::write(&cases_path, serde_json::to_string_pretty(&file)?)?;

    let status = Command::new("node")
        .arg(root().join("crates/jlens-regress/tests/differential.mjs"))
        .arg(&cases_path)
        .arg(&outcomes_path)
        .status()
        .context("running node (Node.js 24+ is needed)")?;
    if !status.success() {
        bail!("the V8 runner failed: {status}");
    }
    let reference: Vec<Value> = serde_json::from_str(&fs::read_to_string(&outcomes_path)?)?;
    if reference.len() != cases.len() {
        bail!(
            "the V8 runner wrote {} outcomes for {} cases",
            reference.len(),
            cases.len()
        );
    }

    let (mut agreed, mut hung, mut interrupted, mut code_units, mut compiled) = (0, 0, 0, 0, 0);
    let mut v8_bugs = 0;
    let mut divergences = Vec::new();
    for (case, expected) in cases.iter().zip(&reference) {
        if expected["hung"] == true {
            hung += 1;
            continue;
        }
        let Some(actual) = outcome(case) else {
            interrupted += 1;
            continue;
        };
        if actual["error"] != true {
            compiled += 1;
        }
        if agree(&actual, expected) {
            agreed += 1;
        } else if code_unit_case(case) {
            code_units += 1;
        } else if v8_bug(case, &actual) {
            v8_bugs += 1;
        } else {
            divergences.push(json!({
                "pattern": case.pattern,
                "flags": case.flags,
                "inputs": case.inputs,
                "jlens-regress": actual,
                "v8": expected,
            }));
        }
    }

    for d in divergences.iter().take(PRINTED) {
        println!(
            "/{}/{}",
            d["pattern"].as_str().unwrap_or_default(),
            d["flags"]
        );
        println!("  inputs:        {}", d["inputs"]);
        println!("  jlens-regress: {}", d["jlens-regress"]);
        println!("  V8:            {}\n", d["v8"]);
    }
    println!(
        "{count} patterns (seed {seed}), {compiled} of them valid: {agreed} agree, {} differ",
        divergences.len()
    );
    println!(
        "skipped: {hung} (V8 timed out), {interrupted} (jlens-regress interrupted), \
         {code_units} (UTF-16 code units without u or v), \
         {v8_bugs} (known V8 bugs)"
    );
    if !divergences.is_empty() {
        let path = dir.join("divergences.json");
        fs::write(&path, serde_json::to_string_pretty(&divergences)?)?;
        bail!(
            "jlens-regress and V8 differ; all divergences are in {}",
            path.display()
        );
    }
    Ok(())
}

/// The outcome of a case in jlens-regress, in the form the V8 runner writes, or `None` when a
/// search ran out of polls.
fn outcome(case: &Case) -> Option<Value> {
    let Ok(re) = Regex::with_flags(&case.pattern, case.flags.as_str()) else {
        return Some(json!({ "error": true }));
    };
    let unicode = case.flags.contains(['u', 'v']);
    let results = case
        .inputs
        .iter()
        .map(|input| search(&re, input, unicode))
        .collect::<Option<Vec<_>>>()?;
    Some(json!({ "results": results }))
}

/// The matches of `re` in `text`, as `String.prototype.matchAll` finds them.
fn search(re: &Regex, text: &str, unicode: bool) -> Option<Value> {
    let mut offsets = vec![0; text.len() + 1];
    let mut u16 = 0;
    for (byte, c) in text.char_indices() {
        offsets[byte] = u16;
        u16 += c.len_utf16();
    }
    offsets[text.len()] = u16;
    let to16 = |r: regress::Range| json!([offsets[r.start], offsets[r.end]]);

    let mut polls = 0;
    let mut stop = || {
        polls += 1;
        polls > POLLS
    };
    let mut matches = Vec::new();
    let mut split = false;
    let mut start = 0;
    while matches.len() < MAX_MATCHES {
        let Some(m) = re.find_from_interruptible(text, start, &mut stop).ok()? else {
            break;
        };
        matches.push(json!({
            "ranges": m.groups().map(|r| r.map(to16)).collect::<Vec<_>>(),
            "groups": m
                .named_groups()
                .map(|(name, r)| (name.to_string(), json!(r.map(to16))))
                .collect::<serde_json::Map<_, _>>(),
        }));
        if m.end() > m.start() {
            start = m.end();
            continue;
        }
        // An empty match moves on by one character, or one UTF-16 code unit without u or v,
        // which inside a surrogate pair is where the comparison stops.
        match text[m.end()..].chars().next() {
            None => break,
            Some(c) if !unicode && c.len_utf16() == 2 => {
                split = true;
                break;
            }
            Some(c) => start = m.end() + c.len_utf8(),
        }
    }
    Some(json!({ "matches": matches, "split": split }))
}

/// Whether the outcomes agree. A search that V8 stopped inside a surrogate pair agrees when
/// jlens-regress found the same matches up to there.
fn agree(actual: &Value, expected: &Value) -> bool {
    if actual["error"] == true || expected["error"] == true {
        return actual["error"] == expected["error"];
    }
    let (Some(actual), Some(expected)) =
        (actual["results"].as_array(), expected["results"].as_array())
    else {
        return false;
    };
    actual.iter().zip(expected).all(|(a, e)| {
        let (Some(am), Some(em)) = (a["matches"].as_array(), e["matches"].as_array()) else {
            return false;
        };
        if e["split"] == true {
            am.len() >= em.len() && am[..em.len()] == em[..]
        } else {
            a == e
        }
    })
}

/// Whether V8 may see the case as UTF-16 code units where jlens-regress sees characters: it has
/// no `u` or `v` flag, and a character outside the BMP in its pattern or inputs.
fn code_unit_case(case: &Case) -> bool {
    let astral = |text: &str| text.chars().any(|c| c > '\u{FFFF}');
    !case.flags.contains(['u', 'v'])
        && (astral(&case.pattern) || case.inputs.iter().any(|i| astral(i)))
}

/// Whether a case that differs may meet one of V8's bugs, where jlens-regress follows the
/// specification:
///
/// - jlens-regress rejected a pattern with two groups of the same name that both take part in a
///   match, which V8 accepts when one of them is in an alternative of a group the other is not in
///   (`/(?<a>)(?=b|(?<a>))/`);
/// - a group that changes the `i` flag, around which V8 folds case wrongly in places
///   (`/(?-i:x)?\W/iu` matches the Kelvin sign, which `/\W/iu` does not; `/(?-i:[^k]{1,2}?)b/i`
///   does not match `Kb`, which `/(?-i:[^k]+?)b/i` does);
/// - with the `v` and `i` flags, `\p{ASCII}` and `\P{ASCII}`, which V8 does not fold
///   (`/[\p{ASCII}]/iv` does not match `ſ`, though `/[a-z]/iv` does);
/// - with the `v` and `i` flags, a string of `\q{...}`, which V8 folds for some characters only
///   (`/[\q{É}]/iv` matches `é`, `/[\q{σ}]/iv` does not match `Σ`).
fn v8_bug(case: &Case, actual: &Value) -> bool {
    let vi = case.flags.contains('v') && case.flags.contains('i');
    (actual["error"] == true && repeated_name(&case.pattern))
        || changes_case(&case.pattern)
        || (vi && (case.pattern.contains("{ASCII}") || case.pattern.contains("\\q{")))
}

/// Whether two groups of `pattern` have the same name. The generator writes `a` escaped in
/// some names, which counts as `a`.
fn repeated_name(pattern: &str) -> bool {
    let pattern = pattern.replace("(?<\\u0061>", "(?<a>");
    ["a", "b"]
        .iter()
        .any(|name| pattern.matches(&format!("(?<{name}>")).count() > 1)
}

/// Whether `pattern` has a group that sets or removes the `i` flag (`(?i:`, `(?-i:`, `(?s-i:`).
fn changes_case(pattern: &str) -> bool {
    pattern.split("(?").skip(1).any(|rest| {
        let modifiers: String = rest.chars().take_while(|c| "ims-".contains(*c)).collect();
        modifiers.contains('i') && rest[modifiers.len()..].starts_with(':')
    })
}

/// A generated case. Most have neither `u` nor `v`, as jsonata-js compiles every pattern.
fn case(rng: &mut Rng) -> Case {
    let mut flags = String::new();
    for (flag, percent) in [('i', 50), ('m', 25), ('s', 20)] {
        if rng.below(100) < percent {
            flags.push(flag);
        }
    }
    let mode = rng.below(10);
    if mode >= 8 {
        flags.push('v');
    } else if mode >= 6 {
        flags.push('u');
    }
    let mut generator = Generator {
        rng,
        sets: mode >= 8,
        groups: 0,
        literals: Vec::new(),
    };
    let pattern = generator.disjunction(3);
    let inputs = (0..INPUTS).map(|_| generator.input()).collect();
    Case {
        pattern,
        flags,
        inputs,
    }
}

/// Builds a pattern from the grammar of ECMAScript's regular expressions, with Annex B's
/// extensions and the odd pieces of `ODD`, then inputs from the characters it wrote.
struct Generator<'a> {
    rng: &'a mut Rng,
    /// The `v` flag.
    sets: bool,
    /// Capturing groups opened so far.
    groups: usize,
    /// The characters the pattern matches literally.
    literals: Vec<char>,
}

impl Generator<'_> {
    fn chance(&mut self, percent: usize) -> bool {
        self.rng.below(100) < percent
    }

    fn char(&mut self) -> char {
        ALPHABET[self.rng.below(ALPHABET.len())]
    }

    fn input(&mut self) -> String {
        let len = self.rng.below(13);
        (0..len)
            .map(|_| {
                if !self.literals.is_empty() && self.chance(60) {
                    self.literals[self.rng.below(self.literals.len())]
                } else {
                    self.char()
                }
            })
            .collect()
    }

    fn disjunction(&mut self, depth: usize) -> String {
        let n = if self.chance(20) {
            2 + self.rng.below(2)
        } else {
            1
        };
        (0..n)
            .map(|_| self.alternative(depth))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn alternative(&mut self, depth: usize) -> String {
        let n = self.rng.below(5);
        (0..n).map(|_| self.term(depth)).collect()
    }

    fn term(&mut self, depth: usize) -> String {
        match self.rng.below(100) {
            0..10 => self.assertion(depth),
            10..15 => self.rng.pick(ODD).to_string(),
            _ => {
                let atom = self.atom(depth);
                if self.chance(30) {
                    atom + &self.quantifier()
                } else {
                    atom
                }
            }
        }
    }

    fn assertion(&mut self, depth: usize) -> String {
        let assertion = match self.rng.below(8) {
            0 => "^".to_string(),
            1 => "$".to_string(),
            2 => "\\b".to_string(),
            3 => "\\B".to_string(),
            _ if depth > 0 => {
                let kind = self.rng.pick(&["?=", "?!", "?<=", "?<!"]);
                format!("({kind}{})", self.disjunction(depth - 1))
            }
            _ => "^".to_string(),
        };
        // Annex B quantifies a lookahead without u or v; everything else, it rejects.
        if self.chance(10) {
            assertion + &self.quantifier()
        } else {
            assertion
        }
    }

    fn atom(&mut self, depth: usize) -> String {
        match self.rng.below(100) {
            0..45 => self.literal(false),
            45..52 => ".".to_string(),
            52..62 => self.class_escape(),
            62..75 => self.class(depth),
            75..92 if depth > 0 => self.group(depth),
            92..100 => self.backreference(),
            _ => self.literal(false),
        }
    }

    fn group(&mut self, depth: usize) -> String {
        match self.rng.below(10) {
            0..4 => {
                self.groups += 1;
                format!("({})", self.disjunction(depth - 1))
            }
            // Few names, so that some repeat: in different alternatives, which ES2025 allows,
            // or in the same one, which it does not.
            4..6 => {
                self.groups += 1;
                let name = self.rng.pick(&["a", "b", "\\u0061"]);
                format!("(?<{name}>{})", self.disjunction(depth - 1))
            }
            6..9 => format!("(?:{})", self.disjunction(depth - 1)),
            _ => {
                let modifiers = self.rng.pick(&[
                    "i", "-i", "m", "s", "i-m", "-s", "is", "s-i", "-ms", "", "-", "ii", "i-i", "x",
                ]);
                format!("(?{modifiers}:{})", self.disjunction(depth - 1))
            }
        }
    }

    /// A reference to a group: by number, possibly one that comes later or does not exist (an
    /// octal escape in Annex B), or by name, possibly one that does not exist (`\k` is then
    /// literal in Annex B, when the pattern has no named group).
    fn backreference(&mut self) -> String {
        match self.rng.below(4) {
            0 | 1 => format!("\\{}", 1 + self.rng.below(self.groups + 2)),
            2 => format!("\\k<{}>", self.rng.pick(&["a", "b"])),
            _ => "\\k<z>".to_string(),
        }
    }

    fn quantifier(&mut self) -> String {
        let quantifier = self.rng.pick(&[
            "*", "+", "?", "{2}", "{0,1}", "{1,}", "{0}", "{2,3}", "{3,2}", "{1,2}",
        ]);
        if self.chance(30) {
            format!("{quantifier}?")
        } else {
            quantifier.to_string()
        }
    }

    fn class_escape(&mut self) -> String {
        if self.chance(15) {
            let property = self.rng.pick(PROPERTIES);
            let p = if self.chance(30) { 'P' } else { 'p' };
            return format!("\\{p}{{{property}}}");
        }
        self.rng
            .pick(&["\\d", "\\D", "\\w", "\\W", "\\s", "\\S"])
            .to_string()
    }

    /// A character the pattern matches, written plainly or as one of its escapes.
    fn literal(&mut self, in_class: bool) -> String {
        let c = self.char();
        self.literals.push(c);
        self.spell(c, in_class)
    }

    fn spell(&mut self, c: char, in_class: bool) -> String {
        let code = c as u32;
        match self.rng.below(10) {
            0 if code < 0x100 => format!("\\x{code:02X}"),
            1 if code < 0x10000 => format!("\\u{code:04X}"),
            1 => {
                let mut units = [0; 2];
                c.encode_utf16(&mut units);
                format!("\\u{:04X}\\u{:04X}", units[0], units[1])
            }
            // Without u or v, `\u{41}` is `u` repeated 41 times.
            2 => format!("\\u{{{code:X}}}"),
            _ => {
                let special = if in_class {
                    "\\]^-.".contains(c)
                } else {
                    "^$\\.*+?()[]{}|/".contains(c)
                };
                if special {
                    format!("\\{c}")
                } else {
                    c.to_string()
                }
            }
        }
    }

    fn class(&mut self, depth: usize) -> String {
        let negated = if self.chance(30) { "^" } else { "" };
        if self.sets && depth > 0 && self.chance(25) {
            let operator = self.rng.pick(&["&&", "--"]);
            let (a, b) = (self.set_operand(depth), self.set_operand(depth));
            return format!("[{negated}{a}{operator}{b}]");
        }
        let n = 1 + self.rng.below(3);
        let items: String = (0..n).map(|_| self.class_item(depth)).collect();
        format!("[{negated}{items}]")
    }

    fn set_operand(&mut self, depth: usize) -> String {
        match self.rng.below(4) {
            0 => self.class(depth - 1),
            1 => self.class_escape(),
            2 => self.string_disjunction(),
            _ => self.literal(true),
        }
    }

    /// `\q{...}`, the strings of a class with the `v` flag.
    fn string_disjunction(&mut self) -> String {
        let n = 1 + self.rng.below(3);
        let strings: Vec<String> = (0..n)
            .map(|_| {
                let len = self.rng.below(3);
                (0..len).map(|_| self.literal(true)).collect()
            })
            .collect();
        format!("\\q{{{}}}", strings.join("|"))
    }

    fn class_item(&mut self, depth: usize) -> String {
        match self.rng.below(100) {
            0..40 => self.literal(true),
            40..60 => {
                let (mut lo, mut hi) = (self.char(), self.char());
                // Mostly in order: a range out of order is a syntax error.
                if lo > hi && self.chance(80) {
                    (lo, hi) = (hi, lo);
                }
                self.literals.extend([lo, hi]);
                let (lo, hi) = (self.spell(lo, true), self.spell(hi, true));
                format!("{lo}-{hi}")
            }
            60..75 => self.class_escape(),
            75..80 => "\\b".to_string(),
            // A class escape as the end of a range: literal `-` in Annex B, an error otherwise.
            80..85 => self.rng.pick(&["\\d-z", "a-\\w", "\\s-\\S"]).to_string(),
            85..95 if self.sets && depth > 0 => self.class(depth - 1),
            85..95 if self.sets => self.string_disjunction(),
            _ => self.literal(true),
        }
    }
}
