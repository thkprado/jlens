# jlens-regress

**This is a fork of [regress](https://github.com/ridiculousfish/regress) 0.12.0** (upstream
commit `aae35de`), made for [jlens](https://github.com/thkprado/jlens), a JSONata engine. It adds
one thing: a match that can be interrupted.

`Regex::find_from_interruptible(text, start, interrupt)` searches like
`find_from(text, start).next()`, and calls `interrupt` every `INTERRUPT_INTERVAL` steps of the
backtracker. When `interrupt` returns `true`, the search stops and returns `Err(Interrupted)`.
jlens uses it to stop patterns that backtrack catastrophically, such as `/^(a+)+$/`, within an
evaluation's timeout or when it is cancelled. Without an interrupt, the backtracker compiles to
the upstream code. The design is in jlens's
[ADR 0009](https://github.com/thkprado/jlens/blob/main/docs/src/adr/0009-bounded-regex.md).

```rust
use regress::Regex;
use std::time::{Duration, Instant};

let re = Regex::new(r"^(a+)+$").unwrap();
let text = format!("{}!", "a".repeat(64));
let deadline = Instant::now() + Duration::from_millis(10);
assert!(re.find_from_interruptible(&text, 0, || Instant::now() >= deadline).is_err());
```

The library keeps upstream's name, `regress`. Everything else is regress 0.12.0, under the same
licences (MIT or Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`), apart from the fixes and
the upstream commits ported below.

The fork is jlens's own and is maintained with jlens
([ADR 0020](https://github.com/thkprado/jlens/blob/main/docs/src/adr/0020-own-regress-fork.md)):
it is not rebased onto new regress versions. jlens ports upstream's fixes one commit at a time,
and keeps its own changes to the interrupt and to bugs it finds. Report every problem with this
crate at jlens, not at regress.

## Tested against V8

Besides upstream's tests, a corpus of V8's own answers checks the engine
([`tests/corpus`](https://github.com/thkprado/jlens/tree/main/crates/jlens-regress/tests/corpus)).
`cargo xtask regex-corpus` runs test262's `built-ins/RegExp` and `annexB/built-ins/RegExp`
tests in Node.js, recording every pattern they compile and every `exec` they call, and the
patterns and searches of this crate's own tests; then it asks V8 for its answer to each. The
corpus holds 18,954 cases from test262 commit `d53ebc8` and Node.js 24.19 (V8 13.6): 3,898
patterns that compile or not, and 15,056 searches with their match and groups. 7,300 of them
(39 %) have neither the `u` nor the `v` flag, the mode JavaScript uses by default.

`cargo test` runs every case, through `find_from_interruptible` on a `&str`. 60 cases are not
compared, because a `&str` cannot hold them: a lone surrogate in the pattern or the input, or a
match that starts or ends between the halves of a surrogate pair. 12 cases differ, each listed
with its reason in `differences.txt`: without `u` or `v`, JavaScript matches UTF-16 code units
and this crate matches characters, so `/^.$/` does not match `"𐌀"` in V8; and a V8 bug with
`[^\q{a|b}]` under the flags `v` and `i`.

## Fixed in jlens-regress

- Without the `u` or `v` flag, `\u{...}` is no longer a code point escape: as in JavaScript,
  `\u` is the letter `u` and the braces a quantifier or themselves, so `/\u{2}/` matches `"uu"`
  and `/\u{3f}/` matches `"u{3f}"`. Named groups take `\u{...}` in every mode. Found by the
  corpus above; upstream's tests ran test262's examples of the escape without their `u` flag.

## Ported from upstream

Last upstream commit reviewed: `4f2bfeb` (2026-10-19, unreleased after 0.12.0). Each port is
a jlens commit named `jlens-regress: <upstream subject>`, which names the upstream commit.

| Upstream commit | Subject | Ported? |
|---|---|---|
| `f830464` | Bump taiki-e/install-action from 2.86.3 to 2.86.5 | Skipped: upstream's CI; jlens's CI tests this crate |
| `a879f89` | Bump taiki-e/install-action from 2.86.5 to 2.87.3 | Skipped: upstream's CI |
| `50bead7` | Add an optional backtracking budget | Skipped: `find_from_budgeted`, a feature jlens does not need (the interrupt follows a deadline and a cancellation, ADR 0009) |
| `9b82b9e` | Drop the u64::MAX sentinel check in charge_backtrack | Skipped: budget code |
| `a4c1b83` | Merge two impl blocks | Skipped: budget code |
| `c9c2bad` | Bump taiki-e/install-action from 2.87.3 to 2.87.8 | Skipped: upstream's CI |
| `771e6c8` | Bump taiki-e/install-action from 2.87.8 to 2.87.11 | Skipped: upstream's CI |
| `0328ab6` | Bump taiki-e/install-action from 2.87.11 to 2.87.27 | Skipped: upstream's CI |
| `9247c44` | Keep the first operand before a single & in a class set | Ported |
| `e182704` | Correct negated class sets that contain strings | Ported |
| `b1bda4d` | Clarify and improve test coverage of may_contain_strings | Ported |
| `2d80955` | Fold case before inverting a nested class set | Ported |
| `7cff490` | Match backreferences to duplicate named groups | Ported |
| `6f6ff20` | Restore Match doc comment and derive Debug for BudgetedMatch | Skipped: budget code |
| `c4c3e66` | Abort the entire match after backtracking budget exhaustion | Skipped: budget code |
| `941e2d8` | Fix a hang in certain nested loops | Ported |
| `b00b68b` | Fix some clipplies | Ported |
| `6afa770` | Store backtrack-stack IPs as u32 | Ported: a refactoring of the backtracker, kept to make later ports cheap; no change in speed beyond noise |
| `452f230` | Save capture group state when exiting a group | Ported. A correctness fix that costs speed: `(\w+)\s+\1` over prose is about 15 % slower |
| `e99bee9` | Simplify re-entering a non-greedy loop | Ported: as `6afa770` |
| `1acdb5d` | Backtracking: make loops and groups into boxed slices | Ported: as `6afa770`, without its hunk in the budget code |
| `4f2bfeb` | CI: install cross via cargo instead of taiki-e/install-action | Skipped: upstream's CI |

The upstream README follows.

---

# regress - REGex in Rust with EcmaScript Syntax

oh no why

## Introduction

regress is a backtracking regular expression engine implemented in Rust, which targets JavaScript regular expression syntax. See [the crate documentation](https://docs.rs/regress) for more.

It's fast, Unicode-aware, has few dependencies, and has a big test suite. It makes fewer guarantees than the `regex` crate but it enables more syntactic features, such as backreferences and lookaround assertions.

## Syntax

regress targets the EcmaScript 2018 standard regexp syntax, including support for gnarly cases such as variable-width lookbehind assertions containing capture groups. Note that subsequent standard have mostly left regexp syntax untouched, with a few exceptions such as the 'v' flag, which is supported.

### That darn 'u' flag

You will be sad to learn that JavaScript does not use UTF-8.

Originally JavaScript was designed for UCS-2, with 16-bit characters. Later UCS-2 was supplanted with UTF-16; however this was not automatic for regexps, but instead required opt-in for each regexp, with the 'u' flag. For example, the grinning face emoji U+1F600 is represented in a JavaScript string as a surrogate pair U+D83D and U+DE00. In a regexp without the 'u' flag, these surrogate pairs are matched as distinct characters:

    const s = "😀";
    const re = /./;
    const m = s.match(re);
    console.log(m); // returns \uD83D, high surrogate

This behavior is almost never desired but is required by the ES spec. It's also super-awkward to express in Rust, which uses UTF-8 extensively. See below for how regress handles this.

The 'u' flag doesn't just modify character sets; it _also_ affects other behaviors such as how case-insensitive matching works, and (most bizarrely) the behavior of backreferences like \2. For example, in non-Unicode mode, \2 is a backreference if there are at least two capture groups; otherwise it is an octal escape (!).

regress mostly ignores the 'u' flag for character decoding - that's instead given by the call site (see below). regress attempts to implement the other behaviors faithfully.

### Character sets

tl;dr use UTF-8 (or ASCII) input and the 'u' flag, unless you are implementing a JavaScript engine and care about strict conformance.

To support JavaScript pre-Unicode semantics, regress supports multiple input forms on the `Regex` object. These are:

- **UTF-8**. The default (unsuffixed) form. Input is `&str`. This always decodes whole characters from the input string.
- **ASCII**. Use the `*_ascii` family of functions on `Regex` if you know your input is ASCII. Input is still `&str`.
- **UTF-16**. Use the `*_utf16` family of functions. Input is `&[u16]`. Characters are always decoded as UTF-16.
- **UCS-2**. OG JavaScript. Use `*_ucs2` functions. Input is `&[u16]`. Surrogate pairs are split freely. Only use if you want to implement strict JS semantics.

Both the UTF-16 and UCS-2 forms require the Rust feature 'utf16' to be enabled. It is off by default.

### Fun Tools

The `regress-tool` binary can be used for some fun.

You can see how things get compiled with the `dump-phases` cli flag:

    > cargo run 'x{3,4}' 'i' --dump-phases

You can run a little benchmark too, for example:

    > cargo run --release -- 'abcd' --flags 'i' --bench ~/3200.txt


## No AI slop

As written.
