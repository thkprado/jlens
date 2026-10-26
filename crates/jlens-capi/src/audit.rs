//! Audited evaluations (0.25 #8): the audit tree of an evaluation and its explanation, as the
//! WebAssembly and Python packages give them, in one reply of values.
//!
//! The core audits an evaluation of JSON values ([`jlens_core::Expression::evaluate_audited`]),
//! so the input crosses whole, as values, and the reply is one `OBJECT`:
//!
//! - `result`: the result, absent when it is undefined or when the evaluation failed;
//! - `error`: when the evaluation failed, its envelope as JSON text, a `STRING`, as a
//!   [`REPLY_ERROR`](crate::REPLY_ERROR) carries it (with `hostException` for a host function's
//!   exception);
//! - `audit`: the audit tree, in the JSON form of `jlens eval --audit json`, its spans in UTF-16
//!   offsets;
//! - `explanation`: why the result was reached, `result` and `lines`;
//! - `explanationText`: the explanation as text, as `jlens eval --explain` prints it.

use jlens_core::audit::{self, AuditLevel, AuditNode, AuditOptions, CaptureLimits};
use jlens_core::{Bindings, format_json};
use serde_json::{Map, Value};

use crate::api::{
    DEFAULTS, EXPRESSIONS, NO_BINDINGS, envelope, failure, find_bindings, find_options, invalid,
    reply, unknown,
};
use crate::values::{Decoder, Encoder};
use crate::{REPLY_VALUES, buffers, host};

/// Evaluates expression `expression` against the values in buffer `input` (0 for no input),
/// with bindings `bindings` (0 for none), options `options` (0 for the defaults) and the audit
/// options in buffer `audit` (0 for the defaults), and replies with values: the object the
/// [module](self) documentation describes. An error of the evaluation is in that object; the
/// reply is an error for an unknown handle, input not in its form, or audit options not in
/// theirs (`U2005`, token `audit`).
///
/// The audit options are JSON text, an object with `level` (`"off"`, `"decisions"` or `"full"`)
/// and `limits`, an object with any of `maxItems`, `maxString`, `maxDepth`, `maxRejected`,
/// `maxNodes` and `maxSource`, each optional.
pub fn evaluate_audited(
    expression: u64,
    input: u64,
    bindings: u64,
    options: u64,
    audit: u64,
) -> u64 {
    reply(audited(expression, input, bindings, options, audit))
}

fn audited(
    expression: u64,
    input: u64,
    bindings: u64,
    options: u64,
    audit: u64,
) -> Result<Vec<u8>, Vec<u8>> {
    let compiled = EXPRESSIONS
        .get(expression)
        .ok_or_else(|| unknown("expression", expression))?;
    let bindings = find_bindings(bindings)?;
    let options = find_options(options)?;
    // The handles are checked before what they hold, so that an unknown one is always `U2002`.
    let input = match input {
        0 => None,
        input => Some(
            buffers::with(input, |bytes| {
                let mut decoder = Decoder::new(bytes);
                let value = decoder.value()?;
                decoder.end().map(|()| value)
            })
            .ok_or_else(|| unknown("buffer", input))?,
        ),
    };
    let audit = match audit {
        0 => Ok(AuditOptions::default()),
        audit => buffers::with(audit, |bytes| audit_options(bytes))
            .ok_or_else(|| unknown("buffer", audit))?,
    };
    let input = input.transpose().map_err(|e| failure(&e, ""))?;
    let audit = audit.map_err(|message| invalid("audit", message))?;

    let options = options.as_deref().unwrap_or(&DEFAULTS);
    let evaluate = |bindings: &Bindings| {
        compiled
            .expression
            .evaluate_audited(input.as_ref(), bindings, options, &audit)
    };
    let (result, mut tree) = match bindings.as_deref() {
        // The errors of the functions host functions call need the source for their offsets.
        Some(bound) if bound.functions => {
            host::evaluating(&compiled.source, || evaluate(&bound.bindings))
        }
        Some(bound) => evaluate(&bound.bindings),
        None => evaluate(&NO_BINDINGS),
    };
    let explanation = audit::explain(&tree);
    if let Some(root) = &mut tree.root {
        utf16_spans(&compiled.source, root);
    }

    let mut out = Map::new();
    match result {
        Ok(Some(value)) => {
            out.insert("result".into(), value);
        }
        Ok(None) => {}
        Err(error) => {
            let envelope = Value::Object(envelope(&error, &compiled.source));
            out.insert("error".into(), format_json(&envelope, false).into());
        }
    }
    out.insert(
        "audit".into(),
        serde_json::to_value(&tree).unwrap_or_default(),
    );
    out.insert(
        "explanation".into(),
        serde_json::to_value(&explanation).unwrap_or_default(),
    );
    out.insert("explanationText".into(), explanation.to_string().into());
    let mut encoder = Encoder::new(vec![REPLY_VALUES]);
    encoder.json(&Value::Object(out));
    Ok(encoder.out)
}

/// Reads audit options from JSON text: a `level` and `limits`, each optional, the limits not
/// given keeping their defaults.
fn audit_options(bytes: &[u8]) -> Result<AuditOptions, String> {
    let Value::Object(given) = serde_json::from_slice(bytes).map_err(|e| e.to_string())? else {
        return Err("audit options must be an object".into());
    };
    let mut options = AuditOptions::default();
    for (key, value) in given {
        match key.as_str() {
            "level" => {
                options.level = serde_json::from_value::<AuditLevel>(value).map_err(|_| {
                    "the level must be \"off\", \"decisions\" or \"full\"".to_owned()
                })?;
            }
            "limits" => options.limits = limits(value)?,
            _ => return Err(format!("unknown audit option `{key}`")),
        }
    }
    Ok(options)
}

/// Capture limits from a JSON object, the limits it does not give keeping their defaults.
fn limits(given: Value) -> Result<CaptureLimits, String> {
    let Value::Object(given) = given else {
        return Err("the limits must be an object".into());
    };
    let Ok(Value::Object(mut merged)) = serde_json::to_value(CaptureLimits::default()) else {
        return Err("no default limits".into());
    };
    for (key, value) in given {
        if !merged.contains_key(&key) {
            return Err(format!("unknown capture limit `{key}`"));
        }
        merged.insert(key, value);
    }
    serde_json::from_value(Value::Object(merged)).map_err(|e| format!("invalid limits: {e}"))
}

/// Converts the spans of the audit tree under `root` from byte offsets of `source` to UTF-16
/// offsets, walking with its own stack, as an audit nests as deep as the evaluation did.
fn utf16_spans(source: &str, root: &mut AuditNode) {
    if source.is_ascii() {
        return;
    }
    // The UTF-16 offset of each byte offset; a byte inside a character maps to its start.
    let mut offsets = vec![0; source.len() + 1];
    let mut units = 0;
    for (at, c) in source.char_indices() {
        offsets[at..at + c.len_utf8()].fill(units);
        units += c.len_utf16();
    }
    offsets[source.len()] = units;
    let at = |byte: usize| offsets[byte.min(source.len())];
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        node.span.start = at(node.span.start);
        node.span.end = at(node.span.end);
        pending.extend(node.children.iter_mut());
    }
}
