//! Numeric functions, including `$formatNumber` (XPath F&O 3.1 `fn:format-number`).

use super::{Call, arg, num, text};
use crate::error::{Error, Result};
use crate::eval::Evaluator;
use crate::value::{Value, js_number};

type R<'a> = Result<Value<'a>>;

pub(super) fn number<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    Ok(match arg(&a, 0) {
        Value::Undefined => Value::Undefined,
        Value::Number(n) => Value::Number(n),
        Value::Bool(b) => Value::Number(if b { 1.0 } else { 0.0 }),
        Value::String(s) => {
            if is_decimal_literal(&s)
                && let Ok(v) = s.parse::<f64>()
                && v.is_finite()
            {
                Value::Number(v)
            } else if let Some(v) = radix_literal(&s) {
                Value::Number(v)
            } else {
                return Err(Error::new("D3030")
                    .with("value", s.as_ref())
                    .with("index", 1));
            }
        }
        other => {
            let mut err = Error::new("D3030").with("index", 1);
            if let Some(v) = other.to_json_detail() {
                err = err.with("value", v);
            }
            return Err(err);
        }
    })
}

/// `^-?[0-9]+(\.[0-9]+)?([Ee][-+]?[0-9]+)?$`
fn is_decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i) {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

/// Hexadecimal, octal and binary literals (`0x1F`, `0o17`, `0b101`). jsonata-js accepts any
/// string its (loosely anchored) regex matches and returns `Number(arg)`, which is NaN unless
/// the whole string is a valid literal.
fn radix_literal(s: &str) -> Option<f64> {
    let is = |prefix: &[&str], valid: fn(char) -> bool, at_start: bool, at_end: bool| -> bool {
        prefix.iter().any(|p| {
            let found = if at_start {
                s.starts_with(p).then_some(0)
            } else {
                s.find(p)
            };
            found.is_some_and(|i| {
                let rest = &s[i + 2..];
                let n = rest.chars().take_while(|&c| valid(c)).count();
                n > 0 && (!at_end || i + 2 + n == s.len())
            })
        })
    };
    let hex = is(&["0x", "0X"], |c| c.is_ascii_hexdigit(), true, false);
    let oct = is(&["0o", "0O"], |c| ('0'..='7').contains(&c), false, false);
    let bin = is(&["0b", "0B"], |c| c == '0' || c == '1', false, true);
    if !(hex || oct || bin) {
        return None;
    }
    let (radix, digits) = match s.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (16, &s[2..]),
        Some("0o") => (8, &s[2..]),
        Some("0b") => (2, &s[2..]),
        _ => return Some(f64::NAN),
    };
    Some(u64::from_str_radix(digits, radix).map_or(f64::NAN, |v| v as f64))
}

/// `Math.round`: the nearest integer, halves rounding towards +∞.
fn js_round(x: f64) -> f64 {
    let f = x.floor();
    if x - f >= 0.5 { f + 1.0 } else { f }
}

/// Shifts the decimal point by editing the number's JavaScript string form, as jsonata-js
/// does to avoid binary floating-point errors.
fn shift(x: f64, places: f64) -> f64 {
    let s = js_number(x);
    let (mantissa, exp) = match s.split_once('e') {
        Some((m, e)) => (m.to_owned(), e.parse::<f64>().unwrap_or(0.0)),
        None => (s, 0.0),
    };
    format!("{mantissa}e{}", exp + places)
        .parse()
        .unwrap_or(f64::NAN)
}

/// `$round`: round half to even, optionally to `precision` decimal places.
pub(crate) fn round(arg: f64, precision: f64) -> f64 {
    let precision = if precision.is_nan() { 0.0 } else { precision };
    let mut value = if precision != 0.0 {
        shift(arg, precision)
    } else {
        arg
    };
    if value.is_infinite() && arg.is_finite() {
        // Shifting overflowed: `arg` is too large to have fractional digits. jsonata-js shifts
        // back from `Infinity` and gets NaN.
        return arg;
    }
    let mut result = js_round(value);
    let diff = result - value;
    if diff.abs() == 0.5 && (result % 2.0).abs() == 1.0 {
        result -= 1.0;
    }
    value = result;
    if precision != 0.0 {
        value = shift(value, -precision);
    }
    if value == 0.0 { 0.0 } else { value }
}

pub(super) fn sqrt<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let Some(v) = num(&a, 0) else {
        return Ok(Value::Undefined);
    };
    if v < 0.0 {
        return Err(Error::new("D3060").with("value", v).with("index", 1));
    }
    Ok(Value::Number(v.sqrt()))
}

pub(super) fn power<'a>(_: &mut Evaluator<'_, 'a>, a: Vec<Value<'a>>, _: &Call<'_, 'a>) -> R<'a> {
    let Some(base) = num(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let exp = num(&a, 1).unwrap_or(f64::NAN);
    // JavaScript's `Math.pow`, which unlike IEEE `pow` gives NaN for 1^NaN and (±1)^±Infinity.
    let result = if base.abs() == 1.0 && !exp.is_finite() {
        f64::NAN
    } else {
        base.powf(exp)
    };
    if !result.is_finite() {
        return Err(Error::new("D3061")
            .with("value", base)
            .with("exp", exp)
            .with("index", 1));
    }
    Ok(Value::Number(result))
}

pub(super) fn format_base<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let Some(value) = num(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let value = round(value, 0.0);
    let radix = num(&a, 1).map_or(10.0, |r| round(r, 0.0));
    if !(2.0..=36.0).contains(&radix) {
        return Err(Error::new("D3100").with("value", radix));
    }
    Ok(Value::string(to_radix(value, radix as u32)))
}

/// `Number.prototype.toString(radix)` for integral values, with every digit exact: from 2^53, V8
/// writes zeros for the digits below the number's precision in a radix that is not a power of two.
fn to_radix(value: f64, radix: u32) -> String {
    if radix == 10 || !value.is_finite() {
        return js_number(value);
    }
    let negative = value < 0.0;
    // The integer as 32-bit limbs, least significant first: its mantissa shifted by its exponent.
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, shift) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | 1 << 52, exponent - 1075)
    };
    let mut limbs = if shift < 0 {
        let n = mantissa >> shift.unsigned_abs().min(63);
        vec![n as u32, (n >> 32) as u32]
    } else {
        let mut limbs = vec![0; (shift / 32) as usize];
        let wide = u128::from(mantissa) << (shift % 32);
        limbs.extend([wide as u32, (wide >> 32) as u32, (wide >> 64) as u32]);
        limbs
    };
    let divisor = u64::from(radix);
    let mut digits = Vec::new();
    loop {
        let mut remainder = 0;
        for limb in limbs.iter_mut().rev() {
            let current = remainder << 32 | u64::from(*limb);
            *limb = (current / divisor) as u32;
            remainder = current % divisor;
        }
        digits.push(std::char::from_digit(remainder as u32, radix).unwrap_or('0'));
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        if limbs.is_empty() {
            break;
        }
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

/// `Number.prototype.toFixed`: rounds half away from zero on the exact binary value. From 1e21,
/// where JavaScript switches to exponent notation (`1e+21`), it keeps writing digits.
fn to_fixed(x: f64, dp: usize) -> String {
    if !x.is_finite() {
        return js_number(x);
    }
    let exact = format!("{:.*}", dp + 30, x.abs());
    let tail = &exact[exact.len() - 30..];
    let tie = tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0');
    let mut out = if tie {
        // Ties go to the larger magnitude, like JavaScript.
        let bumped = x.abs() + 0.5 * 10f64.powi(-(dp as i32));
        format!("{:.*}", dp, bumped)
    } else {
        format!("{:.*}", dp, x.abs())
    };
    if x < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.insert(0, '-');
    }
    out
}

/// Settings of a picture string (`$formatNumber`'s third argument overrides them).
struct Props {
    decimal: char,
    grouping: char,
    exponent: char,
    minus: String,
    percent: char,
    per_mille: char,
    zero: char,
    digit: char,
    pattern: char,
}

#[derive(Clone)]
struct Parts {
    prefix: Vec<char>,
    suffix: Vec<char>,
    active: Vec<char>,
    mantissa: Vec<char>,
    exponent: Option<Vec<char>>,
    integer: Vec<char>,
    fractional: Vec<char>,
    sub: Vec<char>,
}

#[derive(Clone)]
struct Analysis {
    integer_grouping: Vec<usize>,
    regular_grouping: usize,
    min_integer: usize,
    scaling: usize,
    prefix: String,
    fractional_grouping: Vec<usize>,
    min_fraction: usize,
    max_fraction: usize,
    min_exponent: usize,
    suffix: String,
    picture: Vec<char>,
}

fn index_of(hay: &[char], c: char, from: usize) -> Option<usize> {
    hay.iter()
        .skip(from)
        .position(|&x| x == c)
        .map(|i| i + from)
}

fn last_index_of(hay: &[char], c: char) -> Option<usize> {
    hay.iter().rposition(|&x| x == c)
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn regular(indexes: &[usize]) -> usize {
    if indexes.is_empty() {
        return 0;
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

pub(super) fn format_number<'a>(
    _: &mut Evaluator<'_, 'a>,
    a: Vec<Value<'a>>,
    _: &Call<'_, 'a>,
) -> R<'a> {
    let Some(value) = num(&a, 0) else {
        return Ok(Value::Undefined);
    };
    let picture = text(&a, 1).unwrap_or_default();
    let mut p = Props {
        decimal: '.',
        grouping: ',',
        exponent: 'e',
        minus: "-".into(),
        percent: '%',
        per_mille: '\u{2030}',
        zero: '0',
        digit: '#',
        pattern: ';',
    };
    if let Value::Object(options) = arg(&a, 2) {
        for (key, v) in options.borrow().iter() {
            let Some(s) = v.as_str() else { continue };
            let Some(c) = s.chars().next() else { continue };
            match key.as_ref() {
                "decimal-separator" => p.decimal = c,
                "grouping-separator" => p.grouping = c,
                "exponent-separator" => p.exponent = c,
                "minus-sign" => p.minus = s.to_owned(),
                "percent" => p.percent = c,
                "per-mille" => p.per_mille = c,
                "zero-digit" => p.zero = c,
                "digit" => p.digit = c,
                "pattern-separator" => p.pattern = c,
                _ => {}
            }
        }
    }
    format_number_with(value, &picture, &p).map(Value::string)
}

fn format_number_with(value: f64, picture: &str, p: &Props) -> Result<String> {
    let zero_code = p.zero as u32;
    let family: Vec<char> = (0..10)
        .filter_map(|i| char::from_u32(zero_code + i))
        .collect();
    let is_digit = |c: char| family.contains(&c);
    let mut active = family.clone();
    active.extend([p.decimal, p.exponent, p.grouping, p.digit, p.pattern]);
    let is_active = |c: char| active.contains(&c);

    let pic: Vec<char> = picture.chars().collect();
    let sub_pictures: Vec<Vec<char>> = pic
        .split(|&c| c == p.pattern)
        .map(<[char]>::to_vec)
        .collect();
    if sub_pictures.len() > 2 {
        return Err(Error::new("D3080"));
    }

    let split_parts = |sub: Vec<char>| -> Parts {
        let prefix_len = sub
            .iter()
            .position(|&c| is_active(c) && c != p.exponent)
            .unwrap_or(0);
        let prefix: Vec<char> = sub[..prefix_len].to_vec();
        let suffix: Vec<char> = match sub.iter().rposition(|&c| is_active(c) && c != p.exponent) {
            Some(i) => sub[i + 1..].to_vec(),
            None => Vec::new(),
        };
        let active_part: Vec<char> = sub[prefix.len()..sub.len() - suffix.len()].to_vec();
        let exponent_position = index_of(&sub, p.exponent, prefix.len());
        let (mantissa, exponent) = match exponent_position {
            Some(e) if e <= sub.len() - suffix.len() => {
                // jsonata-js indexes the active part with a position taken from the whole
                // sub-picture; reproduced as is.
                let m: Vec<char> = active_part.iter().take(e).copied().collect();
                let x: Vec<char> = active_part.iter().skip(e + 1).copied().collect();
                (m, Some(x))
            }
            _ => (active_part.clone(), None),
        };
        let (integer, fractional) = match index_of(&mantissa, p.decimal, 0) {
            None => (mantissa.clone(), suffix.clone()),
            Some(d) => (mantissa[..d].to_vec(), mantissa[d + 1..].to_vec()),
        };
        Parts {
            prefix,
            suffix,
            active: active_part,
            mantissa,
            exponent,
            integer,
            fractional,
            sub,
        }
    };

    let validate = |parts: &Parts| -> Result<()> {
        let mut error = None;
        let sub = &parts.sub;
        let decimal_pos = index_of(sub, p.decimal, 0);
        if decimal_pos != last_index_of(sub, p.decimal) {
            error = Some("D3081");
        }
        if index_of(sub, p.percent, 0) != last_index_of(sub, p.percent) {
            error = Some("D3082");
        }
        if index_of(sub, p.per_mille, 0) != last_index_of(sub, p.per_mille) {
            error = Some("D3083");
        }
        if sub.contains(&p.percent) && sub.contains(&p.per_mille) {
            error = Some("D3084");
        }
        if !parts.mantissa.iter().any(|&c| is_digit(c) || c == p.digit) {
            error = Some("D3085");
        }
        if parts.active.iter().any(|&c| !is_active(c)) {
            error = Some("D3086");
        }
        match decimal_pos {
            Some(d) => {
                let before = d.checked_sub(1).and_then(|i| sub.get(i));
                if before == Some(&p.grouping) || sub.get(d + 1) == Some(&p.grouping) {
                    error = Some("D3087");
                }
            }
            None => {
                if parts.integer.last() == Some(&p.grouping) {
                    error = Some("D3088");
                }
            }
        }
        if sub
            .windows(2)
            .any(|w| w[0] == p.grouping && w[1] == p.grouping)
        {
            error = Some("D3089");
        }
        if let Some(pos) = index_of(&parts.integer, p.digit, 0)
            && parts.integer[..pos].iter().any(|&c| is_digit(c))
        {
            error = Some("D3090");
        }
        if let Some(pos) = last_index_of(&parts.fractional, p.digit)
            && parts.fractional[pos..].iter().any(|&c| is_digit(c))
        {
            error = Some("D3091");
        }
        if let Some(exp) = &parts.exponent {
            if !exp.is_empty() && (sub.contains(&p.percent) || sub.contains(&p.per_mille)) {
                error = Some("D3092");
            }
            if exp.is_empty() || exp.iter().any(|&c| !is_digit(c)) {
                error = Some("D3093");
            }
        }
        match error {
            Some(code) => Err(Error::new(code)),
            None => Ok(()),
        }
    };

    let analyse = |parts: &Parts| -> Analysis {
        let grouping_positions = |part: &[char], to_left: bool| -> Vec<usize> {
            let mut positions = Vec::new();
            let mut gp = index_of(part, p.grouping, 0);
            while let Some(g) = gp {
                let range: &[char] = if to_left {
                    &part[..g.min(part.len())]
                } else {
                    &part[g.min(part.len())..]
                };
                positions.push(
                    range
                        .iter()
                        .filter(|&&c| is_digit(c) || c == p.digit)
                        .count(),
                );
                // jsonata-js continues the search in the integer part, even for the fraction.
                gp = index_of(&parts.integer, p.grouping, g + 1);
            }
            positions
        };
        let integer_grouping = grouping_positions(&parts.integer, false);
        let regular_grouping = regular(&integer_grouping);
        let fractional_grouping = grouping_positions(&parts.fractional, true);
        let mut min_integer = parts.integer.iter().filter(|&&c| is_digit(c)).count();
        let scaling = min_integer;
        let mut min_fraction = parts.fractional.iter().filter(|&&c| is_digit(c)).count();
        let mut max_fraction = parts
            .fractional
            .iter()
            .filter(|&&c| is_digit(c) || c == p.digit)
            .count();
        let exponent_present = parts.exponent.is_some();
        if min_integer == 0 && max_fraction == 0 {
            if exponent_present {
                min_fraction = 1;
                max_fraction = 1;
            } else {
                min_integer = 1;
            }
        }
        if exponent_present && min_integer == 0 && parts.integer.contains(&p.digit) {
            min_integer = 1;
        }
        if min_integer == 0 && min_fraction == 0 {
            min_fraction = 1;
        }
        let min_exponent = parts
            .exponent
            .as_ref()
            .map_or(0, |e| e.iter().filter(|&&c| is_digit(c)).count());
        Analysis {
            integer_grouping,
            regular_grouping,
            min_integer,
            scaling,
            prefix: parts.prefix.iter().collect(),
            fractional_grouping,
            min_fraction,
            max_fraction,
            min_exponent,
            suffix: parts.suffix.iter().collect(),
            picture: parts.sub.clone(),
        }
    };

    let parts: Vec<Parts> = sub_pictures.into_iter().map(split_parts).collect();
    for part in &parts {
        validate(part)?;
    }
    let mut variables: Vec<Analysis> = parts.iter().map(analyse).collect();
    if variables.len() == 1 {
        let mut negative = variables[0].clone();
        negative.prefix = format!("{}{}", p.minus, negative.prefix);
        variables.push(negative);
    }

    let pic = if value >= 0.0 {
        &variables[0]
    } else {
        &variables[1]
    };
    let adjusted = if pic.picture.contains(&p.percent) {
        value * 100.0
    } else if pic.picture.contains(&p.per_mille) {
        value * 1000.0
    } else {
        value
    };

    let (mantissa, exponent) = if pic.min_exponent == 0 {
        (adjusted, None)
    } else {
        let max_mantissa = 10f64.powi(pic.scaling as i32);
        let min_mantissa = 10f64.powi(pic.scaling as i32 - 1);
        let mut m = adjusted;
        let mut e = 0i32;
        // Infinity never scales into range; jsonata-js loops forever.
        if m != 0.0 && m.is_finite() {
            while m.abs() < min_mantissa {
                m *= 10.0;
                e -= 1;
            }
            while m.abs() > max_mantissa {
                m /= 10.0;
                e += 1;
            }
        }
        (m, Some(e))
    };

    let rounded = round(mantissa, pic.max_fraction as f64);
    let make_string = |v: f64, dp: usize| -> Vec<char> {
        to_fixed(v.abs(), dp)
            .chars()
            .map(|c| {
                if c.is_ascii_digit() {
                    family[(c as u8 - b'0') as usize]
                } else {
                    c
                }
            })
            .collect()
    };
    let mut s = make_string(rounded, pic.max_fraction);
    match s.iter().position(|&c| c == '.') {
        None => s.push(p.decimal),
        Some(i) => s[i] = p.decimal,
    }
    while s.first() == Some(&p.zero) {
        s.remove(0);
    }
    while s.last() == Some(&p.zero) {
        s.pop();
    }
    let mut decimal_pos = index_of(&s, p.decimal, 0).unwrap_or(s.len());
    let pad_left = pic.min_integer as isize - decimal_pos as isize;
    let pad_right = pic.min_fraction as isize - (s.len() as isize - decimal_pos as isize - 1);
    if pad_left > 0 {
        s.splice(0..0, std::iter::repeat_n(p.zero, pad_left as usize));
    }
    if pad_right > 0 {
        s.extend(std::iter::repeat_n(p.zero, pad_right as usize));
    }
    decimal_pos = index_of(&s, p.decimal, 0).unwrap_or(s.len());
    if let Some(group_count) =
        ((decimal_pos as isize - 1).max(0) as usize).checked_div(pic.regular_grouping)
    {
        for group in 1..=group_count {
            let at = decimal_pos - group * pic.regular_grouping;
            s.insert(at, p.grouping);
        }
    } else {
        for &pos in &pic.integer_grouping {
            // A separator needs a digit on its left. jsonata-js inserts one anyway, counting a
            // negative index from the end (`$formatNumber(1234, '#,##,##0')` is "12,34,");
            // jlens skips it (compatibility.md).
            if pos >= decimal_pos {
                continue;
            }
            s.insert(decimal_pos - pos, p.grouping);
            decimal_pos += 1;
        }
    }
    decimal_pos = index_of(&s, p.decimal, 0).unwrap_or(s.len());
    for &pos in &pic.fractional_grouping {
        let at = (pos + decimal_pos + 1).min(s.len());
        s.insert(at, p.grouping);
    }
    decimal_pos = index_of(&s, p.decimal, 0).unwrap_or(s.len());
    if !pic.picture.contains(&p.decimal) || decimal_pos + 1 == s.len() {
        s.pop();
    }
    let mut out: String = s.into_iter().collect();
    if let Some(e) = exponent {
        let mut digits = make_string(f64::from(e), 0);
        if digits.len() < pic.min_exponent {
            digits.splice(
                0..0,
                std::iter::repeat_n(p.zero, pic.min_exponent - digits.len()),
            );
        }
        out.push(p.exponent);
        if e < 0 {
            out.push_str(&p.minus);
        }
        out.extend(digits);
    }
    Ok(format!("{}{out}{}", pic.prefix, pic.suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(v: f64, picture: &str) -> String {
        let p = Props {
            decimal: '.',
            grouping: ',',
            exponent: 'e',
            minus: "-".into(),
            percent: '%',
            per_mille: '\u{2030}',
            zero: '0',
            digit: '#',
            pattern: ';',
        };
        format_number_with(v, picture, &p).unwrap()
    }

    #[test]
    fn format_number_examples() {
        assert_eq!(fmt(12345.6, "#,###.00"), "12,345.60");
        assert_eq!(fmt(1234.5678, "00.000e0"), "12.346e2");
        assert_eq!(fmt(0.14, "01%"), "14%");
        assert_eq!(fmt(-6.0, "000"), "-006");
        assert_eq!(fmt(1e6, "#,##0"), "1,000,000");
    }

    #[test]
    fn format_number_from_1e21_writes_digits() {
        assert_eq!(fmt(1e21, "#,##0.00"), "1,000,000,000,000,000,000,000.00");
        assert_eq!(fmt(1e20, "0%"), "10000000000000000000000%");
        assert_eq!(fmt(-1.5e22, "0"), "-15000000000000000000000");
        // The exact binary value, as below 1e21 (123456789012345680000 writes ...683,968).
        assert_eq!(fmt(1e23, "0"), "99999999999999991611392");
        assert_eq!(fmt(1e300, "#,##0").len(), 301 + 100);
    }

    #[test]
    fn format_base_writes_exact_digits() {
        assert_eq!(
            to_radix(1e30, 3),
            "212120220222112010021212002200000020022221100201112201012210221"
        );
        assert_eq!(to_radix(2e30, 36), "5dwijtb562osg8s15iww");
        assert_eq!(
            to_radix(1e300, 16),
            format!("17e43c8800759c{}", "0".repeat(236))
        );
        assert_eq!(
            to_radix(-f64::MAX, 2),
            format!("-{}", "1".repeat(53) + &"0".repeat(971))
        );
        assert_eq!(to_radix(0.0, 7), "0");
        assert_eq!(to_radix(-0.0, 7), "0");
        assert_eq!(to_radix(f64::NAN, 16), "NaN");
    }

    #[test]
    fn rounding_helpers() {
        assert_eq!(round(2.5, 0.0), 2.0);
        assert_eq!(round(3.5, 0.0), 4.0);
        assert_eq!(round(-2.5, 0.0), -2.0);
        assert_eq!(round(1.005, 2.0), 1.0);
        assert_eq!(round(12345.0, -2.0), 12300.0);
        assert_eq!(round(-1e300, 20.0), -1e300);
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_radix(255.0, 16), "ff");
        assert_eq!(to_radix(-5.0, 2), "-101");
    }
}
