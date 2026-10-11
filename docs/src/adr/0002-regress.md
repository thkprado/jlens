# 0002 — Use `regress` for ECMAScript-compatible regular expressions

**Status:** Accepted

## Context

JSONata regex literals (`/ab+c/i`) and the functions `$match`, `$replace`, `$split` and
`$contains` follow **JavaScript** regular-expression semantics. The suite relies on features
such as backreferences, lookahead and lookbehind, and on JavaScript's capture-group behaviour.

The `regex` crate deliberately omits backreferences and lookaround, because it guarantees
linear-time matching. `fancy-regex` adds them, but its syntax and semantics are Perl/PCRE-like
rather than ECMAScript.

## Decision

Use [`regress`](https://crates.io/crates/regress). It implements ECMAScript regular-expression
syntax and semantics in pure Rust, and it compiles to `wasm32-unknown-unknown`.

## Consequences

- We get regex behaviour that matches jsonata-js closely, including in the browser build.
- `regress` is a backtracking engine, so a pathological pattern can take exponential time. The
  evaluation timeout guards against this (see `Options::timeout`), and the guides will warn about
  untrusted patterns. (It did not: a match ran as one evaluation step.
  [ADR 0009](0009-bounded-regex.md) makes the timeout reach inside a match.)
- Positions reported by `$match` must be converted from UTF-8 byte offsets to the UTF-16 code
  unit indices that JavaScript reports. That conversion is covered by dedicated tests.
