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
/// function passed as an argument arrives as `None`; [`HostCall::function`] gives a handle to
/// call it (ADR 0011). An error returned by the function stops the evaluation; its position and
/// token are set to the call site when it has none.
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
    /// `caller` is the evaluator running the call, holding the same arguments.
    pub(crate) fn call<'a>(
        &self,
        args: &[Value<'a>],
        input: &Value<'a>,
        limit: usize,
        caller: &dyn Caller,
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
        // Converting the arguments reads them: a part of the input that could not be read is
        // not given to the function.
        caller.read_failure()?;
        let input = match input {
            // The input array, wrapped so that it counts as a single context value.
            Value::Array(a) if a.has(OUTER_WRAPPER) && a.len() == 1 => &a.items()[0],
            v => v,
        };
        let call = HostCall {
            input,
            limit,
            too_large: Cell::new(false),
            caller,
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

/// The evaluator running a host call, as the call sees it (ADR 0011). A trait object, so that
/// [`HostCall`] keeps a single lifetime.
pub(crate) trait Caller {
    /// Fails once a level of the input could not be read from its source.
    fn read_failure(&self) -> Result<()>;
    /// The arity of argument `index`, when it is a function.
    fn arity(&self, index: usize) -> Option<usize>;
    /// Applies argument `index`, a function, to `args`.
    fn apply(&self, index: usize, args: &[Option<JsonValue>]) -> Result<Option<JsonValue>>;
}

/// The context of a call to a [`HostFunction`].
pub struct HostCall<'a> {
    input: &'a dyn Inspect,
    limit: usize,
    /// Set when the context value was too large to convert; the call then fails with `U1001`.
    too_large: Cell<bool>,
    caller: &'a dyn Caller,
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

    /// A handle to call argument `index` when it is a function, such as the lambda in
    /// `$retry(function() { $fetch() }, 3)`: a lambda, a built-in function, a partial
    /// application, a chain or a host function. `None` when it is not a function. Arguments are
    /// indexed as the function receives them, after the signature has been checked.
    ///
    /// The function still receives `None` in place of the argument, as ADR 0006 decided; the
    /// handle borrows the call, so it cannot be called once the function has returned.
    ///
    /// ```
    /// use jlens_core::{Bindings, Expression, HostFunction};
    /// use serde_json::json;
    ///
    /// // `$twice($f, $x)` gives `$f($f($x))`.
    /// let twice = HostFunction::with_signature("<fx:x>", |args, call| {
    ///     let f = call.function(0).expect("a function, as the signature requires");
    ///     let once = f.call(&[args[1].clone()])?;
    ///     f.call(&[once])
    /// })
    /// .unwrap();
    /// let bindings = Bindings::new().function("twice", twice);
    /// let expr = Expression::compile("$twice(function($n) { $n * 3 }, 2)").unwrap();
    /// assert_eq!(expr.evaluate(None, &bindings).unwrap(), Some(json!(18)));
    /// ```
    pub fn function(&self, index: usize) -> Option<Callable<'_>> {
        let arity = self.caller.arity(index)?;
        Some(Callable {
            caller: self.caller,
            index,
            arity,
        })
    }
}

/// A function passed to a [`HostFunction`], which it can call while it runs (ADR 0011). Given
/// by [`HostCall::function`].
#[derive(Clone, Copy)]
pub struct Callable<'a> {
    caller: &'a dyn Caller,
    index: usize,
    arity: usize,
}

impl Callable<'_> {
    /// The number of declared parameters, as [`HostFunction::arity`] gives for a host function.
    pub fn arity(&self) -> usize {
        self.arity
    }

    /// Applies the function to `args`, as `$map` applies its function: extra arguments are
    /// passed on, missing ones are undefined, and the context value is `null`. A lambda keeps
    /// the variables of its definition.
    ///
    /// Arguments and the result cross the boundary as JSON, as for the host function itself: a
    /// sequence comes back as an array or a single value, and a function as `None`.
    ///
    /// The call is part of the evaluation: it counts as a level of depth and checks the
    /// timeout and the cancellation token, its result is limited like the evaluation's
    /// (`Options::max_result_size`), and an audit shows its steps under the host function's
    /// call. Its errors come back with their codes; returning one ends the evaluation, as any
    /// error from the expression does.
    pub fn call(&self, args: &[Option<JsonValue>]) -> Result<Option<JsonValue>> {
        self.caller.apply(self.index, args)
    }
}

impl fmt::Debug for Callable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Callable")
            .field("index", &self.index)
            .field("arity", &self.arity)
            .finish_non_exhaustive()
    }
}
