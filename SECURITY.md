# Security policy

## Supported versions

jlens is before 1.0: security fixes go into the latest release only. Update to it before
reporting a problem.

## Reporting a vulnerability

Report vulnerabilities privately, through
[GitHub's private vulnerability reporting](https://github.com/thkprado/jlens/security/advisories/new),
not in a public issue. Include the version, the expression and input that trigger the problem,
and what happens.

You should receive an answer within a week. Once a fix is released, the advisory is published
with credit to the reporter, unless they prefer otherwise.

## Scope

jlens evaluates expressions and documents that may come from untrusted users. The following are
vulnerabilities:

- a panic, a stack overflow or an abort on any expression or input;
- an evaluation that exceeds a limit it was given (depth, time, string or array length) without
  stopping with `U1001`, or that ignores cancellation;
- an evaluation that modifies its input document, or reads anything but its input, its bindings
  and the functions the application registered;
- memory use out of proportion to the expression and input that a limit does not cover.

Without limits, an expression can run for a long time or build large values: set them for
untrusted expressions. See [Guardrails](docs/src/guides/crate.md#guardrails) in the crate guide.
