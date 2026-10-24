# 0009 — Regular expressions that stop within the evaluation's limits

**Status:** Accepted; [ADR 0020](0020-own-regress-fork.md) makes the fork permanent, superseding
the plan in decision 3 to go back to `regress`

## Context

[ADR 0002](0002-regress.md) chose `regress`, a backtracking engine, and counted on the
evaluation timeout to stop pathological patterns. It does not. The evaluator checks the timeout
and the cancellation token between evaluation steps (`Evaluator::guardrails` in `eval.rs`), and a
match is a single step. Measured on 2026-10-11, in a debug build with
`jlens eval --no-input --timeout 100`, `$contains('aa…a!', /^(a+)+$/)` with n a's
ran for 1.2 s at n = 20, 9.9 s at n = 23 and 80 s at n = 26. In WebAssembly the page
freezes for that long. Untrusted expressions were the use case the 0.5 limits were built for,
and a regex literal gets through them.

What `regress` offered when this was decided (0.10.5, the version in use, and 0.12.0, the
latest):

- **No limit and no hook.** Neither the public API nor the backtracker
  (`classicalbacktrack.rs`) has a step budget, a deadline, or a callback.
- **No linear-time backend.** The optional `backend-pikevm` is not a Pike VM in the automata
  sense: it explores states depth-first on a stack, cloning each state, and never merges
  duplicates, so it is exponential on the same patterns. It also panics on an instruction
  (`Loop1CharBody`) that the optimizer emits, and it is `#[doc(hidden)]`.

Choices to make:

1. **How a match stops.** Use a linear-time engine where the pattern allows one, give each match
   a step budget, or have the engine poll the evaluation's own limits.
2. **What the error is.** The timeout's and cancellation's codes (`D1012`, `U1003`), or a new
   limit (`U1001`).
3. **How the engine change ships**, since `regress` cannot do it today.

## Decision

1. **The backtracker polls the evaluation's limits.** jlens matches through a fork of `regress`
   that takes an optional interrupt check: a callback the backtracker calls every *N* steps,
   which returns whether to stop. When it says to stop, the match returns an "interrupted"
   result instead of a match or no match.
   - A step is a unit of work that bounds everything between two checks: backtracks, and the
     instructions run between them, nested lookarounds included. The exact unit and *N* are
     settled by measurement during implementation. Targets: on the catastrophic cases, under
     1 ms between two checks in a release build; on `benches/`, under 2 % slower.
   - As implemented in `jlens-regress`: a step is a failure that sends the backtracker back to
     its stack, or an iteration of a loop, and *N* is 1,024. Measured with
     `cargo run --release -p jlens-regress --example interrupt_cost` on 2026-10-11: on five
     catastrophic patterns, a median of 9 to 13 µs between two checks (maximum about 0.2 ms,
     scheduling noise included); on ordinary matches, within the noise of the run (±4 %, a few
     nanoseconds per call). Measured again after the rebase onto 0.12.0: the same times between
     checks, and the same noise on ordinary matches except one, a backreference that backtracks
     on every word (`(\w+)\s+\1` on prose), 10 % slower than `find_from` against 0 % on 0.10.5.
     The code paths are the same; the difference comes from how the compiler inlines the
     backtracker once the check is in. It applies only to evaluations with a timeout or a token.
   - Measured again for 0.22 (2026-10-24): the example's two sides are two copies of the
     backtracker in one binary, which the release profile (`lto = true`, `codegen-units = 1`)
     inlines and places anew on every change to the crate. A change that runs no differently on
     these patterns moved `find_from` by 4 to 7 %, and the interruptible side of one case by
     34 % when a single file was reverted. With functions and blocks aligned
     (`RUSTFLAGS="-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5"`),
     which takes the placement out, two revisions measure the same within 1 %, and the check
     costs 2 to 4.5 % on ordinary matches (the anchored one is 6 % faster with it). That is the
     cost this decision accepts. The 2 % target holds at jlens's level, where `cargo xtask ab`
     measures the evaluations that match through the check (`regex` 2.7 % faster across the
     engine fixes of 0.22 item 4).
   - Measured with the engine's benchmark (0.22, `crates/jlens-regress/benches/engine.rs`), whose
     two variants are binaries of their own: in one binary, two copies of the backtracker that
     both count nothing differed by up to 16 %, so the example's comparison measured how each
     copy was inlined more than the check. The counter was a field in memory, and every step
     waited for the store of the one before: 2.5 % on average over 17 cases, 7 to 11 % on some.
     Each attempt now counts on a local copy, kept in a register and written back when the
     attempt ends or enters a lookaround: 1.7 % on average, from 4.6 % faster to 9.5 % slower.
     The check itself costs about 7 % on a backreference that backtracks one character at a
     time; the rest of the spread is register allocation, which any change to the backtracker
     redraws: a variant with the interrupt's code that counts nothing is 12.8 % slower on one
     case and 9 % faster on another. The 2 % target holds on average; the performance page
     records each case.
   - jlens passes a check only when the evaluation has a timeout or a cancellation token. Without
     either, the match runs the unchanged code path (the check is a type parameter, so the
     "no check" version compiles to the current code). As with the tracer (ADR 0004), a feature
     that is not used costs nothing.
   - Every match that user input reaches goes through it: the regex function values (`exec` and
     `next` in `regex.rs`, which `$match`, `$replace`, `$split` and `$contains`
     all use) and the regex that `$toMillis` builds from a picture (`datetime.rs`). The
     fixed internal patterns in `datetime.rs` need no check.
2. **The error is the limit that stopped it.** A match that the timeout stops fails with `D1012`,
   with the evaluation's timeout as its value. One that a cancellation stops fails with `U1003`.
   These are the codes the same limits raise between steps, so callers handle nothing new and
   there is no new option.
3. **The fork is a workspace crate, `jlens-regress`,** published on crates.io before
   `jlens-core`.
   - It is `regress` plus the interrupt API, and nothing else. The diff stays small so that it
     is easy to review and to rebase. It started from 0.10.5 and was rebased onto 0.12.0 on
     2026-10-11 for the fixes upstream made in between: JavaScript semantics (the position
     restored when backtracking out of a lazy loop, `i` case folding, nested and negated
     classes, `\x` as an identity escape, duplicate named groups), and a nesting limit in the
     parser (256 levels), which turns a pattern that overflowed the stack into a syntax error.
   - It keeps the upstream licences (MIT or Apache-2.0) and says in its README that it is a
     fork, of which version, and why.
   - jlens builds it with `regress`'s `prohibit-unsafe` feature, so that no `unsafe` path runs.
     The workspace lint `unsafe_code = "forbid"` does not apply to it, because the upstream code
     keeps its `unsafe` blocks behind `cfg!` checks.
   - The same API is proposed upstream as an issue and a pull request at
     `ridiculousfish/regress`. The maintainer opens them. When a `regress` release offers an
     equivalent, jlens goes back to `regress` and deprecates `jlens-regress`.

No default timeout is added, in Rust, the CLI or WebAssembly. Long legitimate evaluations would
start failing, and the three would behave differently. The guides keep telling applications that
run untrusted expressions to set a timeout. With one set, a regex can no longer outlast it.

## Alternatives considered

- **A linear-time engine** (`regex` or `regex-automata`) for the patterns it can run, keeping
  `regress` for the rest. It cannot run backreferences or lookaround, so those patterns would
  still need the check. Its semantics differ from JavaScript's in ways a translation would have
  to cover: `.` and line terminators, `i` case folding, `\b` and `\w`, and captures that
  JavaScript resets on each iteration of a quantifier. It would also add Unicode tables to the
  WebAssembly package. It is worth revisiting for speed, but it does not remove the need for the
  check.
- **A separate step budget** with `U1001`, as a new limit. It would be deterministic, but it
  adds an option, it could reject legitimate matches on large inputs that the timeout allows,
  and jsonata-js has nothing like it. The timeout is already the limit for untrusted expressions.
  Tests stay deterministic with a short timeout and a pattern that would take minutes.
- **Static checks on the pattern** (nested quantifiers, star height) with a cap on the input
  length. Such checks are neither sound nor complete: `(a|a)*`, the polynomial `a*a*a*b` and
  backreferences slip through. They would also reject patterns that jsonata-js accepts.
- **Running the match elsewhere** and abandoning it at the deadline. Native threads cannot be
  killed, and the WebAssembly build is single-threaded.
- **Waiting for upstream** instead of forking. That leaves the hole open on a timeline jlens does
  not control. The fork is temporary, and it lands upstream if `regress` accepts it.
- **Copying `regress` into `jlens-core`.** That avoids a new crate, but it adds about 10,000
  lines of engine and 31,000 lines of Unicode tables to the core crate. It would also mean
  rewriting every `unsafe` block to satisfy the `forbid` lint, which makes the copy hard to sync
  with upstream.

## Consequences

- A catastrophic pattern stops within the timeout, give or take the time between two checks, in
  Rust, in the CLI (`--timeout`) and in WebAssembly. A cancellation token also stops it, from
  another thread. In WebAssembly only the timeout can, because the page's single thread is busy
  matching.
- This differs from jsonata-js, which has no timeout of its own and hangs on such a pattern.
  The compatibility page documents the difference.
- The release publishes one more crate, first: the publish order in `release.yml` and the
  release checklist in CONTRIBUTING change. `cargo deny` checks its licence like any dependency.
- Upgrading `regress` now means rebasing the fork. The fork's tests run every backtracking match
  of upstream's suite through the interrupt API as well, and the conformance baseline and
  `cargo xtask differential` guard against changes in behaviour.
- After 0.12.0, upstream added `Regex::find_from_budgeted` (commit `50bead7`, unreleased as of
  2026-10-11): a count of backtracks that ends the search with "budget exhausted". It is not an
  equivalent: a count cannot follow a deadline or a cancellation, and a search restarted with a
  new budget starts over. The upstream proposal should build on it, for example with a callback
  when the budget runs out.
- This ADR replaces ADR 0002's statement that the evaluation timeout guards against
  pathological patterns. The rest of ADR 0002 stands.
- Property tests generate patterns with nested quantifiers on inputs that cannot match, and
  check that each evaluation ends within its timeout with `D1012`.
