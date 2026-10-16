# Python

The Python package `jlens` runs the jlens engine in CPython, through native extension modules
built with [PyO3](https://pyo3.rs/) ([ADR 0014](../adr/0014-python-package.md)). It covers
evaluation, host functions written in Python, and audit trees with their explanations. Its API
is the one of the [WebAssembly package](wasm.md), in Python's style: `snake_case` names, keyword
options, a timeout in seconds, and offsets counted in code points.

```sh
pip install jlens
```

The package is in beta until jlens 1.0: its API may still change in a minor release, as the
[changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md) will say. The
[installation page](../getting-started/installation.md#python) lists the platforms it has wheels
for.

The snippets in this chapter come from the scripts in `crates/jlens-python/tests/guide/`, which
the package's tests run.

## Compile once, evaluate many times

```python
{{#include ../../../crates/jlens-python/tests/guide/quick_start.py}}
```

An `Expression` is compiled once and evaluated any number of times, from any thread. Compiling
raises a `JlensError` on syntax errors.

```python
expr = Expression(source)                       # compile once
expr.evaluate(input, bindings, **options)        # Python values in, Python values out
expr.evaluate_json(text, bindings, **options)    # JSON text in, JSON text out
expr.evaluate_document(document, bindings, **options)
expr.source                                      # the source text
```

- **`input`** is the input document: `dict`, `list`, `tuple`, `str`, `int`, `float`, `bool` or
  `None`, nested in any way. `None`, or no argument, means there is no input document, so `$` is
  undefined. See [how the input is read](#how-the-input-is-read).
- **`bindings`** is a `dict` of variables and functions: `{"limit": 10}` makes `$limit`
  available, and a callable becomes a [host function](#host-functions).
- **`options`** are keyword arguments: the [limits](#limits) and the clock and seed that make an
  evaluation reproducible.
- **The result** is a plain Python value: `dict`, `list`, `str`, `int`, `float`, `bool` or
  `None`. A number that is integral and below 10<sup>21</sup> in magnitude becomes an `int`, as
  `json.loads` would read it from jsonata-js's output; other numbers become a `float`.

### Undefined and null

JSONata tells an undefined result (the expression produced nothing) from `null`. `evaluate`
returns `None` for both:

```python
{{#include ../../../crates/jlens-python/tests/guide/undefined.py}}
```

`evaluate_json` keeps them apart: it returns `None` for an undefined result, and the text
`"null"` for `null`.

### Options

```python
{{#include ../../../crates/jlens-python/tests/guide/options.py:options}}
```

- `timeout` (seconds, a `float`) stops the evaluation with `D1012`.
- `max_depth`, `max_string_length`, `max_array_length` and `max_result_size` bound the
  evaluation; see [limits](#limits).
- `now` fixes `$now()` and `$millis()`: milliseconds since the epoch, a `datetime`, or an
  ISO 8601 timestamp. A `datetime` without a time zone is local time, as its `timestamp()`
  reads it.
- `seed` (an `int`) fixes `$random()` and `$shuffle()`.

An unknown option raises a `TypeError`, and a value out of range a `ValueError`.

## How the input is read

`evaluate` reads its input during the evaluation, one `dict` or `list` at a time, when the
expression first reaches it, as the WebAssembly package does
([ADR 0010](../adr/0010-lazy-input.md)). A path such as `customer.name` reads two small dictionaries, however large the rest of the
document is.

| Python | JSONata |
|---|---|
| `dict` and its subclasses, with `str` keys | object, in the order of the keys |
| `list`, `tuple` | array |
| `str` and its subclasses | string |
| `int`, `float` and their subclasses (an `IntEnum`, say) | number; NaN and the infinities become `null`, as in JSON; an `int` beyond 2<sup>53</sup> the nearest number, as in jsonata-js |
| `bool` | boolean |
| `None` | `null` |

Anything else, such as a `set`, a `date`, a key that is not a `str`, or a cycle, has no JSON
form. Two consequences follow from reading the input as the expression goes:

- **Errors appear when their part is read.** A value that cannot be read stops the evaluation
  with `U1004`, with the message `json.dumps` would give, when the expression reads the
  dictionary or list that holds it. An expression that never reaches it succeeds (below).
- **The input must not change during the evaluation.** A [host function](#host-functions), or
  another thread, that modifies the input can change what the rest of the evaluation reads. In
  free-threaded Python, that gives a result mixing both states, or `U1004`, never a crash. Copy
  the input first if something must modify it.

```python
{{#include ../../../crates/jlens-python/tests/guide/input.py:input}}
```

Evaluations never modify their input. A lone surrogate in a `str`, which UTF-8 cannot hold,
becomes U+FFFD.

### JSON text

`evaluate_json` takes the input as JSON text and returns the result as JSON text. For a document
you already have as text, such as an HTTP response body, it avoids building Python objects only
to read them:

```python
{{#include ../../../crates/jlens-python/tests/guide/json_text.py}}
```

Invalid JSON raises a `ValueError`.

### Reusing a document

`evaluate` reads the parts of the input that the expression reaches, on every call. To evaluate
several expressions against one document (a set of rules, say), or one expression several
times, convert the document once:

```python
{{#include ../../../crates/jlens-python/tests/guide/document.py:document}}
```

Building a `Document` converts the whole value at once, with the rules above, and raises a
`ValueError` or a `TypeError` for what it cannot convert. Evaluations never modify a document.

## Threads

`Expression` and `Document` objects can be shared between threads. `evaluate_json` and
`evaluate_document` let other Python threads run while they evaluate: they release the GIL,
and attach again only to call a host function written in Python. `evaluate` reads Python
objects as it goes, so it holds the GIL throughout.

```python
{{#include ../../../crates/jlens-python/tests/guide/threads.py}}
```

The package supports free-threaded Python (3.14t and later), where the threads of a pool
evaluate in parallel. A `Document` used by several threads is converted once more on each
thread, the first time that thread evaluates against it.

## Host functions

Python callables, in the bindings or registered on an expression, are called from the expression
like built-in functions:

```python
{{#include ../../../crates/jlens-python/tests/guide/host_functions.py:host_functions}}
```

- **Arguments and the result** are converted like the input. `None` stands for undefined, in
  both directions: a missing argument arrives as `None`, and a function that returns `None`
  gives an undefined result. A function argument arrives as a `jlens.Function`
  ([below](#functions-as-arguments)).
- **A signature** (`register_function`'s third argument) checks the arguments before each call,
  with the errors of the built-in functions (`T0410`, `T0412`). A parameter with `-` receives the
  context value when the argument is missing: `<n-:n>` makes `price.$double()` work. An invalid
  signature raises a `JlensError`.
- **The number of positional parameters** before the first with a default value tells
  higher-order functions how many arguments to pass: `$map` passes the index only to functions
  with two or more. A callable whose signature `inspect` cannot read counts as having none.
- **Any callable** works: a function, a lambda, a method, a class, or a built-in such as `len`.
- **Asynchronous functions are not supported.** A coroutine function fails with `U1002`. Fetch
  what the expression needs before evaluating it.
- **Precedence:** a function in the bindings replaces a registered function of the same name,
  and both replace a built-in function.

### Exceptions

An exception raised by a host function is raised again by the evaluation as is, the same
object, with `position` and `token` attributes added when it has none:

```python
{{#include ../../../crates/jlens-python/tests/guide/host_exceptions.py:host_exceptions}}
```

`evaluate_with_audit` reports it in its `error`, with its `code` attribute (or the name of its
type) and its text. Inside `$eval`, it becomes the error of `$eval` (`D3121`).

### Functions as arguments

A host function can call a JSONata function it receives, such as a step to retry or a fallback:
a lambda, a built-in function, a partial application, a chain or another host function. It
arrives as a `jlens.Function`:

```python
{{#include ../../../crates/jlens-python/tests/guide/functions_as_arguments.py:functions_as_arguments}}
```

- **The call is part of the evaluation**, as when `$map` calls its function. The context value
  is `null`, and a lambda keeps the variables of its definition. The call counts as a level of
  depth, so a recursion through a host function ends with `D1011`; it checks the timeout; its
  result is limited by `max_result_size`.
- **Arguments and the result** are converted as for a host function. Its `arity` attribute is
  the number of parameters the function declares.
- **Errors are raised** as a `JlensError` with their codes, or as the exception a host function
  raised. The host function can catch them, or let them through, which ends the evaluation with
  that error.
- **It is valid until the host function returns.** Called later, or from another thread, it
  raises a `JlensError` with code `U1005`.

## Errors

Errors are `jlens.JlensError` exceptions, a subclass of `Exception`. They carry the JSONata error
fields; every offset counts code points, the unit of `str`, so `source[error.start:error.end]`
is the part of the source the error is about:

```python
{{#include ../../../crates/jlens-python/tests/guide/errors.py:errors}}
```

Evaluation errors carry `value` too, when there is one (for `T2001`, the value that is not a
number):

```python
{{#include ../../../crates/jlens-python/tests/guide/errors.py:error_value}}
```

`str(error)` is the message. Match on `code`, never on the message. Arguments of the wrong type,
such as bindings that are not a `dict`, raise a `TypeError` instead, and a value that cannot be
converted, such as a cyclic binding or invalid JSON text, a `ValueError`. The
[error reference](../language/errors.md) lists every code.

## Explaining results

`evaluate_with_audit` evaluates and records how the result was reached; see the
[audit guide](audit.md). Evaluation errors are reported in the result, not raised:

```python
{{#include ../../../crates/jlens-python/tests/guide/audit.py}}
```

The options are those of `evaluate`, plus two more:
- `level` is `"off"`, `"decisions"` (the default) or `"full"`.
- `limits` sets the capture limits: `max_items`, `max_string`, `max_depth`, `max_rejected`,
  `max_nodes` and `max_source`.

The result is a `dict` (typed as `jlens.AuditResult`) with `result`, `error`, `audit`,
`explanation` and `explanation_text`. `audit` is the audit tree in the JSON format of
`jlens eval --audit json`, with its camelCase field names, and its spans count code points.
`explanation["lines"]` holds structured lines (`depth`, `passed`, `text`, `message`, `node`) for
rendering in a user interface. The input is converted whole first, as for a `Document`.

## Limits

| Limit | Default | Error |
|---|---|---|
| Evaluation depth (`max_depth`) | 50,000 | `D1011` |
| Evaluation time (`timeout`, seconds) | none | `D1012` |
| Length of a string built (`max_string_length`, bytes of UTF-8) | 256 MiB | `U1001` |
| Items of an array built (`max_array_length`) | 16,777,216 | `U1001` |
| Size of the result as JSON text (`max_result_size`, bytes) | 256 MiB | `U1001` |
| Expression nesting | 1,000 levels | `U1001` |
| Nesting of arrays and objects in a value that is returned, compared or serialized | 1,000 levels | `U1001` |

The size of the result counts a value each time it appears, as the JSON text does, and also
limits the arguments passed to a host function. The stack grows as needed, so the depth limit is
that of the native targets; see the [evaluator chapter](../architecture/evaluator.md#guardrails).
There is no cancellation from another thread: use `timeout`.

## Typing

The package ships type stubs and `py.typed`. `Expression`, `Document`, `Function` and
`JlensError` are typed, and so are the values `evaluate_with_audit` takes and returns:
`AuditResult`, `ErrorInfo` and `CaptureLimits`, which are `TypedDict`s. Results of `evaluate` are
`Any`.

## What the package leaves out

The language service and schema inference are not in the Python package: editors reach them
through the [language server](editors.md), and `jlens schema infer` infers a schema from the
command line. They can be added when a Python application asks for them.

## Building from source

The source distribution builds on any platform with Rust 1.88 or later; pip builds it when no
wheel fits, such as for free-threaded Python 3.14. From a checkout, with
[maturin](https://www.maturin.rs/) in a virtual environment:

```sh
cd crates/jlens-python
pip install maturin pytest
maturin develop                          # builds the package into the active environment
python -m pytest -p no:cacheprovider tests
```
