//! Date/time and integer picture strings (XPath F&O 3.1 `format-dateTime`, `format-integer`).
//! Ported from jsonata-js `src/datetime.js` (MIT).
//!
//! Calendar arithmetic uses the proleptic Gregorian calendar in UTC, like JavaScript's `Date`
//! with its `UTC` methods, so no time-zone database is needed.

use std::sync::LazyLock;

use crate::error::{Error, Result};
use crate::functions::Decimal;
use crate::value::js_number;

const FEW: [&str; 20] = [
    "Zero",
    "One",
    "Two",
    "Three",
    "Four",
    "Five",
    "Six",
    "Seven",
    "Eight",
    "Nine",
    "Ten",
    "Eleven",
    "Twelve",
    "Thirteen",
    "Fourteen",
    "Fifteen",
    "Sixteen",
    "Seventeen",
    "Eighteen",
    "Nineteen",
];
const ORDINALS: [&str; 20] = [
    "Zeroth",
    "First",
    "Second",
    "Third",
    "Fourth",
    "Fifth",
    "Sixth",
    "Seventh",
    "Eighth",
    "Ninth",
    "Tenth",
    "Eleventh",
    "Twelfth",
    "Thirteenth",
    "Fourteenth",
    "Fifteenth",
    "Sixteenth",
    "Seventeenth",
    "Eighteenth",
    "Nineteenth",
];
const DECADES: [&str; 9] = [
    "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety", "Hundred",
];
const MAGNITUDES: [&str; 4] = ["Thousand", "Million", "Billion", "Trillion"];
const DAYS: [&str; 8] = [
    "",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MILLIS_IN_A_DAY: f64 = 86_400_000.0;

// ---------------------------------------------------------------------------------------------
// Numbers in words, roman numerals and letters

fn number_to_words(value: f64, ordinal: bool) -> String {
    // Doubles throughout, like jsonata-js, so that values beyond u64 still work.
    fn lookup(num: f64, prev: bool, ord: bool) -> String {
        if num <= 19.0 {
            format!(
                "{}{}",
                if prev { " and " } else { "" },
                if ord {
                    ORDINALS[num as usize]
                } else {
                    FEW[num as usize]
                }
            )
        } else if num < 100.0 {
            let tens = (num / 10.0).floor() as usize;
            let remainder = num % 10.0;
            let mut words = format!("{}{}", if prev { " and " } else { "" }, DECADES[tens - 2]);
            if remainder > 0.0 {
                words.push('-');
                words.push_str(&lookup(remainder, false, ord));
            } else if ord {
                words.pop();
                words.push_str("ieth");
            }
            words
        } else if num < 1000.0 {
            let hundreds = (num / 100.0).floor() as usize;
            let remainder = num % 100.0;
            let mut words = format!("{}{} Hundred", if prev { ", " } else { "" }, FEW[hundreds]);
            if remainder > 0.0 {
                words.push_str(&lookup(remainder, true, ord));
            } else if ord {
                words.push_str("th");
            }
            words
        } else {
            // floor(log10(num)), read from the exact scientific form.
            let exponent: usize = format!("{num:e}")
                .split_once('e')
                .and_then(|(_, e)| e.parse().ok())
                .unwrap_or(0);
            let mag = (exponent / 3).min(MAGNITUDES.len());
            let factor = 10f64.powi(3 * mag as i32);
            let mant = (num / factor).floor();
            let remainder = num - mant * factor;
            let mut words = format!(
                "{}{} {}",
                if prev { ", " } else { "" },
                lookup(mant, false, false),
                MAGNITUDES[mag - 1]
            );
            if remainder > 0.0 {
                words.push_str(&lookup(remainder, true, ord));
            } else if ord {
                words.push_str("th");
            }
            words
        }
    }
    // NaN and infinity (from a `Source`) are written as nothing, like roman numerals and letters.
    // jsonata-js recurses until its stack overflows.
    if !value.is_finite() {
        return String::new();
    }
    lookup(value, false, ordinal)
}

/// The word → value table, in jsonata-js's insertion order (it also builds the parsing regex).
fn word_values() -> Vec<(String, f64)> {
    let mut table: Vec<(String, f64)> = Vec::new();
    let mut set = |k: String, v: f64| match table.iter_mut().find(|(key, _)| *key == k) {
        Some(entry) => entry.1 = v,
        None => table.push((k, v)),
    };
    for (i, w) in FEW.iter().enumerate() {
        set(w.to_lowercase(), i as f64);
    }
    for (i, w) in ORDINALS.iter().enumerate() {
        set(w.to_lowercase(), i as f64);
    }
    for (i, w) in DECADES.iter().enumerate() {
        let lword = w.to_lowercase();
        let v = ((i + 2) * 10) as f64;
        set(lword.clone(), v);
        set(format!("{}ieth", &lword[..lword.len() - 1]), v);
    }
    set("hundredth".into(), 100.0);
    for (i, w) in MAGNITUDES.iter().enumerate() {
        let lword = w.to_lowercase();
        let v = 10f64.powi(3 * (i as i32 + 1));
        set(lword.clone(), v);
        set(format!("{lword}th"), v);
    }
    table
}

fn words_to_number(text: &str) -> f64 {
    let table = word_values();
    static SPLITTER: LazyLock<regress::Regex> =
        LazyLock::new(|| regress::Regex::new(r",\s|\sand\s|[\s\\-]").expect("valid regex"));
    let splitter = &*SPLITTER;
    let mut parts = Vec::new();
    let mut last = 0;
    for m in splitter.find_iter(text) {
        parts.push(&text[last..m.range.start]);
        last = m.range.end;
    }
    parts.push(&text[last..]);
    let mut segs = vec![0.0];
    for part in parts {
        let value = table
            .iter()
            .find(|(k, _)| k == part)
            .map_or(f64::NAN, |(_, v)| *v);
        if value < 100.0 || value.is_nan() {
            let mut top = segs.pop().unwrap_or(0.0);
            if top >= 1000.0 {
                segs.push(top);
                top = 0.0;
            }
            segs.push(top + value);
        } else {
            let top = segs.pop().unwrap_or(0.0);
            segs.push(top * value);
        }
    }
    segs.iter().sum()
}

/// The largest number written in roman numerals: ten thousand `M`s.
const MAX_ROMAN: f64 = 1e7;

const ROMAN: [(u64, &str); 13] = [
    (1000, "m"),
    (900, "cm"),
    (500, "d"),
    (400, "cd"),
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
];

fn decimal_to_roman(mut value: u64) -> String {
    let mut out = String::new();
    'outer: while value > 0 {
        for (n, s) in ROMAN {
            if value >= n {
                out.push_str(s);
                value -= n;
                continue 'outer;
            }
        }
    }
    out
}

fn roman_to_decimal(roman: &str) -> f64 {
    let mut decimal = 0.0;
    let mut max = 1.0;
    for c in roman.chars().rev() {
        let value = match c {
            'M' => 1000.0,
            'D' => 500.0,
            'C' => 100.0,
            'L' => 50.0,
            'X' => 10.0,
            'V' => 5.0,
            'I' => 1.0,
            _ => f64::NAN,
        };
        if value < max {
            decimal -= value;
        } else {
            max = value;
            decimal += value;
        }
    }
    decimal
}

/// Computed in floating point, as in jsonata-js: above 2^53, `value - 1` rounds to `value`, and
/// the letters follow that rounding.
fn decimal_to_letters(mut value: f64, a: char) -> String {
    // Infinity never reaches zero; jsonata-js loops forever.
    if value.is_infinite() {
        return String::new();
    }
    let mut letters = Vec::new();
    while value > 0.0 {
        letters.push(char::from_u32(((value - 1.0) % 26.0) as u32 + a as u32).unwrap_or('?'));
        value = ((value - 1.0) / 26.0).floor();
    }
    letters.iter().rev().collect()
}

fn letters_to_decimal(letters: &str, a: char) -> f64 {
    letters
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| (c as i64 - a as i64 + 1) as f64 * 26f64.powi(i as i32))
        .sum()
}

// ---------------------------------------------------------------------------------------------
// Integer pictures

#[derive(Debug, Clone, Copy, PartialEq)]
enum Primary {
    Decimal,
    Letters,
    Roman,
    Words,
    Sequence,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Case {
    Upper,
    Lower,
    Title,
}

#[derive(Debug, Clone)]
struct Separator {
    position: usize,
    character: char,
}

#[derive(Debug, Clone)]
struct IntegerFormat {
    primary: Primary,
    case: Case,
    ordinal: bool,
    zero_code: u32,
    mandatory_digits: usize,
    optional_digits: usize,
    regular: bool,
    /// One separator (repeated every `position` digits) when `regular`, otherwise each one.
    separators: Vec<Separator>,
    token: String,
    /// The exact number of digits to read, when another number follows with no literal between;
    /// 0 or none for any number, and a negative one (`[Y,1--2]`) reads nothing.
    parse_width: Option<i64>,
}

const DECIMAL_GROUPS: [u32; 37] = [
    0x30, 0x0660, 0x06F0, 0x07C0, 0x0966, 0x09E6, 0x0A66, 0x0AE6, 0x0B66, 0x0BE6, 0x0C66, 0x0CE6,
    0x0D66, 0x0DE6, 0x0E50, 0x0ED0, 0x0F20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80,
    0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0,
    0xFF10,
];

fn analyse_integer_picture(picture: &str) -> Result<IntegerFormat> {
    let mut format = IntegerFormat {
        primary: Primary::Decimal,
        case: Case::Lower,
        ordinal: false,
        zero_code: 0x30,
        mandatory_digits: 0,
        optional_digits: 0,
        regular: false,
        separators: Vec::new(),
        token: String::new(),
        parse_width: None,
    };
    let primary = match picture.rfind(';') {
        None => picture,
        Some(i) => {
            if picture[i + 1..].starts_with('o') {
                format.ordinal = true;
            }
            &picture[..i]
        }
    };
    match primary {
        "A" => {
            format.case = Case::Upper;
            format.primary = Primary::Letters;
        }
        "a" => format.primary = Primary::Letters,
        "I" => {
            format.case = Case::Upper;
            format.primary = Primary::Roman;
        }
        "i" => format.primary = Primary::Roman,
        "W" => {
            format.case = Case::Upper;
            format.primary = Primary::Words;
        }
        "Ww" => {
            format.case = Case::Title;
            format.primary = Primary::Words;
        }
        "w" => format.primary = Primary::Words,
        _ => {
            let mut zero: Option<u32> = None;
            let mut mandatory = 0;
            let mut optional = 0;
            let mut separators = Vec::new();
            let mut position = 0;
            for c in primary.chars().rev() {
                let cp = c as u32;
                if let Some(&group) = DECIMAL_GROUPS.iter().find(|&&g| cp >= g && cp <= g + 9) {
                    mandatory += 1;
                    position += 1;
                    match zero {
                        None => zero = Some(group),
                        Some(z) if z != group => return Err(Error::new("D3131")),
                        _ => {}
                    }
                } else if c == '#' {
                    position += 1;
                    optional += 1;
                } else {
                    separators.push(Separator {
                        position,
                        character: c,
                    });
                }
            }
            if mandatory > 0 {
                format.primary = Primary::Decimal;
                format.zero_code = zero.unwrap_or(0x30);
                format.mandatory_digits = mandatory;
                format.optional_digits = optional;
                let regular = regular_repeat(&separators);
                if regular > 0 {
                    format.regular = true;
                    format.separators = vec![Separator {
                        position: regular,
                        character: separators[0].character,
                    }];
                } else {
                    format.separators = separators;
                }
            } else {
                format.primary = Primary::Sequence;
                format.token = primary.to_owned();
            }
        }
    }
    Ok(format)
}

fn regular_repeat(separators: &[Separator]) -> usize {
    let Some(first) = separators.first() else {
        return 0;
    };
    if separators.iter().any(|s| s.character != first.character) {
        return 0;
    }
    let indexes: Vec<usize> = separators.iter().map(|s| s.position).collect();
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let factor = indexes.iter().copied().reduce(gcd).unwrap_or(0);
    if factor == 0 {
        return 0;
    }
    // A set, as jsonata-js since its pull request #869: a picture can have a million separators.
    let positions: std::collections::HashSet<usize> = indexes.iter().copied().collect();
    for i in 1..=indexes.len() {
        if !positions.contains(&(i * factor)) {
            return 0;
        }
    }
    factor
}

fn format_integer_with(value: f64, format: &IntegerFormat) -> Result<String> {
    let negative = value < 0.0;
    let value = value.abs();
    let int = value as u64;
    let mut out = match format.primary {
        Primary::Letters => {
            decimal_to_letters(value, if format.case == Case::Upper { 'A' } else { 'a' })
        }
        // There is no numeral above M, so a larger number would be a long run of them: §4.6.1
        // allows an upper bound, and above it the token `1` (jsonata-js pull request #869).
        Primary::Roman if value > MAX_ROMAN => integer_digits(value).into_iter().collect(),
        Primary::Roman => {
            let r = decimal_to_roman(int);
            if format.case == Case::Upper {
                r.to_uppercase()
            } else {
                r
            }
        }
        Primary::Words => {
            let w = number_to_words(value.floor(), format.ordinal);
            match format.case {
                Case::Upper => w.to_uppercase(),
                Case::Lower => w.to_lowercase(),
                Case::Title => w,
            }
        }
        Primary::Decimal => {
            let mut digits = integer_digits(value);
            if digits.len() < format.mandatory_digits {
                let pad = format.mandatory_digits - digits.len();
                digits.splice(0..0, std::iter::repeat_n('0', pad));
            }
            if format.zero_code != 0x30 {
                for d in &mut digits {
                    *d = char::from_u32(*d as u32 + format.zero_code - 0x30).unwrap_or(*d);
                }
            }
            let mut s: String = group_digits(&digits, format).into_iter().collect();
            if format.ordinal {
                let chars: Vec<char> = s.chars().collect();
                let last = chars.last().copied();
                let tens = chars.len() > 1 && chars[chars.len() - 2] == '1';
                let suffix = match last {
                    Some('1') if !tens => "st",
                    Some('2') if !tens => "nd",
                    Some('3') if !tens => "rd",
                    _ => "th",
                };
                s.push_str(suffix);
            }
            s
        }
        Primary::Sequence => {
            return Err(Error::new("D3130").with("value", format.token.as_str()));
        }
    };
    if negative {
        out.insert(0, '-');
    }
    Ok(out)
}

/// The decimal digits of a whole, non-negative `value`: the shortest decimal that converts back
/// to it, written in full (`1e21` is a 1 and 21 zeros), as jsonata-js does since its pull
/// requests #864 and #868.
fn integer_digits(value: f64) -> Vec<char> {
    if !value.is_finite() {
        return js_number(value).chars().collect();
    }
    let mut decimal = Decimal::of(value);
    decimal.round(0);
    let mut out = Vec::new();
    decimal.write(0, &ASCII_DIGITS, &mut out);
    out
}

const ASCII_DIGITS: [char; 10] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

/// `$formatInteger`.
/// Fails with `U1001` when the result would exceed `max_len` bytes.
pub(crate) fn format_integer(value: f64, picture: &str, max_len: usize) -> Result<String> {
    let format = analyse_integer_picture(picture)?;
    // A Roman numeral takes an `M` per thousand, ten thousand of them at most.
    if format.primary == Primary::Roman
        && value.abs() <= MAX_ROMAN
        && value.abs() / 1000.0 > max_len as f64
    {
        return Err(crate::value::string_too_long(max_len));
    }
    format_integer_with(value.floor(), &format)
}

/// Inserts the grouping separators of `format` into `digits`, in one pass: a width modifier can
/// ask for millions of digits, which inserting each separator in place made quadratic.
fn group_digits(digits: &[char], format: &IntegerFormat) -> Vec<char> {
    let n = digits.len();
    if format.regular {
        // A separator before each group of `position` digits, counted from the right.
        let sep = &format.separators[0];
        let mut out = Vec::with_capacity(n + n / sep.position);
        for (i, &d) in digits.iter().enumerate() {
            if i > 0 && (n - i).is_multiple_of(sep.position) {
                out.push(sep.character);
            }
            out.push(d);
        }
        return out;
    }
    // jsonata-js inserts the separators from the leftmost, each where `position` characters
    // follow it. Those with more positions than digits pile up in front; the others end up with
    // exactly `position` digits on their right.
    let mut front = Vec::new();
    let mut anchored = Vec::new();
    for sep in format.separators.iter().rev() {
        if sep.position > n {
            let at = (n + front.len()).saturating_sub(sep.position);
            front.insert(at, sep.character);
        } else {
            anchored.push(sep);
        }
    }
    let mut out = Vec::with_capacity(front.len() + n + anchored.len());
    out.extend(front);
    let mut anchored = anchored.into_iter().peekable();
    for (i, &d) in digits.iter().enumerate() {
        while let Some(sep) = anchored.next_if(|s| s.position == n - i) {
            out.push(sep.character);
        }
        out.push(d);
    }
    out.extend(anchored.map(|s| s.character));
    out
}

/// Reads the text a part of a picture matched; `None` where jsonata-js gives `undefined` (a name
/// it does not know).
type Parser = Box<dyn Fn(&str) -> Option<f64>>;

/// A set of characters a part of a timestamp can be a run of, in upper case: timestamps are
/// matched case-insensitively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CharSet {
    Digits,
    Letters,
    Roman,
}

impl CharSet {
    fn contains(self, c: char) -> bool {
        match self {
            CharSet::Digits => c.is_ascii_digit(),
            CharSet::Letters => c.is_ascii_uppercase(),
            CharSet::Roman => "MDCLXVI".contains(c),
        }
    }
}

/// What one part of a picture matches in a timestamp (jsonata-js's `generateMatcher`, since its
/// pull request #869): a literal, a number in words, or a run of `chars`, the longest that lets
/// the rest match or exactly `width` of them, maybe followed by an ordinal suffix. A time zone has
/// a prefix before the run, and the run can be split by a separator.
#[derive(Debug, Clone, Default)]
struct MatchPart {
    literal: Option<Vec<char>>,
    words: bool,
    /// The characters each position of the prefix can hold (`GMT` and a sign).
    prefix: &'static [&'static str],
    chars: Option<CharSet>,
    separator: Option<Vec<char>>,
    /// `[Z0]` and `[Z01]`: two digits of minutes after a colon, tried first.
    minutes: bool,
    /// A fixed number of characters; 0 for none, and a negative one matches nothing.
    width: i64,
    ordinal: bool,
}

/// The matcher and parser for one integer picture.
fn integer_matcher(format: &IntegerFormat) -> Result<(MatchPart, Parser)> {
    let upper = format.case == Case::Upper;
    Ok(match format.primary {
        Primary::Letters => (
            MatchPart {
                chars: Some(CharSet::Letters),
                ..MatchPart::default()
            },
            Box::new(move |v: &str| Some(letters_to_decimal(v, if upper { 'A' } else { 'a' }))),
        ),
        Primary::Roman => (
            MatchPart {
                chars: Some(CharSet::Roman),
                ..MatchPart::default()
            },
            Box::new(move |v: &str| Some(roman_to_decimal(&v.to_uppercase()))),
        ),
        Primary::Words => (
            MatchPart {
                words: true,
                ..MatchPart::default()
            },
            Box::new(|v: &str| Some(words_to_number(&v.to_lowercase()))),
        ),
        Primary::Decimal => {
            let f = format.clone();
            (
                MatchPart {
                    chars: Some(CharSet::Digits),
                    width: format.parse_width.unwrap_or(0),
                    ordinal: format.ordinal,
                    ..MatchPart::default()
                },
                Box::new(move |v: &str| Some(parse_decimal(v, &f))),
            )
        }
        Primary::Sequence => {
            return Err(Error::new("D3130").with("value", format.token.as_str()));
        }
    })
}

/// A number in a decimal picture: its ordinal suffix and grouping separators removed, in one
/// pass, and its digits of the picture's family made ASCII, leaving other characters for
/// `parseInt` to stop at (jsonata-js since its pull request #869; before it, a regular separator
/// other than a comma was kept, and a character below the family made the number NaN).
fn parse_decimal(v: &str, f: &IntegerFormat) -> f64 {
    let mut chars: Vec<char> = v.chars().collect();
    if f.ordinal {
        chars.truncate(chars.len().saturating_sub(2));
    }
    let separators: std::collections::HashSet<char> =
        f.separators.iter().map(|s| s.character).collect();
    let digits: String = chars
        .into_iter()
        .filter(|c| !separators.contains(c))
        .map(|c| {
            let code = c as u32;
            if f.zero_code != 0x30 && (f.zero_code..=f.zero_code + 9).contains(&code) {
                char::from_u32(code - f.zero_code + 0x30).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    js_parse_int(&digits)
}

/// `parseInt(s, 10)`: leading sign and digits, NaN if there are none.
fn js_parse_int(s: &str) -> f64 {
    let s = s.trim_start();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, s.strip_prefix('+').unwrap_or(s)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    sign * digits.parse::<f64>().unwrap_or(f64::NAN)
}

/// `$parseInteger`.
pub(crate) fn parse_integer(value: &str, picture: &str) -> Result<f64> {
    let format = analyse_integer_picture(picture)?;
    let (_, parse) = integer_matcher(&format)?;
    Ok(parse(value).unwrap_or(f64::NAN))
}

// ---------------------------------------------------------------------------------------------
// Calendar arithmetic

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The range of a JavaScript `Date`: 100,000,000 days on either side of 1970. Dates beyond it
/// fail (`$fromMillis`) or are not dates (`$toMillis`); within it every component is exact.
const MAX_MILLIS: f64 = 8.64e15;

/// `Date.UTC(year, monthIndex, day, hours, minutes, seconds, millis)`, with JavaScript's
/// normalization of out-of-range fields and of two-digit years. Like V8, it gives NaN for a
/// time beyond ±8.64e15 ms.
fn date_utc(year: f64, month: f64, day: f64, h: f64, mi: f64, s: f64, ms: f64) -> f64 {
    let time = calendar_millis(year, month, day, h, mi, s, ms);
    if time.abs() > MAX_MILLIS {
        f64::NAN
    } else {
        time
    }
}

/// `date_utc` without the range of `Date`, for the dates that a component of a date in range is
/// counted from, which can lie beyond it (1 January of -271821). NaN for a year beyond
/// ±1,000,000 or a month beyond ±10,000,000, as in V8, which keeps the arithmetic in `i64`.
fn calendar_millis(year: f64, month: f64, day: f64, h: f64, mi: f64, s: f64, ms: f64) -> f64 {
    let fields = [year, month, day, h, mi, s, ms];
    if fields.iter().any(|f| !f.is_finite()) {
        return f64::NAN;
    }
    let mut year = year.trunc();
    if (0.0..=99.0).contains(&year) {
        year += 1900.0;
    }
    let month = month.trunc();
    if year.abs() > 1_000_000.0 || month.abs() > 10_000_000.0 {
        return f64::NAN;
    }
    let (y, m) = (year as i64, month as i64);
    let days =
        days_from_civil(y + m.div_euclid(12), m.rem_euclid(12) + 1, 1) as f64 + day.trunc() - 1.0;
    days * MILLIS_IN_A_DAY
        + h.trunc() * 3_600_000.0
        + mi.trunc() * 60_000.0
        + s.trunc() * 1000.0
        + ms.trunc()
}

/// The UTC fields of a timestamp, like the `getUTC*` methods of `Date`.
#[derive(Debug, Clone, Copy)]
struct Fields {
    millis: f64,
    year: i64,
    month: i64, // 0-11
    day: i64,
    hours: i64,
    minutes: i64,
    seconds: i64,
    ms: i64,
    weekday: i64, // 0 = Sunday
}

fn fields(millis: f64) -> Fields {
    let t = millis.trunc() as i64;
    let days = t.div_euclid(86_400_000);
    let in_day = t.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    Fields {
        millis: t as f64,
        year,
        month: month - 1,
        day,
        hours: in_day / 3_600_000,
        minutes: in_day / 60_000 % 60,
        seconds: in_day / 1000 % 60,
        ms: in_day % 1000,
        weekday: (days + 4).rem_euclid(7),
    }
}

/// ISO 8601: the week (and the month-week in XPath F&O) starts on the Monday of the week that
/// contains the first Thursday.
fn start_of_first_week(year: i64, month: i64) -> f64 {
    let jan1 = calendar_millis(year as f64, month as f64, 1.0, 0.0, 0.0, 0.0, 0.0);
    let mut day = fields(jan1).weekday;
    if day == 0 {
        day = 7;
    }
    if day > 4 {
        jan1 + (8 - day) as f64 * MILLIS_IN_A_DAY
    } else {
        jan1 - (day - 1) as f64 * MILLIS_IN_A_DAY
    }
}

fn next_month(y: i64, m: i64) -> (i64, i64) {
    if m == 11 { (y + 1, 0) } else { (y, m + 1) }
}

fn previous_month(y: i64, m: i64) -> (i64, i64) {
    if m == 0 { (y - 1, 11) } else { (y, m - 1) }
}

fn delta_weeks(start: f64, end: f64) -> f64 {
    (end - start) / (MILLIS_IN_A_DAY * 7.0) + 1.0
}

#[derive(Debug, Clone)]
enum Component {
    Number(f64),
    Text(String),
    None,
}

fn fragment(date: &Fields, component: char) -> Component {
    let n = |v: i64| Component::Number(v as f64);
    match component {
        'Y' => n(date.year),
        'M' => n(date.month + 1),
        'D' => n(date.day),
        'd' => {
            let today = calendar_millis(
                date.year as f64,
                date.month as f64,
                date.day as f64,
                0.0,
                0.0,
                0.0,
                0.0,
            );
            let first = calendar_millis(date.year as f64, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0);
            Component::Number((today - first) / MILLIS_IN_A_DAY + 1.0)
        }
        'F' => n(if date.weekday == 0 { 7 } else { date.weekday }),
        'W' => {
            let start = start_of_first_week(date.year, 0);
            let today = calendar_millis(
                date.year as f64,
                date.month as f64,
                date.day as f64,
                0.0,
                0.0,
                0.0,
                0.0,
            );
            let mut week = delta_weeks(start, today);
            if week > 52.0 {
                if today >= start_of_first_week(date.year + 1, 0) {
                    week = 1.0;
                }
            } else if week < 1.0 {
                week = delta_weeks(start_of_first_week(date.year - 1, 0), today);
            }
            Component::Number(week.floor())
        }
        'w' => {
            let start = start_of_first_week(date.year, date.month);
            let today = calendar_millis(
                date.year as f64,
                date.month as f64,
                date.day as f64,
                0.0,
                0.0,
                0.0,
                0.0,
            );
            let mut week = delta_weeks(start, today);
            if week > 4.0 {
                let (ny, nm) = next_month(date.year, date.month);
                if today >= start_of_first_week(ny, nm) {
                    week = 1.0;
                }
            } else if week < 1.0 {
                let (py, pm) = previous_month(date.year, date.month);
                week = delta_weeks(start_of_first_week(py, pm), today);
            }
            Component::Number(week.floor())
        }
        'X' => {
            let start = start_of_first_week(date.year, 0);
            let end = start_of_first_week(date.year + 1, 0);
            n(if date.millis < start {
                date.year - 1
            } else if date.millis >= end {
                date.year + 1
            } else {
                date.year
            })
        }
        'x' => {
            let start = start_of_first_week(date.year, date.month);
            let (ny, nm) = next_month(date.year, date.month);
            let end = start_of_first_week(ny, nm);
            n(if date.millis < start {
                previous_month(date.year, date.month).1 + 1
            } else if date.millis >= end {
                nm + 1
            } else {
                date.month + 1
            })
        }
        'H' => n(date.hours),
        'h' => {
            let h = date.hours % 12;
            n(if h == 0 { 12 } else { h })
        }
        'P' => Component::Text(if date.hours >= 12 { "pm" } else { "am" }.into()),
        'm' => n(date.minutes),
        's' => n(date.seconds),
        'f' => n(date.ms),
        'C' | 'E' => Component::Text("ISO".into()),
        _ => Component::None,
    }
}

// ---------------------------------------------------------------------------------------------
// Date/time pictures

/// A width modifier, as jsonata-js's `parseInt` reads it: it can be negative (`[Y,1--2]`).
#[derive(Debug, Clone, Default)]
struct Width {
    min: Option<i64>,
    max: Option<i64>,
}

/// The largest width modifier: a component is padded to its minimum width, so a large one would
/// make a long string from a short expression (`D3142`, jsonata-js since its pull request #869).
const MAX_WIDTH_MODIFIER: f64 = 100.0;

/// The most positions times parts `$toMillis` matches: the matcher's time and memory grow with
/// it, and one million is far more than a real timestamp needs (jsonata-js's `maxMatchSize`).
const MAX_MATCH_SIZE: usize = 1_000_000;

/// The length in UTF-16 code units, as jsonata-js counts it.
fn js_length(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[derive(Debug, Clone)]
struct Marker {
    component: char,
    width: Option<Width>,
    presentation1: String,
    presentation2: Option<char>,
    names: Option<Case>,
    integer_format: Option<IntegerFormat>,
    n: i64,
}

#[derive(Debug, Clone)]
enum Part {
    Literal(String),
    Marker(Marker),
}

fn default_presentation(component: char) -> Option<&'static str> {
    Some(match component {
        'Y' | 'M' | 'D' | 'd' | 'W' | 'w' | 'X' | 'x' | 'H' | 'h' | 'f' => "1",
        'F' | 'P' | 'C' | 'E' => "n",
        'm' | 's' => "01",
        'Z' | 'z' => "01:01",
        _ => return None,
    })
}

/// Analyses a date/time picture, and the least number of UTF-16 code units a timestamp needs
/// to match it: each literal whole, and a character for each marker. The whole picture is read,
/// so that an invalid one is an error however short the timestamp.
fn analyse_datetime_picture(picture: &str) -> Result<(Vec<Part>, usize)> {
    let chars: Vec<char> = picture.chars().collect();
    let mut spec: Vec<Part> = Vec::new();
    let mut min_length = 0;
    let add_literal = |spec: &mut Vec<Part>, min_length: &mut usize, start: usize, end: usize| {
        if end > start {
            let literal: String = chars[start..end].iter().collect();
            let literal = literal.replace("]]", "]");
            *min_length += js_length(&literal);
            spec.push(Part::Literal(literal));
        }
    };
    let (mut start, mut pos) = (0, 0);
    while pos < chars.len() {
        if chars[pos] == '[' {
            if chars.get(pos + 1) == Some(&'[') {
                add_literal(&mut spec, &mut min_length, start, pos);
                spec.push(Part::Literal("[".into()));
                min_length += 1;
                pos += 2;
                start = pos;
                continue;
            }
            add_literal(&mut spec, &mut min_length, start, pos);
            start = pos;
            let Some(close) = chars[start..].iter().position(|&c| c == ']') else {
                return Err(Error::new("D3135"));
            };
            pos = start + close;
            let marker: String = chars[start + 1..pos]
                .iter()
                .filter(|c| !c.is_whitespace())
                .collect();
            let marker: Vec<char> = marker.chars().collect();
            let component = marker.first().copied().unwrap_or(' ');
            let mut def = Marker {
                component,
                width: None,
                presentation1: String::new(),
                presentation2: None,
                names: None,
                integer_format: None,
                n: -1,
            };
            let pres_mod: String = match marker.iter().rposition(|&c| c == ',') {
                Some(comma) => {
                    let width_mod: String = marker[comma + 1..].iter().collect();
                    let parse_width = |w: Option<&str>| -> Result<Option<i64>> {
                        Ok(match w {
                            None | Some("*") => None,
                            Some(w) => {
                                let v = js_parse_int(w);
                                if v > MAX_WIDTH_MODIFIER {
                                    return Err(Error::new("D3142").with("value", w));
                                }
                                (!v.is_nan()).then_some(v as i64)
                            }
                        })
                    };
                    let (min, max) = match width_mod.find('-') {
                        None => (Some(width_mod.as_str()), None),
                        Some(d) => (Some(&width_mod[..d]), Some(&width_mod[d + 1..])),
                    };
                    def.width = Some(Width {
                        min: parse_width(min)?,
                        max: parse_width(max)?,
                    });
                    // `substring(1, comma)` swaps its arguments when the marker starts
                    // with the comma: the presentation is then the comma itself.
                    marker[comma.min(1)..comma.max(1)].iter().collect()
                }
                None => marker.iter().skip(1).collect(),
            };
            let pres_chars: Vec<char> = pres_mod.chars().collect();
            if pres_chars.len() == 1 {
                def.presentation1 = pres_mod.clone();
            } else if pres_chars.len() > 1 {
                let last = *pres_chars.last().unwrap_or(&' ');
                if "atco".contains(last) {
                    def.presentation2 = Some(last);
                    def.presentation1 = pres_chars[..pres_chars.len() - 1].iter().collect();
                } else {
                    def.presentation1 = pres_mod.clone();
                }
            } else {
                match default_presentation(component) {
                    Some(p) => def.presentation1 = p.into(),
                    None => return Err(Error::new("D3132").with("value", component.to_string())),
                }
            }
            let p1: Vec<char> = def.presentation1.chars().collect();
            if p1.first() == Some(&'n') {
                def.names = Some(Case::Lower);
            } else if p1.first() == Some(&'N') {
                def.names = Some(if p1.get(1) == Some(&'n') {
                    Case::Title
                } else {
                    Case::Upper
                });
            } else if "YMDdFWwXxHhmsf".contains(component) {
                let mut pattern = def.presentation1.clone();
                if let Some(p2) = def.presentation2 {
                    pattern.push(';');
                    pattern.push(p2);
                }
                let mut format = analyse_integer_picture(&pattern)?;
                if let Some(min) = def.width.as_ref().and_then(|w| w.min)
                    && (format.mandatory_digits as i64) < min
                {
                    format.mandatory_digits = min as usize;
                }
                if component == 'Y' {
                    def.n = -1;
                    if let Some(max) = def.width.as_ref().and_then(|w| w.max) {
                        def.n = max;
                        format.mandatory_digits = max.max(0) as usize;
                    } else {
                        let w = format
                            .mandatory_digits
                            .saturating_add(format.optional_digits);
                        if w >= 2 {
                            def.n = w as i64;
                        }
                    }
                }
                // A number directly after another needs a precise width to be parsed, unless this
                // one cannot start with a digit (words, roman numerals, letters), so there is no
                // doubt where the other ends; fractional seconds are always read as digits
                // (jsonata-js since its pull request #869).
                if (format.primary == Primary::Decimal || component == 'f')
                    && let Some(Part::Marker(prev)) = spec.last_mut()
                    && let Some(prev_format) = &mut prev.integer_format
                {
                    // A year's maximum width, even a negative one, as jsonata-js keeps it.
                    let width = match prev.width.as_ref().and_then(|w| w.max) {
                        Some(max) if prev.component == 'Y' => max,
                        _ => prev_format.mandatory_digits as i64,
                    };
                    prev_format.parse_width = Some(width);
                }
                def.integer_format = Some(format);
            }
            if component == 'Z' || component == 'z' {
                def.integer_format = Some(analyse_integer_picture(&def.presentation1)?);
            }
            spec.push(Part::Marker(def));
            min_length += 1;
            start = pos + 1;
        }
        pos += 1;
    }
    add_literal(&mut spec, &mut min_length, start, pos.min(chars.len()));
    Ok((spec, min_length))
}

fn apply_case(s: &str, case: Option<Case>) -> String {
    match case {
        Some(Case::Upper) => s.to_uppercase(),
        Some(Case::Lower) => s.to_lowercase(),
        _ => s.to_owned(),
    }
}

/// `$fromMillis` / `$now`: formats a timestamp. Fails with `U1001` when a width modifier
/// (`[Y,1000000000]`) asks for more than `max_len` digits, with `D3150` when the timezone
/// does not start with a number, and with `D1001` for NaN or a date beyond the range
/// (jsonata-js writes every component as "NaN" in these cases).
pub(crate) fn from_millis(
    millis: f64,
    picture: Option<&str>,
    timezone: Option<&str>,
    max_len: usize,
) -> Result<String> {
    let (mut offset_hours, mut offset_minutes) = (0.0, 0.0);
    if let Some(tz) = timezone {
        let offset = js_parse_int(tz);
        if offset.is_nan() {
            return Err(Error::new("D3150").with("value", tz));
        }
        // Both parts take the sign: "-0330" is -3 hours and -30 minutes. jsonata-js 2.2.2 floors
        // the hours, so it reads -4 hours and -30 minutes, an hour off (jsonata-js#852).
        offset_minutes = offset % 100.0;
        offset_hours = (offset - offset_minutes) / 100.0;
    }
    let (spec, _) = analyse_datetime_picture(
        picture.unwrap_or("[Y0001]-[M01]-[D01]T[H01]:[m01]:[s01].[f001][Z01:01t]"),
    )?;
    let offset_millis = (60.0 * offset_hours + offset_minutes) * 60_000.0;
    // jsonata-js writes every component of NaN, which `$toMillis` gives for a date that does
    // not exist, or of a date beyond the range as "NaN".
    let local = millis + offset_millis;
    if local.is_nan() || local.abs() > MAX_MILLIS {
        return Err(Error::new("D1001").with("value", crate::value::json_number(millis)));
    }
    let date = fields(local);

    // The text is checked against the size limit as it grows: each marker writes up to 100
    // characters (`[Y,100]`), many times its own length.
    let mut out = String::new();
    for part in &spec {
        let piece = match part {
            Part::Literal(s) => std::borrow::Cow::Borrowed(s.as_str()),
            Part::Marker(m) => {
                if m.integer_format
                    .as_ref()
                    .is_some_and(|f| f.mandatory_digits > max_len)
                {
                    return Err(crate::value::string_too_long(max_len));
                }
                format_component(&date, m, offset_hours, offset_minutes)?.into()
            }
        };
        if out.len() + piece.len() > max_len {
            return Err(crate::value::string_too_long(max_len));
        }
        out.push_str(&piece);
    }
    Ok(out)
}

fn format_component(
    date: &Fields,
    m: &Marker,
    offset_hours: f64,
    offset_minutes: f64,
) -> Result<String> {
    let value = fragment(date, m.component);
    if "YMDdFWwXxHhms".contains(m.component) {
        let Component::Number(mut v) = value else {
            return Ok(String::new());
        };
        if m.component == 'Y' && m.n != -1 {
            v %= 10f64.powi(m.n as i32);
        }
        if let Some(names) = m.names {
            let name = match m.component {
                'M' | 'x' => MONTHS.get(v as usize - 1).copied().unwrap_or(""),
                'F' => DAYS.get(v as usize).copied().unwrap_or(""),
                _ => return Err(Error::new("D3133").with("value", m.component.to_string())),
            };
            let mut name = apply_case(name, Some(names));
            if let Some(max) = m.width.as_ref().and_then(|w| w.max)
                && name.chars().count() as i64 > max
            {
                name = name.chars().take(max.max(0) as usize).collect();
            }
            return Ok(name);
        }
        return format_integer_with(v, m.integer_format.as_ref().expect("integer format"));
    }
    match m.component {
        'f' => match (value, &m.integer_format) {
            (Component::Number(v), Some(format)) if format.primary == Primary::Decimal => {
                format_fraction(v, m)
            }
            (Component::Number(v), Some(format)) => format_integer_with(v, format),
            // A name presentation (`[fn]`) leaves no integer format; jsonata-js then throws a
            // TypeError.
            (Component::Number(_), None) => Err(Error::new("D3133").with("value", "f")),
            _ => Ok(String::new()),
        },
        'Z' | 'z' => {
            // The sign is written once, before the hours and the minutes it applies to
            // (jsonata-js#852): jsonata-js 2.2.2 writes "-0030" as "-1:-30" with `[Z0]`.
            let offset = offset_hours * 100.0 + offset_minutes;
            let format = m.integer_format.as_ref().expect("integer format");
            let mut s = if format.regular {
                format_integer_with(offset.abs(), format)?
            } else {
                match format.mandatory_digits {
                    1 | 2 => {
                        let mut s = format_integer_with(offset_hours.abs(), format)?;
                        if offset_minutes != 0.0 {
                            s.push(':');
                            s.push_str(&format_integer(offset_minutes.abs(), "00", usize::MAX)?);
                        }
                        s
                    }
                    3 | 4 => format_integer_with(offset.abs(), format)?,
                    n => return Err(Error::new("D3134").with("value", n)),
                }
            };
            s.insert(0, if offset < 0.0 { '-' } else { '+' });
            if m.component == 'z' {
                s.insert_str(0, "GMT");
            }
            if offset == 0.0 && m.presentation2 == Some('t') {
                s = "Z".into();
            }
            Ok(s)
        }
        'P' => match value {
            Component::Text(t) => Ok(if m.names == Some(Case::Upper) {
                t.to_uppercase()
            } else {
                t
            }),
            _ => Ok(String::new()),
        },
        _ => match value {
            Component::Text(t) => Ok(t),
            Component::Number(n) => Ok(js_number(n)),
            // jsonata-js writes "undefined" for an unknown component given a presentation.
            Component::None => Err(Error::new("D3132").with("value", m.component.to_string())),
        },
    }
}

/// Fractional seconds in a decimal presentation, as the picture rules give (XPath F&O 3.1
/// §9.8.4.5): the digits of the fraction, padded and cut on the right, so `[f01]` of 33 ms is
/// "03". A single digit (`[f]`, `[f1]`) allows the full precision, insignificant zeros are left
/// out down to the mandatory digits, and the separators of the picture count from the left.
/// jsonata-js 2.2.2 writes the milliseconds as an integer ("33"); this follows its fix
/// (jsonata-js#853, pull request #855).
fn format_fraction(ms: f64, m: &Marker) -> Result<String> {
    let picture: Vec<char> = m.presentation1.chars().collect();
    let (format, min, max) = fraction_format(m)?;
    // `ms` is a whole number below 1,000.
    let mut fraction: Vec<u8> = format!("{:03}", ms as u32).into_bytes();
    while fraction.last() == Some(&b'0') {
        fraction.pop();
    }
    fraction.resize(fraction.len().max(min), b'0');
    fraction.truncate(max);
    let zero = format.zero_code;
    let mut digits: Vec<String> = fraction
        .iter()
        .map(|&d| {
            char::from_u32(zero + u32::from(d - b'0'))
                .unwrap_or('\u{FFFD}')
                .to_string()
        })
        .collect();
    // Each separator goes before the digit at its position, as jsonata-js prepends it (two
    // separators in a row come out in reverse order there too).
    let mut position = 0;
    for &c in &picture {
        if c == '#' || (zero..=zero + 9).contains(&(c as u32)) {
            position += 1;
        } else if let Some(d) = digits.get_mut(position) {
            d.insert(0, c);
        }
    }
    Ok(digits.concat())
}

/// The format of fractional seconds (§9.8.4.5), and the least and most digits they are written
/// with (jsonata-js's `fractionalSecondsFormat`).
fn fraction_format(m: &Marker) -> Result<(IntegerFormat, usize, usize)> {
    // An empty modifier keeps a semicolon of the picture a separator.
    let format = analyse_integer_picture(&format!("{};", m.presentation1))?;
    let mut min = format.mandatory_digits as i64;
    let mut max = min + format.optional_digits as i64;
    if let Some(width) = &m.width {
        if let Some(w) = width.min {
            min = min.max(w);
        }
        max = max.max(min);
        if let Some(w) = width.max {
            max = max.max(w);
        }
    } else if m.presentation1.chars().count() == 1 {
        max = 3;
    }
    Ok((format, min.max(0) as usize, max.max(0) as usize))
}

/// `$toMillis`: parses an ISO 8601 timestamp, or one described by a picture string. `now` is the
/// evaluation's timestamp, used to fill in unspecified leading components.
pub(crate) fn to_millis(timestamp: &str, picture: Option<&str>, now: f64) -> Result<Option<f64>> {
    match picture {
        None => parse_iso8601(timestamp)
            .map(Some)
            .ok_or_else(|| Error::new("D3110").with("value", timestamp)),
        Some(picture) => parse_datetime(timestamp, picture, now),
    }
}

/// Validates and parses `^\d{4}(-[01]\d)?(-[0-3]\d)?(T[0-2]\d:[0-5]\d:[0-5]\d)?(\.\d+)?([+-][0-2]\d:?[0-5]\d|Z)?$`
/// like `Date.parse`. A timestamp without an offset is read as UTC.
fn parse_iso8601(s: &str) -> Option<f64> {
    static ISO8601: LazyLock<regress::Regex> = LazyLock::new(|| {
        regress::Regex::new(
            r"^(\d{4})(?:-([01]\d))?(?:-([0-3]\d))?(?:T([0-2]\d):([0-5]\d):([0-5]\d))?(?:\.(\d+))?(?:([+-])([0-2]\d):?([0-5]\d)|(Z))?$",
        )
        .expect("valid regex")
    });
    let re = &*ISO8601;
    let m = re.find(s)?;
    let group = |i: usize| m.group(i).map(|r| &s[r]);
    let int =
        |i: usize, default: f64| group(i).map_or(default, |g| g.parse::<f64>().unwrap_or(f64::NAN));
    let (year, month, day) = (int(1, 0.0), int(2, 1.0), int(3, 1.0));
    let (h, mi, sec) = (int(4, 0.0), int(5, 0.0), int(6, 0.0));
    if !(1.0..=12.0).contains(&month) || !(1.0..=31.0).contains(&day) || h > 24.0 {
        return Some(f64::NAN);
    }
    let ms = group(7).map_or(0.0, |f| {
        let digits: String = f.chars().chain(std::iter::repeat('0')).take(3).collect();
        digits.parse().unwrap_or(0.0)
    });
    let mut millis = days_from_civil(year as i64, month as i64, day as i64) as f64
        * MILLIS_IN_A_DAY
        + h * 3_600_000.0
        + mi * 60_000.0
        + sec * 1000.0
        + ms;
    if let Some(sign) = group(8) {
        let offset = (int(9, 0.0) * 60.0 + int(10, 0.0)) * 60_000.0;
        millis += if sign == "-" { offset } else { -offset };
    }
    Some(millis)
}

/// A character as a regular expression with the `i` flag compares it, as jsonata-js's
/// `canonicalize` does per UTF-16 code unit: in upper case, unless that would change its length
/// or make a character outside ASCII an ASCII one. A character outside the Basic Multilingual
/// Plane is two code units there, which have no case.
fn canonical(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    if c as u32 > 0xFFFF {
        return c;
    }
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) if (u as u32) <= 0xFFFF && !u.is_ascii() => u,
        _ => c,
    }
}

fn canonicalize(s: &str) -> Vec<char> {
    s.chars().map(canonical).collect()
}

/// The words a number in words is made of, in upper case and grouped by their first letter, in
/// the order jsonata-js tries them: its `wordValues`' keys, then "and".
fn words_by_initial() -> &'static std::collections::HashMap<char, Vec<Vec<char>>> {
    static WORDS: std::sync::OnceLock<std::collections::HashMap<char, Vec<Vec<char>>>> =
        std::sync::OnceLock::new();
    WORDS.get_or_init(|| {
        let mut map: std::collections::HashMap<char, Vec<Vec<char>>> =
            std::collections::HashMap::new();
        let words = word_values().into_iter().map(|(w, _)| w);
        for word in words.chain(std::iter::once("and".to_owned())) {
            let upper: Vec<char> = word.to_uppercase().chars().collect();
            map.entry(upper[0]).or_default().push(upper);
        }
        map
    })
}

/// The rest of the picture cannot be matched from this part at this position.
const FAILED: u8 = 1;
/// A number in words has already been continued from this position.
const VISITED: u8 = 2;

/// Where a part can end, given where it starts, in the order a backtracking regular expression
/// would try them. Only one cursor per part is in use at a time.
#[derive(Default)]
struct Cursor {
    // A number in words: a depth-first search through the words, `depth` levels deep.
    depth: usize,
    positions: Vec<usize>,
    options: Vec<&'static [Vec<char>]>,
    separator: Vec<Option<char>>,
    tried: Vec<usize>,
    // Any other part: one end, or a range tried from the highest to the lowest.
    low: i64,
    high: i64,
    key: Option<usize>,
    first: Option<usize>,
}

/// The state of one match of a timestamp against a picture's parts.
struct Matching<'a> {
    parts: &'a [MatchPart],
    text: Vec<char>,
    stride: usize,
    memo: Vec<u8>,
    /// For each part that ends in a range of positions, the lowest one tried, plus one.
    lowest_tried: Vec<u32>,
    run_ends: std::collections::HashMap<CharSet, Vec<usize>>,
    /// For each literal part, where it occurs, found the first time it is tested.
    literal_starts: Vec<Option<Vec<bool>>>,
}

impl Matching<'_> {
    fn starts_with(&self, s: &[char], pos: usize) -> bool {
        self.text.get(pos..pos + s.len()) == Some(s)
    }

    /// Whether the literal of part `index` occurs at `pos`: every start is found the first time,
    /// in one pass over the text (Knuth-Morris-Pratt), so each test takes constant time however
    /// long the literal is.
    fn literal_at(&mut self, index: usize, literal: &[char], pos: usize) -> bool {
        if literal.is_empty() {
            return true;
        }
        if self.literal_starts[index].is_none() {
            let mut overlap = vec![0usize; literal.len()];
            let mut k = 0;
            for i in 1..literal.len() {
                while k > 0 && literal[i] != literal[k] {
                    k = overlap[k - 1];
                }
                if literal[i] == literal[k] {
                    k += 1;
                }
                overlap[i] = k;
            }
            let mut starts = vec![false; self.stride];
            let mut k = 0;
            for (i, &c) in self.text.iter().enumerate() {
                while k > 0 && c != literal[k] {
                    k = overlap[k - 1];
                }
                if c == literal[k] {
                    k += 1;
                }
                if k == literal.len() {
                    starts[i + 1 - k] = true;
                    k = overlap[k - 1];
                }
            }
            self.literal_starts[index] = Some(starts);
        }
        self.literal_starts[index]
            .as_ref()
            .is_some_and(|starts| starts[pos])
    }

    /// The position after the run of characters of `set` that starts at `pos`.
    fn run_end(&mut self, set: CharSet, pos: usize) -> usize {
        let text = &self.text;
        let stride = self.stride;
        self.run_ends.entry(set).or_insert_with(|| {
            let mut ends = vec![0; stride];
            ends[text.len()] = text.len();
            for i in (0..text.len()).rev() {
                ends[i] = if set.contains(text[i]) {
                    ends[i + 1]
                } else {
                    i
                };
            }
            ends
        })[pos]
    }

    fn push_word(&self, cursor: &mut Cursor, pos: usize) {
        let depth = cursor.depth;
        let (options, separator): (&'static [Vec<char>], Option<char>) = match self.text.get(pos) {
            Some(c) => match words_by_initial().get(c) {
                Some(words) => (words, None),
                None => (&[], "-, ".contains(*c).then_some(*c)),
            },
            None => (&[], None),
        };
        if cursor.positions.len() <= depth {
            cursor.positions.push(0);
            cursor.options.push(&[]);
            cursor.separator.push(None);
            cursor.tried.push(0);
        }
        cursor.positions[depth] = pos;
        cursor.options[depth] = options;
        cursor.separator[depth] = separator;
        cursor.tried[depth] = 0;
        cursor.depth += 1;
    }

    fn start(&mut self, index: usize, cursor: &mut Cursor, pos: usize) {
        let part = &self.parts[index];
        let base = index * self.stride;
        if part.words {
            cursor.depth = 0;
            if self.memo[base + pos] & VISITED == 0 {
                self.push_word(cursor, pos);
            }
            return;
        }
        cursor.low = 0;
        cursor.high = -1;
        cursor.key = None;
        cursor.first = None;
        let only = |cursor: &mut Cursor, end: usize| {
            cursor.low = end as i64;
            cursor.high = end as i64;
        };
        if let Some(literal) = &part.literal {
            if self.literal_at(index, literal, pos) {
                only(cursor, pos + literal.len());
            }
            return;
        }
        let length = self.text.len();
        let chars = part.chars.unwrap_or(CharSet::Digits);
        let mut from = pos;
        for set in part.prefix {
            if from == length || !set.contains(self.text[from]) {
                return;
            }
            from += 1;
        }
        if let Some(separator) = &part.separator {
            // The separator is none of the characters before it, so they all come before it.
            let head = self.run_end(chars, from);
            if head == from || !self.starts_with(separator, head) {
                return;
            }
            from = head + separator.len();
        }
        let mut end = self.run_end(chars, from);
        if end == from {
            return;
        }
        if part.width != 0 {
            // A width that is no positive whole number (`[Y,1--2]`) matches nothing.
            if part.width < 0 || from + part.width as usize > end {
                return;
            }
            end = from + part.width as usize;
        }
        if part.minutes
            && self.text.get(end) == Some(&':')
            && self.run_end(CharSet::Digits, end + 1) >= end + 3
        {
            // The minutes of a time zone are tried before the hours alone, as `(?::[0-9]{2})?`.
            cursor.first = Some(end + 3);
        }
        if part.width == 0 && !part.ordinal {
            // Ranges that end at the same position share the lowest position tried, since the
            // higher ones would already have failed.
            let key = base + end;
            cursor.key = Some(key);
            cursor.low = from as i64 + 1;
            let lowest = self.lowest_tried[key];
            cursor.high = if lowest == 0 {
                end as i64
            } else {
                (end as i64).min(i64::from(lowest) - 2)
            };
            return;
        }
        if !part.ordinal {
            only(cursor, end);
        } else if self.text.get(end..end + 2).is_some_and(|s| {
            ["TH", "ST", "ND", "RD"]
                .iter()
                .any(|o| o.chars().eq(s.iter().copied()))
        }) {
            only(cursor, end + 2);
        }
    }

    fn next(&mut self, index: usize, cursor: &mut Cursor) -> Option<usize> {
        let part = &self.parts[index];
        if part.words {
            // As `(?:word1|word2|…|[-, ])+`, which tries to continue with each word in turn
            // before ending where it is; each position is searched from once.
            let base = index * self.stride;
            while cursor.depth > 0 {
                let top = cursor.depth - 1;
                let at = cursor.positions[top];
                let options = cursor.options[top];
                let count = options.len() + usize::from(cursor.separator[top].is_some());
                if cursor.tried[top] < count {
                    let tried = cursor.tried[top];
                    cursor.tried[top] += 1;
                    let word: &[char] = match options.get(tried) {
                        Some(word) => word,
                        None => {
                            std::slice::from_ref(cursor.separator[top].as_ref().expect("separator"))
                        }
                    };
                    let pos = at + word.len();
                    if self.starts_with(word, at) && self.memo[base + pos] & VISITED == 0 {
                        self.memo[base + pos] |= VISITED;
                        self.push_word(cursor, pos);
                    }
                } else {
                    cursor.depth -= 1;
                    if top > 0 {
                        // A number has at least one word.
                        return Some(at);
                    }
                }
            }
            return None;
        }
        if let Some(end) = cursor.first.take() {
            return Some(end);
        }
        if cursor.high < cursor.low {
            return None;
        }
        if let Some(key) = cursor.key {
            self.lowest_tried[key] = (cursor.high + 1) as u32;
        }
        let end = cursor.high as usize;
        cursor.high -= 1;
        Some(end)
    }
}

/// Matches a timestamp against the parts of a picture, finding the match of the regular
/// expression made of a capturing group per part (jsonata-js's `matchParts`, since its pull
/// request #869). Like a backtracking engine, it tries each part's possible ends in turn, in the
/// same order, but never tries the rest of the parts from the same part and position twice: its
/// time is the timestamp's length times the number of parts, however the picture is built, where
/// a regular expression can take time exponential in the parts that match the same characters
/// (jsonata-js#833). The text of each part, or `None` if the timestamp does not match.
fn match_parts(parts: &[MatchPart], timestamp: &str) -> Option<Vec<String>> {
    let original: Vec<char> = timestamp.chars().collect();
    let count = parts.len();
    if count == 0 {
        return original.is_empty().then(Vec::new);
    }
    let text = canonicalize(timestamp);
    let length = text.len();
    let stride = length + 1;
    let mut m = Matching {
        parts,
        text,
        stride,
        memo: vec![0; stride * count],
        lowest_tried: vec![0; stride * count],
        run_ends: std::collections::HashMap::new(),
        literal_starts: vec![None; count],
    };
    let mut cursors: Vec<Cursor> = (0..count).map(|_| Cursor::default()).collect();
    let mut starts = vec![0usize; count];
    m.start(0, &mut cursors[0], 0);
    let mut depth = 1;
    while depth > 0 {
        let index = depth - 1;
        match m.next(index, &mut cursors[index]) {
            None => {
                m.memo[index * stride + starts[index]] |= FAILED;
                depth -= 1;
            }
            Some(end) if index == count - 1 => {
                if end == length {
                    let mut out = Vec::with_capacity(count);
                    for i in 0..count {
                        let to = if i + 1 < count { starts[i + 1] } else { length };
                        out.push(original[starts[i]..to].iter().collect());
                    }
                    return Some(out);
                }
            }
            Some(end) => {
                if m.memo[(index + 1) * stride + end] & FAILED == 0 {
                    starts[index + 1] = end;
                    m.start(index + 1, &mut cursors[index + 1], end);
                    depth = index + 2;
                }
            }
        }
    }
    None
}

/// The component a part of a picture gives and its parser; none for a literal.
type ComponentParser = Option<(char, Parser)>;

/// The matcher and parser of each part of a date/time picture (jsonata-js's `generateMatcher`).
fn datetime_matcher(spec: &[Part]) -> Result<Vec<(MatchPart, ComponentParser)>> {
    let mut out = Vec::with_capacity(spec.len());
    for part in spec {
        out.push(match part {
            Part::Literal(s) => (
                MatchPart {
                    literal: Some(canonicalize(s)),
                    ..MatchPart::default()
                },
                None,
            ),
            Part::Marker(m) if m.component == 'Z' || m.component == 'z' => {
                let format = m.integer_format.as_ref().expect("integer format");
                let separator = format.regular.then(|| format.separators[0].clone());
                let mut matcher = MatchPart {
                    prefix: if m.component == 'z' {
                        &["G", "M", "T", "-+"]
                    } else {
                        &["-+"]
                    },
                    chars: Some(CharSet::Digits),
                    separator: separator.as_ref().map(|s| vec![canonical(s.character)]),
                    minutes: separator.is_none() && format.mandatory_digits <= 2,
                    ..MatchPart::default()
                };
                // Before another number in digits, the digits after any separator are fixed when
                // the offset is always written with that many: the minutes after a separator,
                // the hours of `[Z01]`, the hours and minutes of `[Z0001]`.
                if format.parse_width.is_some_and(|w| w != 0) {
                    if let Some(sep) = &separator {
                        matcher.width = sep.position as i64;
                    } else if matches!(format.mandatory_digits, 2 | 4) {
                        matcher.width = format.mandatory_digits as i64;
                    }
                }
                let is_z = m.component == 'z';
                let separator = separator.map(|s| s.character);
                // The sign applies to the hours and the minutes, also of "-00:30", and the
                // minutes are the last two digits of "-0330" or "-330". jsonata-js 2.2.2 reads
                // the sign with the hours only and takes the first three characters as hours
                // (jsonata-js#852, pull request #856).
                let parse: Parser = Box::new(move |v: &str| {
                    let v = if is_z { &v[3.min(v.len())..] } else { v };
                    let sign = if v.starts_with('-') { -1.0 } else { 1.0 };
                    let v = v.get(1..).unwrap_or_default();
                    let split = match separator {
                        // The separator can be the sign, and is matched case-insensitively.
                        Some(sep) => v
                            .char_indices()
                            .find(|&(_, c)| canonical(c) == canonical(sep))
                            .map(|(i, c)| (i, c.len_utf8())),
                        None => v.find(':').map(|i| (i, 1)),
                    };
                    let (hours, minutes) = match split {
                        Some((i, len)) => (js_parse_int(&v[..i]), js_parse_int(&v[i + len..])),
                        None if v.len() <= 2 => (js_parse_int(v), 0.0),
                        None => (
                            js_parse_int(&v[..v.len() - 2]),
                            js_parse_int(&v[v.len() - 2..]),
                        ),
                    };
                    Some(sign * (hours * 60.0 + minutes))
                });
                (matcher, Some((m.component, parse)))
            }
            Part::Marker(m) if m.component == 'f' => {
                let mut matcher = MatchPart {
                    chars: Some(CharSet::Digits),
                    ..MatchPart::default()
                };
                // Before another number in digits, the digits are fixed when the fraction is
                // always written with that many.
                if let Some(format) = &m.integer_format
                    && format.parse_width.is_some_and(|w| w != 0)
                    && format.primary == Primary::Decimal
                {
                    let (_, min, max) = fraction_format(m)?;
                    if min == max {
                        matcher.width = min as i64;
                    }
                }
                let parse: Parser = Box::new(|v: &str| {
                    let digits: String = v.chars().take(3).collect();
                    Some(format!("0.{digits}").parse::<f64>().unwrap_or(f64::NAN) * 1000.0)
                });
                (matcher, Some(('f', parse)))
            }
            Part::Marker(m) => match &m.integer_format {
                Some(format) => {
                    let (matcher, parse) = integer_matcher(format)?;
                    (matcher, Some((m.component, parse)))
                }
                None => {
                    // A name, read in any case, as `[MN]` and `[Mn]` ask (jsonata-js's pull
                    // request #875, not merged yet; 2.3.0 reads only "April").
                    let max = m.width.as_ref().and_then(|w| w.max).filter(|&w| w != 0);
                    let key = move |name: &str| -> Vec<char> {
                        let name = match max {
                            Some(max) => name.chars().take(max.max(0) as usize).collect(),
                            None => name.to_owned(),
                        };
                        canonicalize(&name)
                    };
                    let lookup: Vec<(Vec<char>, f64)> = match m.component {
                        'M' | 'x' => MONTHS
                            .iter()
                            .enumerate()
                            .map(|(i, n)| (key(n), (i + 1) as f64))
                            .collect(),
                        'F' => DAYS
                            .iter()
                            .enumerate()
                            .skip(1)
                            .map(|(i, n)| (key(n), i as f64))
                            .collect(),
                        'P' => vec![(canonicalize("am"), 0.0), (canonicalize("pm"), 1.0)],
                        c => return Err(Error::new("D3133").with("value", c.to_string())),
                    };
                    let parse: Parser = Box::new(move |v: &str| {
                        let v = canonicalize(v);
                        lookup.iter().find(|(k, _)| *k == v).map(|(_, n)| *n)
                    });
                    (
                        MatchPart {
                            chars: Some(CharSet::Letters),
                            ..MatchPart::default()
                        },
                        Some((m.component, parse)),
                    )
                }
            },
        });
    }
    Ok(out)
}

fn parse_datetime(timestamp: &str, picture: &str, now: f64) -> Result<Option<f64>> {
    let (spec, min_length) = analyse_datetime_picture(picture)?;
    // Too short a timestamp cannot match, and too long a one would take too long to match.
    let max_length = js_length(timestamp);
    if min_length > max_length || spec.len().saturating_mul(max_length + 1) > MAX_MATCH_SIZE {
        return Ok(None);
    }
    let (parts, parsers): (Vec<MatchPart>, Vec<ComponentParser>) =
        datetime_matcher(&spec)?.into_iter().unzip();
    let Some(info) = match_parts(&parts, timestamp) else {
        return Ok(None);
    };

    // `None` is a component jsonata-js sets to `undefined`: a name it does not know.
    let mut components: std::collections::HashMap<char, Option<f64>> =
        std::collections::HashMap::new();
    for (parser, text) in parsers.iter().zip(&info) {
        if let Some((component, parse)) = parser {
            components.insert(*component, parse(text));
        }
    }
    if components.is_empty() {
        return Ok(None);
    }
    let set = |components: &std::collections::HashMap<char, Option<f64>>, c: char| {
        components
            .get(&c)
            .copied()
            .flatten()
            .is_some_and(|v| v != 0.0 && !v.is_nan())
    };

    let mut mask = 0u32;
    for c in "YXMxWwdD".chars() {
        mask = (mask << 1) + u32::from(set(&components, c));
    }
    let is_type = |ty: u32, mask: u32| (!ty & mask) == 0 && (ty & mask) != 0;
    let date_mask = mask;
    let date_a = is_type(161, date_mask);
    let date_b = !date_a && is_type(130, date_mask);
    let date_c = is_type(84, date_mask);
    let date_d = !date_c && is_type(72, date_mask);
    let mut mask = 0u32;
    for c in "PHhmsf".chars() {
        mask = (mask << 1) + u32::from(set(&components, c));
    }
    let time_a = is_type(23, mask);
    let time_b = !time_a && is_type(47, mask);

    let date_comps = if date_b {
        "YD"
    } else if date_c {
        "XxwF"
    } else if date_d {
        "XWF"
    } else {
        "YMD"
    };
    let time_comps = if time_b { "Phmsf" } else { "Hmsf" };
    let now_fields = fields(now);
    let (mut start_specified, mut end_specified) = (false, false);
    for part in date_comps.chars().chain(time_comps.chars()) {
        if components.get(&part).copied().flatten().is_some() {
            start_specified = true;
            if end_specified {
                return Err(Error::new("D3136"));
            }
        } else if start_specified {
            // Past the specified block: default to zero (one for month and day).
            components.insert(part, Some(if "MDd".contains(part) { 1.0 } else { 0.0 }));
            end_specified = true;
        } else {
            // Before it: default to the current timestamp's component.
            let value = match fragment(&now_fields, part) {
                Component::Number(v) => Some(v),
                _ => None,
            };
            components.insert(part, value);
        }
    }

    let get = |c: char| components.get(&c).copied().flatten().unwrap_or(0.0);
    let mut month = get('M');
    month = if month > 0.0 { month - 1.0 } else { 0.0 };
    let mut day = get('D');
    if date_b {
        let first = date_utc(get('Y'), 0.0, 1.0, 0.0, 0.0, 0.0, 0.0);
        let derived = fields(first + (get('d') - 1.0) * MILLIS_IN_A_DAY);
        month = derived.month as f64;
        day = derived.day as f64;
    }
    if date_c || date_d {
        return Err(Error::new("D3136"));
    }
    let mut hours = get('H');
    if time_b {
        hours = if get('h') == 12.0 { 0.0 } else { get('h') };
        if get('P') == 1.0 {
            hours += 12.0;
        }
    }
    let mut millis = date_utc(get('Y'), month, day, hours, get('m'), get('s'), get('f'));
    let tz = [get('Z'), get('z')]
        .into_iter()
        .find(|v| *v != 0.0 && !v.is_nan())
        .unwrap_or(0.0);
    if tz != 0.0 {
        millis -= tz * 60_000.0;
    }
    // jsonata-js checks the range before the offset, so it can return a date that
    // `$fromMillis` rejects.
    Ok(Some(if millis.abs() > MAX_MILLIS {
        f64::NAN
    } else {
        millis
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The separators inserted one by one, as jsonata-js does.
    fn group_by_inserting(digits: &[char], format: &IntegerFormat) -> Vec<char> {
        let mut digits = digits.to_vec();
        if format.regular {
            let sep = &format.separators[0];
            let n = (digits.len() - 1) / sep.position;
            for ii in (1..=n).rev() {
                let pos = digits.len() - ii * sep.position;
                digits.insert(pos, sep.character);
            }
        } else {
            for sep in format.separators.iter().rev() {
                let pos = digits.len().saturating_sub(sep.position);
                digits.insert(pos, sep.character);
            }
        }
        digits
    }

    #[test]
    fn grouping_in_one_pass_matches_inserting() {
        let pictures = [
            "0",
            "#,##0",
            "0,000",
            "#.##0",
            "0;0",
            "0a0b0",
            "a0",
            "0a",
            "ab0",
            "0ab",
            "a,b,0",
            "0,0,0",
            "#,#00,0",
            "00,,00",
            "w,x,y,z,0",
            "#,##,##0",
            "0a00b000c",
        ];
        for picture in pictures {
            let format = analyse_integer_picture(picture).unwrap();
            for len in 1..12 {
                let digits: Vec<char> = "123456789012".chars().take(len).collect();
                assert_eq!(
                    group_digits(&digits, &format),
                    group_by_inserting(&digits, &format),
                    "{picture} on {len} digits"
                );
            }
        }
    }

    #[test]
    fn integers_in_words_roman_and_letters() {
        assert_eq!(
            format_integer(123.0, "w", usize::MAX).unwrap(),
            "one hundred and twenty-three"
        );
        assert_eq!(
            format_integer(21.0, "Ww;o", usize::MAX).unwrap(),
            "Twenty-First"
        );
        assert_eq!(format_integer(1999.0, "I", usize::MAX).unwrap(), "MCMXCIX");
        assert_eq!(format_integer(28.0, "A", usize::MAX).unwrap(), "AB");
        // jsonata-js's floating-point arithmetic, past u64::MAX.
        assert_eq!(
            format_integer(1e21, "a", usize::MAX).unwrap(),
            "olzyyselzqowgem"
        );
        assert_eq!(
            format_integer(1234567.0, "#,##0", usize::MAX).unwrap(),
            "1,234,567"
        );
        assert_eq!(format_integer(3.0, "1;o", usize::MAX).unwrap(), "3rd");
        // From 1e21, JavaScript's exponent notation written out, as jsonata-js#864 does.
        for (value, picture, digits) in [
            (1e21, "#,##0", "1,000,000,000,000,000,000,000"),
            (-1e21, "0", "-1000000000000000000000"),
            (123456789012345678901234.0, "0", "123456789012345690000000"),
            (12345678901234567000.0, "0", "12345678901234567000"),
            (2e22, "1;o", "20000000000000000000000th"),
        ] {
            assert_eq!(
                format_integer(value, picture, usize::MAX).unwrap(),
                digits,
                "{value}"
            );
        }
        assert_eq!(
            format_integer(1.5e21, "\u{660}", usize::MAX).unwrap(),
            format!("\u{661}\u{665}{}", "\u{660}".repeat(20))
        );
        assert_eq!(
            format_integer(1e300, "0", usize::MAX).unwrap(),
            format!("1{}", "0".repeat(300))
        );
        assert_eq!(
            format_integer(f64::INFINITY, "0", usize::MAX).unwrap(),
            "Infinity"
        );
        assert_eq!(
            parse_integer("one hundred and twenty-three", "w").unwrap(),
            123.0
        );
        assert_eq!(parse_integer("MCMXCIX", "I").unwrap(), 1999.0);
        assert_eq!(parse_integer("1,234,567", "#,##0").unwrap(), 1_234_567.0);
    }

    #[test]
    fn calendar_round_trip() {
        for days in [-719_468, -1, 0, 1, 10_957, 18_000, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(
            date_utc(2018.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0),
            1_514_764_800_000.0
        );
    }

    #[test]
    fn formats_and_parses_timestamps() {
        assert_eq!(
            from_millis(1_514_764_800_000.0, None, None, usize::MAX).unwrap(),
            "2018-01-01T00:00:00.000Z"
        );
        assert_eq!(
            from_millis(
                1_521_801_216_617.0,
                Some("[D1o] [MNn] [Y]"),
                None,
                usize::MAX
            )
            .unwrap(),
            "23rd March 2018"
        );
        assert_eq!(
            parse_iso8601("2018-01-01T00:00:00.000Z"),
            Some(1_514_764_800_000.0)
        );
        assert_eq!(parse_iso8601("2018"), Some(1_514_764_800_000.0));
        assert_eq!(
            to_millis("23rd March 2018", Some("[D1o] [MNn] [Y]"), 0.0).unwrap(),
            Some(1_521_763_200_000.0)
        );
    }
}
