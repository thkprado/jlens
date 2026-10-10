# Summary

[Introduction](introduction.md)

# Getting started

- [Installation](getting-started/installation.md)
- [First steps](getting-started/first-steps.md)

# Language reference

- [Overview](language/overview.md)
- [Paths](language/paths.md)
- [Building results](language/construction.md)
- [Operators](language/operators.md)
- [Sorting, grouping and aggregation](language/grouping.md)
- [Programming](language/programming.md)
- [Regular expressions](language/regex.md)
- [Dates and times](language/date-time.md)
- [Function library](language/functions.md)
  - [String functions](language/functions/string.md)
  - [Numeric functions](language/functions/numeric.md)
  - [Aggregation functions](language/functions/aggregation.md)
  - [Boolean functions](language/functions/boolean.md)
  - [Array functions](language/functions/array.md)
  - [Object functions](language/functions/object.md)
  - [Higher-order functions](language/functions/higher-order.md)
  - [Date and time functions](language/functions/date-time.md)
  - [Encoding functions](language/functions/encoding.md)
  - [Other functions](language/functions/misc.md)
- [Error reference](language/errors.md)
- [Audit annotations](language/annotations.md)
- [Compatibility with jsonata-js](language/compatibility.md)

# Guides

- [Command-line interface](guides/cli.md)
- [Rust crate](guides/crate.md)
- [WebAssembly (browser)](guides/wasm.md)
- [Audit and explainability](guides/audit.md)
- [Schema and autocomplete](guides/schema.md)

# Architecture

- [Overview](architecture/overview.md)
- [Parsing](architecture/parser.md)
- [Evaluation](architecture/evaluator.md)
- [Schema and language service](architecture/language-service.md)
- [Roadmap](architecture/roadmap.md)
- [Conformance testing](architecture/conformance.md)
- [Performance](architecture/performance.md)
- [Decision records](adr/README.md)
  - [0001 Own JSONata implementation](adr/0001-own-implementation.md)
  - [0002 ECMAScript regex via regress](adr/0002-regress.md)
  - [0003 Value and sequence model](adr/0003-value-model.md)
  - [0004 Auditing through a tracer](adr/0004-audit-tracer.md)
  - [0005 Audit annotations in comments](adr/0005-comment-annotations.md)
