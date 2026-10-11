//! Functions provided by the host application (see ADR 0006).

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::signature::Signature;
use crate::value::{Inspect, OUTER_WRAPPER, Value};

type Implementation =
    dyn Fn(&[Option<JsonValue>], &HostCall<'_>) -> Result<Option<JsonValue>> + Send + Sync;

/// A function implemented by the host application, bound to an expression through
/// [`Bindings::function`](crate::Bindings::function) and called like a built-in.
///
/// Arguments and results cross the boundary as JSON: `None` stands for *undefined*, and a
/// function passed as an argument arrives as `None`. An error returned by the function stops
/// the evaluation; its position and token are set to the call site when it has none.
///
/// ```
/// use jlens_core::{Bindings, Expression, HostFunction};
/// use serde_json::json;
///
/// let tax = HostFunction::with_signature("<n-n?:n>", |args, _call| {
///     let amount = args[0].as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0);
///     let rate = args[1].as_ref().and_then(|v| v.as_f64()).unwrap_or(0.25);
///     Ok(Some(json!(amount * rate)))
/// })
/// .unwrap();
/// let bindings = Bindings::new().function("tax", tax);
/// // `price.$tax()` takes its amount from the context, as the `-` in the signature allows.
/// let expr = Expression::compile("$tax(200) + price.$tax()").unwrap();
/// let result = expr.evaluate(Some(&json!({"price": 8})), &bindings).unwrap();
/// assert_eq!(result, Some(json!(52)));
/// ```
#[derive(Clone)]
pub struct HostFunction {
    imp: Arc<Implementation>,
    signature: Option<Arc<Signature>>,
    arity: usize,
}

impl HostFunction {
    /// A function without a signature, declaring `arity` parameters.
    ///
    /// The arity plays the part of a JavaScript function's `length`. Higher-order functions use
    /// it to decide which arguments to pass (`$map` passes the index only to functions of two
    /// parameters or more), and partial application (`$f(?, 2)`) to know how many parameters
    /// there are. Arguments are passed as given, however many the call has: index them with
    /// `get`.
    pub fn new<F>(arity: usize, imp: F) -> Self
    where
        F: Fn(&[Option<JsonValue>], &HostCall<'_>) -> Result<Option<JsonValue>>
            + Send
            + Sync
            + 'static,
    {
        HostFunction {
            imp: Arc::new(imp),
            signature: None,
            arity,
        }
    }

    /// A function with a [signature](crate::signature) such as `<n-n?:n>`, which arguments are
    /// checked against before each call, as for built-in functions (`T0410`, `T0412`). The
    /// function then receives at least one argument per parameter in the signature (`None` for
    /// a missing optional one), and the context value where a parameter has the `-` modifier.
    /// The arity is the number of parameters in the signature.
    ///
    /// Fails with the signature's syntax error (`S0401`, `S0402`) if it is invalid.
    pub fn with_signature<F>(signature: &str, imp: F) -> Result<Self>
    where
        F: Fn(&[Option<JsonValue>], &HostCall<'_>) -> Result<Option<JsonValue>>
            + Send
            + Sync
            + 'static,
    {
        let signature = Signature::parse(signature)?;
        Ok(HostFunction {
            imp: Arc::new(imp),
            arity: signature.params.len(),
            signature: Some(Arc::new(signature)),
        })
    }

    /// The signature, if the function has one.
    pub fn signature(&self) -> Option<&Signature> {
        self.signature.as_deref()
    }

    /// The number of declared parameters.
    pub fn arity(&self) -> usize {
        self.arity
    }

    /// Calls the function with evaluated arguments, converting them to JSON and the result back.
    /// The arguments together, and the context value, are limited to `limit` bytes of JSON.
    pub(crate) fn call<'a>(
        &self,
        args: &[Value<'a>],
        input: &Value<'a>,
        limit: usize,
    ) -> Result<Value<'a>> {
        let too_large = || {
            Error::new("U1001").with(
                "message",
                format!(
                    "The arguments of a host function would exceed the size limit of {limit} bytes"
                ),
            )
        };
        let mut budget = limit;
        if !args.iter().all(|arg| arg.json_fits(&mut budget)) {
            return Err(too_large());
        }
        let json: Vec<Option<JsonValue>> = args.iter().map(Value::to_json).collect();
        let input = match input {
            // The input array, wrapped so that it counts as a single context value.
            Value::Array(a) if a.has(OUTER_WRAPPER) && a.len() == 1 => &a.items()[0],
            v => v,
        };
        let call = HostCall {
            input,
            limit,
            too_large: Cell::new(false),
        };
        let result = (self.imp)(&json, &call);
        if call.too_large.get() {
            return Err(too_large());
        }
        Ok(result?.as_ref().map_or(Value::Undefined, Value::from_json))
    }
}

impl fmt::Debug for HostFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostFunction")
            .field(
                "signature",
                &self.signature.as_ref().map(|s| s.definition.as_str()),
            )
            .field("arity", &self.arity)
            .finish_non_exhaustive()
    }
}

/// The context of a call to a [`HostFunction`].
pub struct HostCall<'a> {
    input: &'a dyn Inspect,
    limit: usize,
    /// Set when the context value was too large to convert; the call then fails with `U1001`.
    too_large: Cell<bool>,
}

impl HostCall<'_> {
    /// The context value of the call: the input of the function call expression, such as the
    /// current item in `items.$f()`. `None` when it is undefined.
    ///
    /// The value is converted to JSON when this method is called, so a function that does not
    /// need it does not pay for converting it. A value larger than the size limit of a result
    /// (`Options::max_result_size`) is `None`, and the evaluation then fails with `U1001`,
    /// whatever the function returns.
    pub fn input(&self) -> Option<JsonValue> {
        self.input.to_json_within(self.limit).unwrap_or_else(|_| {
            self.too_large.set(true);
            None
        })
    }
}
