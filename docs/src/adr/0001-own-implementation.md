# 0001 — Implement JSONata ourselves, porting jsonata-js 2.2.2

**Status:** Accepted

## Context

The project began as a rule engine built on a home-grown JSON DSL over JSONPath. It had
`all`/`any`/`not` groups, `vars`, aggregate formulas such as `sumProduct` and `avgField`, and an
audit tree. Each new requirement added more operators to that DSL, and it was drifting towards an
under-specified programming language. JSONata already offers everything that DSL needed, and
more:
- path navigation and filters;
- aggregation;
- variables;
- conditionals and result construction.

JSONata also has a specification and a language-neutral conformance suite.

We considered two options:

1. **Build on an existing Rust JSONata crate.** That is less work up front. However, the existing
   crates cover the language only partially. They don't expose an AST with stable spans or a
   recovering parser, and we would depend on a third party for the parts we need to extend most:
   auditing and tooling.
2. **Write our own implementation** as a port of the reference implementation, jsonata-js 2.2.2.

## Decision

We write our own implementation and port the reference implementation stage by stage:
tokenizer, Pratt parser, `processAST`, evaluator, function library, date/time formatting and
signatures.

The Java port ([jsonata-java](https://github.com/dashjoin/jsonata-java)) and its annotated walkthrough
serve as a secondary reference, especially for the pitfalls they document.

Correctness is measured against the official test suite (see
[Conformance testing](../architecture/conformance.md)).

## Consequences

- We control the AST, which lets us add precise spans, a recovering parser, trivia and comment
  annotations, and evaluator instrumentation for audits.
- The cost is substantial: roughly 7,700 lines of JavaScript to port, including the date/time
  picture-string engine.
- Conformance is objective and tracked per case, so behaviour differences are visible and must
  be justified in `tests/conformance/overrides.json`.
- We track upstream JSONata releases by re-vendoring the suite (`cargo xtask update-suite`).
