# 0020 — `jlens-regress` is jlens's own fork of `regress`

**Status:** Accepted. Supersedes decision 3 of [ADR 0009](0009-bounded-regex.md) where it makes
the fork temporary; the rest of ADR 0009 stands.

## Context

[ADR 0009](0009-bounded-regex.md) made `jlens-regress`, a fork of `regress` 0.12.0 (upstream
commit `aae35de`) with interruptible matches, a temporary crate. The interrupt API was to be
proposed upstream as an issue and a pull request. When a `regress` release offered an
equivalent, jlens was to go back to `regress` and deprecate the fork. Until then, upgrades were
to be rebases of the fork onto new upstream versions.

What happened since, as of 2026-10-24:

- **The proposal has no answer.** In
  [regress#140](https://github.com/ridiculousfish/regress/issues/140), a request for a
  backtracking limit, the maintainer said in February 2026 that they were open to one if it cost
  nothing when no limit is set. On 2026-10-11 jlens described its fork there, which does exactly
  that, and offered to adapt it into a pull request. The comment has had no reply.
- **A pull request is not an option.** `regress`'s README has a section "No AI slop". jlens is
  developed with AI assistance, and its fork was written that way. jlens respects the policy and
  does not send code upstream.
- **Upstream moves, in the code the fork changes.** `regress` has 22 commits after `aae35de`,
  none released. Among them: `Regex::find_from_budgeted`, a count of backtracks that now aborts
  the whole match when it runs out (still not a deadline or a cancellation, as ADR 0009
  explains); fixes to class sets with the `v` flag, to backreferences to duplicate named groups,
  and to a hang in certain nested loops; and a reorganization of the backtracker
  (`classicalbacktrack.rs`, +137 −50 lines), the file where the fork's interrupt lives. The
  next rebase would redo the interrupt on a backtracker that has changed, and measure its cost
  again.
- **The crate is published.** `jlens-regress` has been on crates.io since jlens 0.6.0, published
  first by every release, with the version of the workspace.

Choices to make:

1. Whether the fork stays temporary, waiting for upstream, or becomes jlens's own.
2. How upstream's fixes reach it.
3. What changes of its own it takes.
4. Its name.

## Decision

1. **The fork is jlens's own, with no plan to go back to `regress`.** jlens no longer waits for an
   upstream equivalent, and no longer plans to deprecate the crate when one comes. If a
   `regress` release ever offers an equivalent, going back is weighed like any change of
   dependency: on what it offers (a deadline and a cancellation inside the match, no cost when
   unused) and on its cost on `benches/`, not as a promise kept.
2. **Upstream fixes are ported, not rebased.** The fork stays on `regress` 0.12.0 plus the
   upstream commits jlens ports one by one.
   - **What is ported:** fixes to JavaScript semantics, hangs, panics and soundness, with the
     tests upstream adds for them. Performance changes only when `cargo xtask ab` shows a gain
     and the `interrupt_cost` example shows the interrupt still within ADR 0009's targets.
     Upstream's own features (`find_from_budgeted`, say) are ported only when jlens needs them.
   - **How:** each port is a commit that names the upstream commit it ports. The fork's README
     lists the ported commits and the last upstream commit reviewed.
   - **When:** when `regress` publishes a release, and otherwise once per jlens minor version,
     as an item of its plan.
3. **The fork's own changes stay few.** It takes what jlens needs from its regex engine: the
   interrupt, and fixes to bugs that jlens's fuzzing, differential tests or users find. They are
   marked `jlens-regress` in the code, as now. The diff to upstream stays small, because a small
   diff is what keeps ports cheap. Larger work on the engine (speed, a linear-time path) is not
   done in the fork.
4. **The name stays `jlens-regress`, and the library stays `regress`.** It follows the
   `jlens-*` names of the other crates and says what the crate is. A rename would publish a new
   crate and deprecate a published one, for no gain to its users.

Bugs in the fork are reported at jlens. When one also affects `regress`, its maintainer may be
told in an issue, written by a person. The fix in the fork does not wait for upstream.

## Alternatives considered

- **Keep waiting for upstream and keep rebasing.** The fixes arrive either way, by porting, and
  without redoing the interrupt on each reorganization of the backtracker. Waiting has no date,
  and the pull request it waits for cannot be sent.
- **Go back to `regress` with `find_from_budgeted`.** A count of backtracks cannot follow a
  deadline or a cancellation (ADR 0009). It is also unreleased.
- **Another ECMAScript engine.** `corvus-regex-ecma262` was created on 2026-10-20, has a single
  author and no record yet on WebAssembly or interruption. `fancy-regex`, the other Rust engine
  with backreferences and lookaround, follows Perl's semantics rather than ECMAScript's
  (ADR 0002).
- **A shorter name.** Discussed above: the gain is cosmetic, the cost is a second published crate.
- **Copying the fork into `jlens-core`.** Rejected for the reasons ADR 0009 gives: tens of
  thousands of lines in the core crate, and `unsafe` blocks under its `forbid` lint.

## Consequences

- The fork's README no longer says that it will be deprecated, and asks for every problem to be
  reported at jlens.
- The first port reviews the 22 upstream commits after `aae35de`.
- Nothing changes for users: the crate, its name, its API and its version scheme stay as they
  are.
- Replacing the fork by an engine of jlens's own stays possible. It would take its own ADR.
