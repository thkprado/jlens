# 0017 — Results written straight into another representation

**Status:** Accepted.

## Context

[ADR 0010](0010-lazy-input.md) reads an input in its own representation, through
`input::Source`, so that a JavaScript object or a Python `dict` is not converted to
`serde_json` first. The result takes no such path: every entry point returns a
`serde_json::Value`, which the WebAssembly and Python packages then convert again into their own
objects.

Roadmap 0.19 measured what that costs in Python (WSL, Python 3.14.8, jlens 0.18.0). The object
constructor case of `benches/compare/python.py`, 200 objects of three fields, takes 367 µs in
jlens and 195 µs in `jsonatapy`. Without its result converted it evaluates faster in jlens
(`$count(items.{...})`: 155 µs against a `Document`, 177 µs in `jsonatapy`). Converting the
200 objects costs about 138 µs, in two passes:

- `Value::to_json_spending` builds a `serde_json::Value`: a `String` for each key and string, and
  a map that hashes each key with SipHash.
- `to_py` walks it and builds Python objects, with a new `str` for each key of each `dict`, which
  Python hashes again when it inserts it.

A string of the result costs 90 ns, against 24 ns in `jsonatapy`.

Three facts bound the design:

- **The size limit of a result stays.** A result whose compact JSON text would be longer than
  `Options::max_result_size`, or nested more than 1,000 levels deep, fails with `U1001`.
  The limit is counted while converting, which stops as soon as it is exceeded, so a huge result
  never gets built.
- **Converting a result reads what the evaluation left unread.** A part of the input returned
  as is (`customer`) is written straight from its source, and a level that its source then fails
  to read fails the evaluation with `U1004`.
- **The Python package evaluates a `Document` without holding the interpreter**
  (`py.detach`), so that other threads run. Python objects can only be built while attached
  again, and jlens values cannot leave the evaluation: they are not `Send`, and they borrow
  the input.

## Decision

1. **A public trait, `jlens_core::output::Builder`, builds a result one value at a time.** It is
   the counterpart of `input::Source`, and mentions no internal type. It has associated types
   for a value, an array being built and an object being built, and methods to build `null`, a
   boolean, a number, a string, and to start, fill and finish an array or an object. The core
   walks the result depth first, as `JSON.stringify` would: undefined values and functions are
   left out of objects and written as `null` in arrays.
   - **Numbers are `f64`, always finite.** A number that is not finite is written as `null`,
     as JSON has none. The builder decides how to represent an integer: Python writes an
     integral number below 10^21 as an `int`, as it does now.
   - **Keys are given as `&str`.** The core does not say which keys are the same: a builder whose
     keys cost to create (a Python `str`, a JavaScript string) keeps them itself, in a table
     for the whole result. Keys repeat across the objects of a result far more than they differ.
   - **The methods cannot fail.** Building a value in memory fails only when memory runs out. A
     builder whose representation can fail (Python raises `MemoryError`) keeps its first
     failure, returns any value, and reports it once the evaluation has returned. The trait then
     needs no error type, and the core no way to carry one.
2. **The entry points give the result to a closure, as an `output::Output`.**
   `Expression::evaluate_source_to` and `Expression::evaluate_document_to` take the arguments of
   `evaluate_source` and `evaluate_document`, and a closure called once the evaluation has
   succeeded. The closure writes the result with `Output::write(&mut builder)`, which applies
   the size and nesting limits and the read failures above, and its return value is returned.
   The Python package calls `evaluate_document_to` detached, and attaches once, inside the
   closure, to write the result.
3. **The `serde_json` result goes through the same walk.** `evaluate` and the other entry
   points write their result with a builder of `serde_json` values, private to the core. There is
   one walk over results, that every test and the conformance suite exercise, rather than one
   for `serde_json` and another for builders. The parts of the input returned unread are
   written through the builder too, from their `serde_json` value or their source.
4. **The packages adopt it where it pays.** The Python package implements a builder over
   Python objects, with one `str` per key for the whole result. The WebAssembly package
   adopts it if a measurement shows a gain over building `serde_json` values and converting
   them through JSON text and `JSON.parse`.

## Alternatives not taken

- **`serde::Serialize` for results.** A result could implement `Serialize`, and the packages
  write it with `pythonize` or `serde-wasm-bindgen`. The size limit would have to be counted
  inside `serialize`, and its failure carried through the serializer's error type. The
  serializers create each key anew, which is the cost this ADR removes, and the serde data model
  is wider than JSON's. A serializer straight to JSON text could still use it later, for
  `evaluate_json`.
- **An owned result returned to the caller, written later.** The result borrows the input and
  holds `Rc`s: it cannot outlive the evaluation, nor leave a thread. A closure keeps it inside
  the call.
- **A builder passed as an argument, without a closure.** It is simpler for a caller that holds
  everything the builder needs. It does not serve the Python package's `evaluate_document`,
  whose builder can only exist once the evaluation is done and the interpreter attached again.
- **An error type on the trait.** It would make every method return a `Result` and the entry
  points carry a second kind of error, for failures that only running out of memory causes.

## Consequences

- An application can have results in its own representation without a `serde_json` pass. The
  `serde_json` entry points keep their signatures and behaviour.
- The walk over results becomes generic: it is compiled once per builder. `cargo xtask ab`
  checks that the `serde_json` builder costs what the conversion it replaces did.
- A builder does not see the evaluation's values, only JSON: the value model stays private,
  as ADR 0010 kept it.
- The tracer interface still sees results through `audit::ValueRef`, as JSON.
- The WebAssembly package adopted it in 0.19. Like its input, read one level per call to a
  JavaScript helper (ADR 0010), its builder writes the result as a list of operations, with
  its numbers and strings, and one call to a JavaScript helper builds the values, decoding
  each key once. The object constructor case evaluates against a `Document` in 305 µs instead
  of 466 µs. `evaluateJson` still builds `serde_json` values: a builder gets each key after
  its value, so it cannot write the JSON text in order.
