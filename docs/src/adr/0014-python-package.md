# 0014 — The Python package: PyO3 bindings that mirror the WebAssembly package

**Status:** Accepted

## Context

JSONata in Python has three implementations, checked on 2026-10-15: `jsonata-python` 0.7.1, a
port in pure Python; `jsonata` 0.2.5, a wrapper of jsonata-js; and `jsonata-rs` 0.1.4, a young
Rust port of jsonata-java. jlens would bring its conformance, audits and speed to Python, where
data pipelines and rules live. The package has to answer seven questions: which binding, where
its code lives and under which names, which API, how a Python value is read, how threads share
what the package returns, which interpreters its wheels serve, and what it leaves out.

**The binding.** PyO3 is the binding of Rust to CPython, and maturin builds its wheels. Both
are what other Rust packages on PyPI use (`pydantic-core`, `orjson`, `polars`). The workspace
forbids `unsafe_code`, and PyO3's macros expand to code that calls CPython's C API. A spike
(PyO3 0.30.1, maturin 1.15.0, Rust 1.99, Windows x86-64) answered that question: a module with
`#[pyclass]`, `#[pymethods]` and `#[pymodule]` compiles under `unsafe_code = "forbid"` with no
warning, and an `unsafe` block written in the crate is still refused. The unsafe code is inside
PyO3, as it is inside `wasm-bindgen` for the WebAssembly package.

**Reading a Python value.** The WebAssembly package reads its input one level at a time through
`Source` (ADR 0010). The spike read Python values the same way (a `dict` as an object, a `list`
or `tuple` as an array, cycles detected through the chain of parents) and compared it with the
other ways to give jlens a document. Microseconds per evaluation, best of five, Python 3.14.8,
release build, on a document of items with five fields each (`id`, `name`, `price`, a list of
tags, a nested object):

| Expression | Python values, one level at a time | `json.dumps`, then the JSON text | Parsed JSON, read lazily | `Document` |
|---|---:|---:|---:|---:|
| 1,000 items (106 KB of JSON) | | | | |
| `items[0].name` | 222 | 3,872 | 62 | 4.1 |
| `$count(items)` | 216 | 3,608 | 1.3 | 0.9 |
| `$sum(items.price)` | 1,770 | 4,595 | 551 | 37 |
| `items[price > 98].id` | 1,842 | 4,603 | 654 | 135 |
| `$` | 6,429 | 6,079 | 2,103 | 2,352 |
| 100,000 items (11 MB of JSON) | | | | |
| `items[0].name` | 36,199 | 450,707 | 9,961 | 2,480 |
| `$sum(items.price)` | 235,319 | 539,915 | 90,994 | 13,225 |
| `$` | 846,609 | 847,078 | 359,145 | 381,154 |

`json.dumps` alone takes 1,604 µs on 1,000 items and 213,087 µs on 100,000. Reading levels is
2 to 17 times faster than the round trip through JSON text when the expression reads part of
the document, and costs about the same (6% more at worst) when it reads all of it, since
`json.dumps` visits every value too. Unlike the WebAssembly package, no level is worth reading
whole through JSON text. A result converted straight into Python objects costs 15% less than a
result written as JSON text and read back with `json.loads` (6,079 against 7,133 µs for `$` on
1,000 items).

Python values have no JSON form for everything. The spike stopped the evaluation with `U1004`,
the error of a level that cannot be read, for a cycle (`Circular reference detected`, as
`json.dumps` says), a `set`, a `date` or a key that is not a `str`, with `json.dumps`'s message.
An `int` beyond 2^53 becomes the nearest number, as in jsonata-js. NaN and the infinities become
`null`, as the WebAssembly package does with `JSON.stringify`.

**Threads.** `jlens_core::Expression` is `Send` and `Sync`. `jlens_core::Document` holds
`Rc`s and is neither. A PyO3 class that is not `Send` is declared `unsendable`, and the spike
showed what that costs: an `unsendable` `Document` used from another thread raises
`PanicException`, which derives from `BaseException` (so `except Exception` misses it), and
prints a Rust panic on standard error. Building a document once and sharing it with a pool of
worker threads is a common Python pattern, so that would be a trap. Making the class `Send` by
hand needs `unsafe impl Send`, which the workspace forbids. The "parsed JSON, read lazily" column
is the other way: a `serde_json::Value` behind an `Arc` is `Send` and `Sync`, and each evaluation
reads it lazily, 4 to 15 times slower than a `Document` when the expression reads the items.

**Free-threaded Python** (PEP 703) runs threads without the GIL. A module declares that it
supports it (`#[pymodule(gil_used = false)]`), and its classes must then be safe without the
GIL, which `Send + Sync` classes are. The stable ABI (`abi3`) does not cover free-threaded
builds; PEP 803 adds `abi3t` from Python 3.15. The spike built three wheels of the same module:
`cp39-abi3` for Python with the GIL, `cp314-cp314t` for 3.14 without it, and
`cp315-abi3.abi3t` (PyO3's `abi3t-py315` feature), which served Python 3.15.0 free-threaded,
with the GIL disabled. Each wheel takes 0.9 MB on Windows x86-64.

**A bug the spike found.** A `Source` input 20,000 levels deep, returned whole (`$` or
`$string($)`), overflows the stack in jlens-core: the conversion of the result into JSON
(`Value::to_json_spending`) and the serialization of `$string` recurse without
`stack::grow`. `$count($)` on the same input works. The bug is not Python's: a pure Rust
`Source` reproduces it, and the Python package would expose it to any nested list.

## Decision

- **PyO3 and maturin, under the workspace's lints.** The crate keeps `unsafe_code = "forbid"`
  and `missing_docs`, as every crate of the workspace does.
- **The crate is `crates/jlens-python`**, in the workspace, not published to crates.io. It is a
  maturin project in the mixed layout: the native module `jlens._jlens` and a Python package
  `jlens` beside it in `crates/jlens-python/python/jlens/`, which re-exports the module and holds
  the type stubs (`_jlens.pyi`) and `py.typed`. **The PyPI name is `jlens`** (free on
  2026-10-15), with the workspace's version, in beta (`Development Status :: 4 - Beta`) until
  1.0, as the WebAssembly package is.
- **The API mirrors the WebAssembly package's**, in Python's style (`snake_case`, keyword
  arguments):
  - `Expression(source)`, with `evaluate(input=None, bindings=None, **options)`,
    `evaluate_json(text, bindings=None, **options)` (the input and the result as JSON text),
    `evaluate_document(document, ...)`, `evaluate_with_audit(...)`, `register_function(name,
    implementation, signature=None)` and `source`.
  - `Document(value)` and `Document.from_json(text)`.
  - The options are those of `EvaluateOptions`: `timeout` in seconds (a `float`, as Python's
    own APIs take it), `max_depth`, `max_string_length`, `max_array_length`, `max_result_size`,
    `now` (milliseconds since the epoch, a `datetime` or an ISO 8601 string) and `seed`.
  - Errors raise `jlens.JlensError`, a subclass of `Exception`, with `code`, `position`,
    `start`, `end`, `token` and `value`. Its offsets count code points, the unit of Python's
    `str`, so that `source[error.position]` points where jsonata-js's UTF-16 offset points.
  - `evaluate` returns `None` for an undefined result, and for `null`. `evaluate_json` tells
    them apart: `None` for undefined, `"null"` for `null`.
  - The language service (`LanguageService`, `infer_schema`) is left out: editors reach it
    through the language server and the VS Code extension. It can be added when a Python
    application asks for it.
- **`evaluate` reads its Python input one level at a time**, through `Source`, as the
  WebAssembly package does, with the conversions above: `dict` (and its subclasses) as an
  object, `list` and `tuple` as arrays, `str`, `int`, `float`, `bool` and `None`; anything else
  stops the evaluation with `U1004`. The input must not change during the evaluation: a change
  can give a result mixing both states, or `U1004`, never a crash. Results are converted
  straight into Python objects: an integral number that fits in 64 bits becomes an `int`, as
  `json.loads` would give it, and other numbers a `float`.
- **`Document` can be shared between threads.** It holds the parsed JSON (`Arc<serde_json::Value>`)
  and is a `frozen` class. Each thread converts it into a `jlens_core::Document` the first time
  it evaluates against it, and keeps that copy in a thread-local cache, keyed by a `Weak`
  reference to the JSON: a later evaluation on that thread runs at the speed of a `Document`.
  The cache drops a copy once its document is gone, at the thread's next evaluation, or when
  the thread ends. Building a `Document` from a Python value converts it whole, with the same
  rules as `evaluate`.
- **The module supports free-threaded Python** (`gil_used = false`). `evaluate_json` and
  `evaluate_document` detach from the interpreter while they evaluate, so that other Python
  threads run meanwhile, and attach again to call a host function written in Python. `evaluate`
  stays attached, since it reads Python objects as it goes.
- **Wheels:** for each release target (Linux x86-64 and ARM64, manylinux and musllinux; macOS
  x86-64 and ARM64; Windows x86-64), one `abi3` wheel for Python 3.10 and later with the GIL,
  and one `abi3t` wheel for free-threaded Python 3.15 and later; and a source distribution.
  Free-threaded 3.14 builds from the source distribution, which needs Rust. **The oldest Python
  supported is 3.10**: CPython stopped supporting 3.9 in 2025, and 3.10 is still the Python of
  current long-term distributions (Ubuntu 22.04). With `abi3`, an older floor costs one more
  version in the test matrix, not one more wheel; it is raised in a minor release, when PyO3
  drops it or few users remain.
- **The deep-input bug is fixed in jlens-core before the binding** (roadmap 0.17 #2): the
  conversion of results and `$string` go through `stack::grow`, with a robustness case on a
  deep `Source`.

## Consequences

- A Python user installs one wheel of under 1 MB, with nothing to compile, on the release
  targets. Other platforms and free-threaded 3.14 need a Rust toolchain.
- The Python and WebAssembly packages have the same objects, options and errors, so the book
  can describe them together, and an application can move between them. Where Python's style
  differs (`snake_case`, seconds, code-point offsets), the Python chapter says so.
- `evaluate` on Python values is the default and the fast path for a document evaluated once.
  A document evaluated many times belongs in a `Document`, 2 to 240 times faster per
  evaluation (twice as fast when the result is the whole document), at the cost of one
  conversion per thread.
- Each thread that evaluates against a `Document` holds its own converted copy until the
  document is dropped. A pool of many threads using one large document multiplies its memory
  by the number of threads.
- The workspace builds `jlens-python` with `cargo build` like any crate; the pytest suite needs
  maturin and a Python, which `cargo test` does not. Its CI and release steps are 0.17 #3 to #6.
- jlens-core gains a fix for every `Source` input, the WebAssembly package's included.
