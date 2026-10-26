//! Bindings, and host functions written in the host's language (ADR 0006, ADR 0011; 0.25 #7).
//!
//! Bindings are made once ([`bindings_new`]): variables, written as values ([`crate::values`]),
//! and host functions, each with a name and an arity or a signature. An evaluation takes their
//! handle. A host function is a call back into the host:
//!
//! ```text
//! function(context, index, frame, arguments) -> reply
//! ```
//!
//! with `context` as the bindings were given it, `index` the function's place in the bindings'
//! list, `frame` the number of this call, and `arguments` the handle of a buffer that Rust frees
//! once the function returns: an `ARRAY` of values, in which an undefined argument is
//! `UNDEFINED` and a function `FUNCTION` with its arity. The reply is the handle of a buffer the
//! host made, which Rust frees:
//!
//! - [`REPLY_UNDEFINED`]: the result is undefined;
//! - [`REPLY_VALUES`] then one value: the result;
//! - [`REPLY_ERROR`] then a JSON object: the function failed. With `failure`, a number that
//!   [`call`] gave, the error is that call's, let through. Otherwise it is an error of `code`,
//!   with `message` and `token` when they are given, and `exception`, a number the host gives
//!   the exception it keeps, which the evaluation's error envelope carries back as
//!   `hostException` so that the host can throw it again.
//!
//! While the function runs, it can call a function among its arguments with [`call`], given
//! `frame` and the argument's index, from the same thread only: the core's handle borrows the
//! call (ADR 0011), so each call is published in a frame of a scoped thread-local, found by its
//! number. Once the function has returned, or from another thread, the frame is gone, and the
//! call fails with `U1005`.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use jlens_core::{Bindings, Error, HostCall, HostFunction};
use scoped_tls_hkt::scoped_thread_local;
use serde::Deserialize;
use serde_json::Value;

use crate::api::{envelope, failure, handle_reply, invalid, reply, unknown};
use crate::handles::Table;
use crate::values::{Decoder, Encoder, FUNCTION, OBJECT, UNDEFINED};
use crate::{REPLY_ERROR, REPLY_UNDEFINED, REPLY_VALUES, buffers};

/// The host's function that runs its host functions (see the [module](self) documentation).
pub type Function = extern "C" fn(context: u64, index: u32, frame: u64, arguments: u64) -> u64;

/// The live bindings. `Bindings` is `Send` and `Sync`: one serves every thread.
pub(crate) static BINDINGS: Table<Bound> = Table::new();

/// Bindings, and whether they hold host functions.
pub(crate) struct Bound {
    pub(crate) bindings: Bindings,
    pub(crate) functions: bool,
}

/// A host function, as [`bindings_new`] reads it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    name: String,
    #[serde(default)]
    arity: usize,
    signature: Option<String>,
}

/// Makes bindings of the host functions in buffer `functions` (0 for none), a JSON array of
/// objects with a `name`, and an `arity` or a `signature`, and of the variables in buffer
/// `variables` (0 for none), an `OBJECT` of values. The functions are run by `function`, given
/// `context`, and are marked as not reading the context of their call. Replies with the
/// bindings' handle or the error: a signature's (`S0401`, `S0402`), or `U2005` for functions or
/// variables not in their form, or functions with no `function`.
pub fn bindings_new(
    functions: u64,
    variables: u64,
    function: Option<Function>,
    context: u64,
) -> u64 {
    reply(make(functions, variables, function, context))
}

fn make(
    functions: u64,
    variables: u64,
    function: Option<Function>,
    context: u64,
) -> Result<Vec<u8>, Vec<u8>> {
    let specs: Vec<Spec> = match functions {
        0 => Vec::new(),
        functions => buffers::with(functions, |bytes| serde_json::from_slice(bytes))
            .ok_or_else(|| unknown("buffer", functions))?
            .map_err(|e| invalid("functions", e.to_string()))?,
    };
    let variables = match variables {
        0 => None,
        variables => Some(
            buffers::with(variables, |bytes| {
                let mut decoder = Decoder::new(bytes);
                let value = decoder.value()?;
                decoder.end().map(|()| value)
            })
            .ok_or_else(|| unknown("buffer", variables))?,
        ),
    };
    let mut bindings = Bindings::new();
    match variables.transpose().map_err(|e| failure(&e, ""))? {
        Some(Value::Object(fields)) => {
            for (name, value) in fields {
                bindings = bindings.var(name, value);
            }
        }
        Some(_) => {
            return Err(invalid(
                "variables",
                format!("not an object (tag {OBJECT})"),
            ));
        }
        None => {}
    }
    let functions = !specs.is_empty();
    if functions && function.is_none() {
        return Err(invalid("functions", "no function to run them".into()));
    }
    for (index, spec) in specs.into_iter().enumerate() {
        let Some(function) = function else { break };
        let index = u32::try_from(index).map_err(|_| invalid("functions", "too many".into()))?;
        let name = spec.name.clone();
        let imp = move |args: &[Option<Value>], call: &HostCall<'_>| {
            run(function, context, index, &name, args, call)
        };
        let host = match &spec.signature {
            Some(signature) => {
                HostFunction::with_signature(signature, imp).map_err(|e| failure(&e, signature))?
            }
            None => HostFunction::new(spec.arity, imp),
        };
        bindings = bindings.function(spec.name, host.without_input());
    }
    handle_reply(BINDINGS.insert(Bound {
        bindings,
        functions,
    }))
}

/// Frees bindings: 1 when they were live, 0 for an unknown or freed handle. An evaluation
/// running on another thread keeps them until it returns.
pub fn bindings_free(bindings: u64) -> u32 {
    u32::from(BINDINGS.remove(bindings).is_some())
}

/// A call of a host function in progress, published for the functions it receives.
struct Frame<'a> {
    id: u64,
    call: &'a HostCall<'a>,
    /// The source of the expression being evaluated, for the offsets of errors.
    source: &'a str,
    /// The call this one runs in, if any: a function of an outer call may still be called.
    parent: Option<&'a Frame<'a>>,
}

scoped_thread_local!(static FRAME: for<'a> &'a Frame<'a>);
scoped_thread_local!(static SOURCE: for<'a> &'a str);

/// Numbers the frames, and the failures of [`call`], from 1.
static NUMBERS: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The errors of the functions called with [`call`] on this thread, by number, while a
    /// host function runs: one that its host function lets through comes back as its number.
    static FAILURES: RefCell<Vec<(u64, Error)>> = const { RefCell::new(Vec::new()) };
}

/// Runs `evaluation` of the expression `source`, which the errors of [`call`] are offsets in.
pub(crate) fn evaluating<R>(source: &str, evaluation: impl FnOnce() -> R) -> R {
    SOURCE.set(source, evaluation)
}

/// Calls host function `index` of the bindings made with `context`.
fn run(
    function: Function,
    context: u64,
    index: u32,
    name: &str,
    args: &[Option<Value>],
    call: &HostCall<'_>,
) -> jlens_core::Result<Option<Value>> {
    let mut encoder = Encoder::new(Vec::new());
    let mut array = jlens_core::output::Builder::array(&mut encoder, args.len());
    for (i, arg) in args.iter().enumerate() {
        match (call.function(i), arg) {
            (Some(callable), _) => {
                encoder.out.push(FUNCTION);
                encoder.count(callable.arity());
            }
            (None, None) => encoder.out.push(UNDEFINED),
            (None, Some(value)) => encoder.json(value),
        }
        jlens_core::output::Builder::push(&mut encoder, &mut array, ());
    }
    jlens_core::output::Builder::end_array(&mut encoder, array);
    let arguments = buffers::insert(encoder.out);
    if arguments == 0 {
        return Err(Error::new("U1001").with("message", "Too many live objects in the C API"));
    }
    let id = NUMBERS.fetch_add(1, Ordering::Relaxed);
    let reply = framed(id, call, || {
        crate::upcall(|| function(context, index, id, arguments))
    });
    buffers::free(arguments);
    let result = read_reply(reply, name);
    if !FRAME.is_set() {
        // The outermost host call returned: no failure can come back any more.
        FAILURES.with_borrow_mut(Vec::clear);
    }
    result
}

/// Runs `f` with the frame of call `id` published.
fn framed<R>(id: u64, call: &HostCall<'_>, f: impl FnOnce() -> R) -> R {
    let in_source = |source: &str| {
        let enter = |parent: Option<&Frame<'_>>| {
            let frame = Frame {
                id,
                call,
                source,
                parent,
            };
            FRAME.set(&frame, f)
        };
        if FRAME.is_set() {
            FRAME.with(|parent| enter(Some(parent)))
        } else {
            enter(None)
        }
    };
    if SOURCE.is_set() {
        SOURCE.with(in_source)
    } else {
        in_source("")
    }
}

/// The result of a host function, in buffer `reply`.
fn read_reply(reply: u64, name: &str) -> jlens_core::Result<Option<Value>> {
    let invalid = |message: &str| {
        Error::new("U2005")
            .with("token", "reply")
            .with("message", format!("the host function ${name} {message}"))
    };
    let bytes = buffers::take(reply).ok_or_else(|| invalid("gave no reply"))?;
    match bytes.split_first() {
        Some((&REPLY_UNDEFINED, [])) => Ok(None),
        Some((&REPLY_VALUES, values)) => {
            let mut decoder = Decoder::new(values);
            let value = decoder.value()?;
            decoder.end()?;
            Ok(Some(value))
        }
        Some((&REPLY_ERROR, envelope)) => {
            let envelope: Value = serde_json::from_slice(envelope)
                .map_err(|e| invalid(&format!("failed with an error not in its form: {e}")))?;
            Err(host_error(&envelope))
        }
        _ => Err(invalid("gave a reply not in its form")),
    }
}

/// The error a host function failed with.
fn host_error(envelope: &Value) -> Error {
    if let Some(number) = envelope.get("failure").and_then(Value::as_u64)
        && let Some(error) = FAILURES.with_borrow(|failures| {
            failures
                .iter()
                .find(|(n, _)| *n == number)
                .map(|(_, error)| error.clone())
        })
    {
        return error;
    }
    let text = |name: &str| envelope.get(name).and_then(Value::as_str);
    let mut error = Error::new(text("code").unwrap_or("Exception"));
    if let Some(message) = text("message") {
        error = error.with("message", message);
    }
    if let Some(token) = text("token") {
        error = error.with("token", token);
    }
    if let Some(exception) = envelope.get("exception").and_then(Value::as_u64) {
        error = error.with("hostException", exception);
    }
    error
}

/// Calls argument `index`, a function, of the host function's call `frame`, with the arguments
/// in buffer `arguments` (0 for none), an `ARRAY` of values and `UNDEFINED`s, which stays the
/// caller's. Replies with the result as values, or the error, whose envelope has a `failure`,
/// the number to reply with when the host function lets the error through. A call after the
/// host function returned, or from another thread, fails with `U1005`, and an argument that is
/// not a function with `U2005`.
pub fn call(frame: u64, index: u32, arguments: u64) -> u64 {
    reply(call_in(frame, index, arguments))
}

fn call_in(frame: u64, index: u32, arguments: u64) -> Result<Vec<u8>, Vec<u8>> {
    let args = match arguments {
        0 => Vec::new(),
        arguments => buffers::with(arguments, |bytes| {
            let mut decoder = Decoder::new(bytes);
            let args = decoder.arguments()?;
            decoder.end().map(|()| args)
        })
        .ok_or_else(|| unknown("buffer", arguments))?
        .map_err(|e| failure(&e, ""))?,
    };
    let expired = || failure(&Error::new("U1005"), "");
    if !FRAME.is_set() {
        return Err(expired());
    }
    FRAME.with(|mut found| {
        while found.id != frame {
            found = found.parent.ok_or_else(expired)?;
        }
        let callable = found
            .call
            .function(index as usize)
            .ok_or_else(|| invalid("function", format!("argument {index} is not a function")))?;
        match callable.call(&args) {
            Ok(None) => Ok(vec![REPLY_UNDEFINED]),
            Ok(Some(value)) => {
                let mut encoder = Encoder::new(vec![REPLY_VALUES]);
                encoder.json(&value);
                Ok(encoder.out)
            }
            Err(error) => {
                let number = NUMBERS.fetch_add(1, Ordering::Relaxed);
                let mut envelope = envelope(&error, found.source);
                envelope.insert("failure".into(), number.into());
                FAILURES.with_borrow_mut(|failures| failures.push((number, error)));
                let mut reply = vec![REPLY_ERROR];
                reply.extend_from_slice(
                    jlens_core::format_json(&Value::Object(envelope), false).as_bytes(),
                );
                Err(reply)
            }
        }
    })
}
