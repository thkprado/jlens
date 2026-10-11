//! Date/time and integer picture strings (XPath F&O 3.1 `format-dateTime`, `format-integer`).
//! Ported from jsonata-js `src/datetime.js` (MIT).
//!
//! Calendar arithmetic uses the proleptic Gregorian calendar in UTC, like JavaScript's `Date`
//! with its `UTC` methods, so no time-zone database is needed.

use crate::error::{Error, Result};
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
    let splitter = regress::Regex::new(r",\s|\sand\s|[\s\\-]").expect("valid regex");
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
    parse_width: Option<usize>,
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
    for i in 1..=indexes.len() {
        if !indexes.contains(&(i * factor)) {
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
            let mut digits: Vec<char> = js_number(value).chars().collect();
            if digits.len() < format.mandatory_digits {
                let pad = format.mandatory_digits - digits.len();
                digits.splice(0..0, std::iter::repeat_n('0', pad));
            }
            if format.zero_code != 0x30 {
                for d in &mut digits {
                    *d = char::from_u32(*d as u32 + format.zero_code - 0x30).unwrap_or(*d);
                }
            }
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
            let mut s: String = digits.into_iter().collect();
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

/// `$formatInteger`.
/// Fails with `U1001` when the result would exceed `max_len` bytes.
pub(crate) fn format_integer(value: f64, picture: &str, max_len: usize) -> Result<String> {
    let format = analyse_integer_picture(picture)?;
    // A Roman numeral takes an `M` per thousand: `1e21` would need 10^18 bytes.
    if format.primary == Primary::Roman && value.abs() / 1000.0 > max_len as f64 {
        return Err(crate::value::string_too_long(max_len));
    }
    format_integer_with(value.floor(), &format)
}

type Parser = Box<dyn Fn(&str) -> f64>;

/// A regular expression fragment and parser for one integer picture.
fn integer_matcher(format: &IntegerFormat) -> Result<(String, Parser)> {
    let upper = format.case == Case::Upper;
    Ok(match format.primary {
        Primary::Letters => (
            if upper { "[A-Z]+" } else { "[a-z]+" }.into(),
            Box::new(move |v: &str| letters_to_decimal(v, if upper { 'A' } else { 'a' })),
        ),
        Primary::Roman => (
            if upper { "[MDCLXVI]+" } else { "[mdclxvi]+" }.into(),
            Box::new(move |v: &str| roman_to_decimal(&v.to_uppercase())),
        ),
        Primary::Words => {
            let mut alternatives: Vec<String> = word_values().into_iter().map(|(k, _)| k).collect();
            alternatives.push("and".into());
            alternatives.push("[\\-, ]".into());
            (
                format!("(?:{})+", alternatives.join("|")),
                Box::new(|v: &str| words_to_number(&v.to_lowercase())),
            )
        }
        Primary::Decimal => {
            let mut regex = "[0-9]".to_owned();
            match format.parse_width {
                Some(w) => regex.push_str(&format!("{{{w}}}")),
                None => regex.push('+'),
            }
            if format.ordinal {
                regex.push_str("(?:th|st|nd|rd)");
            }
            let f = format.clone();
            (
                regex,
                Box::new(move |v: &str| {
                    let mut digits: String = if f.ordinal {
                        v.chars()
                            .take(v.chars().count().saturating_sub(2))
                            .collect()
                    } else {
                        v.to_owned()
                    };
                    if f.regular {
                        digits = digits.replace(',', "");
                    } else {
                        for sep in &f.separators {
                            digits = digits.replace(sep.character, "");
                        }
                    }
                    if f.zero_code != 0x30 {
                        digits = digits
                            .chars()
                            // A character below the digit family is no digit: it ends the
                            // number (jsonata-js throws a `RangeError`).
                            .map(|c| {
                                (c as u32 + 0x30)
                                    .checked_sub(f.zero_code)
                                    .and_then(char::from_u32)
                                    .unwrap_or(char::REPLACEMENT_CHARACTER)
                            })
                            .collect();
                    }
                    js_parse_int(&digits)
                }),
            )
        }
        Primary::Sequence => {
            return Err(Error::new("D3130").with("value", format.token.as_str()));
        }
    })
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
    Ok(parse(value))
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

/// `Date.UTC(year, monthIndex, day, hours, minutes, seconds, millis)`, with JavaScript's
/// normalization of out-of-range fields and of two-digit years.
fn date_utc(year: f64, month: f64, day: f64, h: f64, mi: f64, s: f64, ms: f64) -> f64 {
    let fields = [year, month, day, h, mi, s, ms];
    if fields.iter().any(|f| !f.is_finite()) {
        return f64::NAN;
    }
    let mut y = year.trunc() as i64;
    if (0..=99).contains(&y) {
        y += 1900;
    }
    let m = month.trunc() as i64;
    y += m.div_euclid(12);
    let m = m.rem_euclid(12);
    let days = days_from_civil(y, m + 1, 1) + day.trunc() as i64 - 1;
    days as f64 * MILLIS_IN_A_DAY
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
    let jan1 = date_utc(year as f64, month as f64, 1.0, 0.0, 0.0, 0.0, 0.0);
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
            let today = date_utc(
                date.year as f64,
                date.month as f64,
                date.day as f64,
                0.0,
                0.0,
                0.0,
                0.0,
            );
            let first = date_utc(date.year as f64, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0);
            Component::Number((today - first) / MILLIS_IN_A_DAY + 1.0)
        }
        'F' => n(if date.weekday == 0 { 7 } else { date.weekday }),
        'W' => {
            let start = start_of_first_week(date.year, 0);
            let today = date_utc(
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
            let today = date_utc(
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

#[derive(Debug, Clone, Default)]
struct Width {
    min: Option<usize>,
    max: Option<usize>,
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

fn analyse_datetime_picture(picture: &str) -> Result<Vec<Part>> {
    let chars: Vec<char> = picture.chars().collect();
    let mut spec: Vec<Part> = Vec::new();
    let add_literal = |spec: &mut Vec<Part>, start: usize, end: usize| {
        if end > start {
            let literal: String = chars[start..end].iter().collect();
            spec.push(Part::Literal(literal.replace("]]", "]")));
        }
    };
    let (mut start, mut pos) = (0, 0);
    while pos < chars.len() {
        if chars[pos] == '[' {
            if chars.get(pos + 1) == Some(&'[') {
                add_literal(&mut spec, start, pos);
                spec.push(Part::Literal("[".into()));
                pos += 2;
                start = pos;
                continue;
            }
            add_literal(&mut spec, start, pos);
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
                    let parse_width = |w: Option<&str>| -> Option<usize> {
                        match w {
                            None | Some("*") => None,
                            Some(w) => {
                                let v = js_parse_int(w);
                                (!v.is_nan()).then_some(v as usize)
                            }
                        }
                    };
                    let (min, max) = match width_mod.find('-') {
                        None => (Some(width_mod.as_str()), None),
                        Some(d) => (Some(&width_mod[..d]), Some(&width_mod[d + 1..])),
                    };
                    def.width = Some(Width {
                        min: parse_width(min),
                        max: parse_width(max),
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
                    && format.mandatory_digits < min
                {
                    format.mandatory_digits = min;
                }
                if component == 'Y' {
                    def.n = -1;
                    if let Some(max) = def.width.as_ref().and_then(|w| w.max) {
                        def.n = max as i64;
                        format.mandatory_digits = max;
                    } else {
                        let w = format
                            .mandatory_digits
                            .saturating_add(format.optional_digits);
                        if w >= 2 {
                            def.n = w as i64;
                        }
                    }
                }
                // An integer directly after another needs a precise width to be parsed.
                if let Some(Part::Marker(prev)) = spec.last_mut()
                    && let Some(prev_format) = &mut prev.integer_format
                {
                    prev_format.parse_width = Some(prev_format.mandatory_digits);
                }
                def.integer_format = Some(format);
            }
            if component == 'Z' || component == 'z' {
                def.integer_format = Some(analyse_integer_picture(&def.presentation1)?);
            }
            spec.push(Part::Marker(def));
            start = pos + 1;
        }
        pos += 1;
    }
    add_literal(&mut spec, start, pos.min(chars.len()));
    Ok(spec)
}

fn apply_case(s: &str, case: Option<Case>) -> String {
    match case {
        Some(Case::Upper) => s.to_uppercase(),
        Some(Case::Lower) => s.to_lowercase(),
        _ => s.to_owned(),
    }
}

/// `$fromMillis` / `$now`: formats a timestamp. Fails with `U1001` when a width modifier
/// (`[Y,1000000000]`) asks for more than `max_len` digits.
pub(crate) fn from_millis(
    millis: f64,
    picture: Option<&str>,
    timezone: Option<&str>,
    max_len: usize,
) -> Result<String> {
    let (mut offset_hours, mut offset_minutes) = (0.0, 0.0);
    if let Some(tz) = timezone {
        let offset = js_parse_int(tz);
        offset_hours = (offset / 100.0).floor();
        offset_minutes = offset % 100.0;
    }
    let spec = analyse_datetime_picture(
        picture.unwrap_or("[Y0001]-[M01]-[D01]T[H01]:[m01]:[s01].[f001][Z01:01t]"),
    )?;
    let offset_millis = (60.0 * offset_hours + offset_minutes) * 60_000.0;
    let date = fields(millis + offset_millis);

    let mut out = String::new();
    for part in &spec {
        match part {
            Part::Literal(s) => out.push_str(s),
            Part::Marker(m) => {
                if m.integer_format
                    .as_ref()
                    .is_some_and(|f| f.mandatory_digits > max_len)
                {
                    return Err(crate::value::string_too_long(max_len));
                }
                out.push_str(&format_component(&date, m, offset_hours, offset_minutes)?)
            }
        }
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
                && name.chars().count() > max
            {
                name = name.chars().take(max).collect();
            }
            return Ok(name);
        }
        return format_integer_with(v, m.integer_format.as_ref().expect("integer format"));
    }
    match m.component {
        'f' => match (value, &m.integer_format) {
            (Component::Number(v), Some(format)) => format_integer_with(v, format),
            // A name presentation (`[fn]`) leaves no integer format; jsonata-js then throws a
            // TypeError.
            (Component::Number(_), None) => Err(Error::new("D3133").with("value", "f")),
            _ => Ok(String::new()),
        },
        'Z' | 'z' => {
            let offset = offset_hours * 100.0 + offset_minutes;
            let format = m.integer_format.as_ref().expect("integer format");
            let mut s = if format.regular {
                format_integer_with(offset, format)?
            } else {
                match format.mandatory_digits {
                    1 | 2 => {
                        let mut s = format_integer_with(offset_hours, format)?;
                        if offset_minutes != 0.0 {
                            s.push(':');
                            s.push_str(&format_integer(offset_minutes, "00", usize::MAX)?);
                        }
                        s
                    }
                    3 | 4 => format_integer_with(offset, format)?,
                    n => return Err(Error::new("D3134").with("value", n)),
                }
            };
            if offset >= 0.0 {
                s.insert(0, '+');
            }
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

/// `$toMillis`: parses an ISO 8601 timestamp, or one described by a picture string. `now` is the
/// evaluation's timestamp, used to fill in unspecified leading components. `find` runs the
/// regex built from the picture, within the evaluation's limits (`Evaluator::find_regex`).
pub(crate) fn to_millis(
    timestamp: &str,
    picture: Option<&str>,
    now: f64,
    find: impl FnOnce(&regress::Regex, &str) -> Result<Option<regress::Match>>,
) -> Result<Option<f64>> {
    match picture {
        None => parse_iso8601(timestamp)
            .map(Some)
            .ok_or_else(|| Error::new("D3110").with("value", timestamp)),
        Some(picture) => parse_datetime(timestamp, picture, now, find),
    }
}

/// Validates and parses `^\d{4}(-[01]\d)?(-[0-3]\d)?(T[0-2]\d:[0-5]\d:[0-5]\d)?(\.\d+)?([+-][0-2]\d:?[0-5]\d|Z)?$`
/// like `Date.parse`. A timestamp without an offset is read as UTC.
fn parse_iso8601(s: &str) -> Option<f64> {
    let re = regress::Regex::new(
        r"^(\d{4})(?:-([01]\d))?(?:-([0-3]\d))?(?:T([0-2]\d):([0-5]\d):([0-5]\d))?(?:\.(\d+))?(?:([+-])([0-2]\d):?([0-5]\d)|(Z))?$",
    )
    .expect("valid regex");
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

fn escape_regex(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn parse_datetime(
    timestamp: &str,
    picture: &str,
    now: f64,
    find: impl FnOnce(&regress::Regex, &str) -> Result<Option<regress::Match>>,
) -> Result<Option<f64>> {
    let spec = analyse_datetime_picture(picture)?;
    let mut regex = String::from("^");
    let mut parsers: Vec<Option<(char, Parser)>> = Vec::new();
    for part in &spec {
        match part {
            Part::Literal(s) => {
                regex.push_str(&format!("({})", escape_regex(s)));
                parsers.push(None);
            }
            Part::Marker(m) if m.component == 'Z' || m.component == 'z' => {
                let format = m.integer_format.as_ref().expect("integer format");
                let separator = format.regular.then(|| format.separators[0].character);
                let mut r = String::new();
                if m.component == 'z' {
                    r.push_str("GMT");
                }
                r.push_str("[-+][0-9]+");
                if let Some(sep) = separator {
                    r.push_str(&escape_regex(&sep.to_string()));
                    r.push_str("[0-9]+");
                }
                regex.push_str(&format!("({r})"));
                let is_z = m.component == 'z';
                parsers.push(Some((
                    m.component,
                    Box::new(move |v: &str| {
                        let v = if is_z { &v[3.min(v.len())..] } else { v };
                        let (hours, minutes) =
                            match separator.and_then(|s| v.find(s).map(|i| (s, i))) {
                                Some((s, i)) => {
                                    (js_parse_int(&v[..i]), js_parse_int(&v[i + s.len_utf8()..]))
                                }
                                None => {
                                    if v.len() - 1 <= 2 {
                                        (js_parse_int(v), 0.0)
                                    } else {
                                        (js_parse_int(&v[..3]), js_parse_int(&v[3..]))
                                    }
                                }
                            };
                        hours * 60.0 + minutes
                    }),
                )));
            }
            Part::Marker(m) if m.component == 'f' => {
                regex.push_str("([0-9]+)");
                parsers.push(Some((
                    'f',
                    Box::new(|v: &str| {
                        let digits: String = v.chars().take(3).collect();
                        format!("0.{digits}").parse::<f64>().unwrap_or(f64::NAN) * 1000.0
                    }),
                )));
            }
            Part::Marker(m) => match &m.integer_format {
                Some(format) => {
                    let (r, parse) = integer_matcher(format)?;
                    regex.push_str(&format!("({r})"));
                    parsers.push(Some((m.component, parse)));
                }
                None => {
                    regex.push_str("([a-zA-Z]+)");
                    let max = m.width.as_ref().and_then(|w| w.max);
                    let truncate = move |name: &str| -> String {
                        match max {
                            Some(max) => name.chars().take(max).collect(),
                            None => name.to_owned(),
                        }
                    };
                    let lookup: Vec<(String, f64)> = match m.component {
                        'M' | 'x' => MONTHS
                            .iter()
                            .enumerate()
                            .map(|(i, n)| (truncate(n), (i + 1) as f64))
                            .collect(),
                        'F' => DAYS
                            .iter()
                            .enumerate()
                            .skip(1)
                            .map(|(i, n)| (truncate(n), i as f64))
                            .collect(),
                        'P' => vec![
                            ("am".into(), 0.0),
                            ("AM".into(), 0.0),
                            ("pm".into(), 1.0),
                            ("PM".into(), 1.0),
                        ],
                        c => return Err(Error::new("D3133").with("value", c.to_string())),
                    };
                    parsers.push(Some((
                        m.component,
                        Box::new(move |v: &str| {
                            lookup
                                .iter()
                                .find(|(k, _)| k == v)
                                .map_or(f64::NAN, |(_, n)| *n)
                        }),
                    )));
                }
            },
        }
    }
    regex.push('$');
    let matcher = regress::Regex::with_flags(&regex, "i")
        .map_err(|e| Error::new("D3136").with("message", e.to_string()))?;
    let Some(info) = find(&matcher, timestamp)? else {
        return Ok(None);
    };

    let mut components: std::collections::HashMap<char, f64> = std::collections::HashMap::new();
    for (i, parser) in parsers.iter().enumerate() {
        if let Some((component, parse)) = parser
            && let Some(range) = info.group(i + 1)
        {
            components.insert(*component, parse(&timestamp[range]));
        }
    }
    if components.is_empty() {
        return Ok(None);
    }

    let mut mask = 0u32;
    let mut shift = |set: bool| {
        mask <<= 1;
        mask += u32::from(set);
    };
    for c in "YXMxWwdD".chars() {
        shift(components.get(&c).is_some_and(|v| *v != 0.0 && !v.is_nan()));
    }
    let is_type = |ty: u32, mask: u32| (!ty & mask) == 0 && (ty & mask) != 0;
    let date_mask = mask;
    let date_a = is_type(161, date_mask);
    let date_b = !date_a && is_type(130, date_mask);
    let date_c = is_type(84, date_mask);
    let date_d = !date_c && is_type(72, date_mask);
    let mut mask = 0u32;
    for c in "PHhmsf".chars() {
        mask <<= 1;
        mask += u32::from(components.get(&c).is_some_and(|v| *v != 0.0 && !v.is_nan()));
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
        match components.entry(part) {
            std::collections::hash_map::Entry::Occupied(_) => {
                start_specified = true;
                if end_specified {
                    return Err(Error::new("D3136"));
                }
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                if start_specified {
                    // Past the specified block: default to zero (one for month and day).
                    slot.insert(if "MDd".contains(part) { 1.0 } else { 0.0 });
                    end_specified = true;
                } else if let Component::Number(v) = fragment(&now_fields, part) {
                    // Before it: default to the current timestamp's component.
                    slot.insert(v);
                }
            }
        }
    }

    let get = |c: char| components.get(&c).copied().unwrap_or(0.0);
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
    let tz = components
        .get(&'Z')
        .or(components.get(&'z'))
        .copied()
        .unwrap_or(0.0);
    if tz != 0.0 && !tz.is_nan() {
        millis -= tz * 60_000.0;
    }
    Ok(Some(millis))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            to_millis("23rd March 2018", Some("[D1o] [MNn] [Y]"), 0.0, |re, s| Ok(
                re.find(s)
            ))
            .unwrap(),
            Some(1_521_763_200_000.0)
        );
    }
}
