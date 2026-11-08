# Roadmap

What comes next for jlens, and what is not planned. Finished plans, one paragraph per release,
are in the [roadmap history](roadmap-history.md), and every change in the
[changelog](https://github.com/thkprado/jlens/blob/main/CHANGELOG.md). A plan's table lists its
items, one row each: what it delivers, when it is done, and its state.

## Priorities

Three principles order the work after 0.2:

1. **The core is a library for custom integrations.** Applications embed `jlens-core` in their
   own pipelines. It must stay small and decoupled, let the application set every limit, never
   panic, and change its API only on purpose.
2. **The CLI is a tool for automation and AI agents.** It never prompts, it gives the same
   output for the same input, and it has one documented machine-readable contract: what goes to
   standard output, what goes to standard error, and what each exit code means.
3. **Editors come after.** The language server is done; the VS Code extension is 0.15.

For ad-hoc JSON queries, `jq` remains the usual tool. jlens is for JSONata that will run
somewhere else, such as a rules engine or a jsonata-js service. It checks those expressions,
tests them and explains their results before they ship.

## 0.31: a small map for objects

Every object jlens builds or converts is an `IndexMap`: a vector of entries and a hash table over
it, two allocations and a hashed key for each lookup, even for an object of three fields. Tuples
are such objects, one per item (4 of the 9 allocations per item of `tuples`); the object `$match`
returns costs 4 allocations a match (0.52 µs against jsonata-js's 0.23 µs on the Windows heap);
and `$distinct` compares objects by looking each key up in the other. 0.31 gives `jlens-core` a
map of its own, a vector of fields searched in order, with an index only past a number of fields,
and measures each step against 0.30.0. Nothing public changes.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | What objects hold, before any change, below | The field counts of the objects the benchmark workload builds and converts, and of tuples; allocations and times of `tuples`, `object constructor`, `group-by`, `transform`, `distinct items` and a `$match` case, at 0.30.0; written to `docs/notes/perf-0.31.md` | done: no object of the workload has more than 6 fields (the suite's datasets: 11); a path looks a key up in each item; of the 7 allocations of a constructed object, 1 is the hash table and 4 its literal keys and string (#5) |
| 2 | The map, below; after #1 | A module of `jlens-core` with the operations the crate uses, insertion order kept, the threshold of the index set by #1's counts and a measured crossover of a search in order against foldhash; unit tests and a property test against `IndexMap` as the model; an ADR | done: `jlens_core::map::Map`, an index past 8 fields ([ADR 0022](../adr/0022-small-map.md)); a search in order is faster up to 8 fields; no fields kept in the object |
| 3 | `Map` is the new map, crate-wide; after #2 | Every object, tuple and `$match` result uses it, and the hasher that lazy values share goes; `cargo xtask ab` against 0.30.0 in each mode, Python and Java; #1's allocations counted again; each case within the floor or faster, or its cause recorded | done: one allocation fewer for each object, tuple and match; `tuples` 10 to 14% faster, `distinct items` 17 to 20% with a converted input, `object constructor` 5 to 7%; nothing slower past the floor |
| 4 | Objects compared without hashing; after #3 | `deep_eq` and `$distinct` compare two small objects field by field, without a lookup per key; `distinct items` and a case of objects with few fields measured | done: fields compared in pairs until two keys differ; at most 4% faster up to 8 fields, where a lookup hashes nothing; `$distinct` over objects of 12 fields 44% faster against a `Document` |
| 5 | Literal keys and strings made once, below; after #4 | An object constructor's literal keys and literal string values are made once, not for each object; `object constructor` counted and measured against #4 and 0.30.0 | done: kept by node id for each evaluation; 2 allocations for each object instead of 6; `object constructor` 14 to 17% faster than #4, 21% than 0.30.0 |
| 6 | Upstream before the release: `regress` after `95c0404`, stacker#155 and #156, jsonata-js's commits after `c5be116` and its version on npm | Each `regress` commit worth porting ported one at a time, or none; each jsonata-js fix checked against jlens; the watch list updated | done: nothing new since 0.30: `regress` at `95c0404`, #156 open and unmerged, jsonata-js 2.2.2 with no commit after `c5be116`; jlens gives 2.2.2's results on the cases of the open pull requests #841 and #842 |
| 7 | The tables measured again, unattended, after the dependencies are brought up to date: jlens and the control columns; other engines only where their version changed; the same tables measured on an Apple M1 as well, for macOS on ARM | `ab` against 0.30.0 in every mode, Python and Java; the performance page's tables and version header updated, with the M1's results beside them; each case within the floor or its cause recorded | done: nothing to update; on both machines the gains of #3 to #5 hold in every package and no case is slower past the floor but `filter + aggregate` with Python values on the M1 (3 to 4%, cause not found); the M1 is 1.4 to 2.5 times faster, and quieter, with the engines in the same order |
| 8 | Release 0.31.0 | The release checklist; the channels checked | planned |

**#1, what objects hold.** The map's shape depends on how many fields objects have: a search in
order is faster than hashing for a few fields and slower for many. #1 counted them where jlens
spends its time and recorded the numbers that #3 compares against, in
[`docs/notes/perf-0.31.md`](https://github.com/thkprado/jlens/blob/main/docs/notes/perf-0.31.md).
Lookups matter as much as construction: `filter + aggregate` builds no object and looks a key up
in 367 objects of 6 fields.

**#2, the map.** [ADR 0022](../adr/0022-small-map.md): fields in a vector, in insertion order,
with their shared `Rc<str>` keys; a lookup compares lengths, then text. Measured on this machine,
a search in order finds a key present faster than foldhash up to about 12 fields, one absent up
to about 7, and builds the map faster up to about 24, so a map of more than 8 fields keeps an
index of its keys (a `hashbrown` table of positions). Fields kept in the `Object` itself save an
allocation only on the system heap, cost more with `mimalloc` and for objects that hold more, and
triple a small object's size: they were left out. The module has the operations of `IndexMap` the
crate uses, and a property test compares the two maps; a trial swap of the type showed that only
the hasher's constructors are missing, which #3 removes. The measurements are in the notes.

**#3, the map in place.** Every object, tuple and `$match` result now has the map of #2, and the
hasher the lazy values shared is gone: one allocation fewer for each object built. A tuple step
now copies the tuple it extends with room for the fields it adds, which saved one more allocation
for each tuple. Against 0.30.0, `tuples` is 10 to 14% faster, `distinct items` 17 to 20% when the
input is converted, and Python and Java follow. `filter + aggregate` and `group-by` read 1.5 to
2.4% slower from a lazily converted input, whose lookups do not reach the map; an aligned build
puts them at the floor. The measurements are in the notes.

**#4, objects compared in pairs.** Two objects of one shape hold their keys in one order, so
`deep_eq` compares their fields in pairs and looks a key up only from the first pair whose keys
differ; the values compared, and their order, are those of the lookups. Up to 8 fields a lookup
hashed nothing and skipped most keys on their length: `distinct items` is 1 to 4% faster, and
objects of two fields compare as they did. Past 8 fields each lookup hashed its key: `$distinct`
over 200 objects of 12 fields, each twice, is 44% faster against a `Document` and 15% from JSON.
The measurements are in the notes.

**#5, literal keys and strings.** `items.{'code': code, 'kind': 'item'}` allocated each key with
`Rc::from` and the string `'item'` again for each item: 4 of the 6 allocations of each object of
`object constructor` at #4. An `Expression` is shared between threads, so its tree cannot hold the
`Rc<str>` a `Value` takes. The evaluation now keeps the string of each literal it evaluates, by
node id, and clones it; since `$eval` evaluates trees of its own, numbered from 0 as well, each
use compares the text kept with the literal's. An object of `object constructor` takes 2
allocations instead of 6: the constructor is 14 to 17% faster than at #4 on the system heap, 21%
than 0.30.0, and 2% with `mimalloc`, where an allocation costs little. The measurements are in
the notes.

## After 0.31: where releases are measured

From 0.32 on, the performance page's tables are measured on the Apple M1 alone, which 0.31 #7
found quieter (two comparisons within 1.3 points, against up to 3 on the Ryzen) and steady through
three hours. x86_64 keeps a check without tables: while the Ryzen is available, each release runs
`cargo xtask ab` against the previous tag in every mode and `python.py ab` under Linux on it,
about 30 minutes, and investigates only what passes the floor. A release is compared with an
older one by measuring the older tag on the M1, not by the Ryzen's tables.

| # | Item | Done when | Status |
|---|---|---|---|
| 1 | The page on the M1 | The next release's measure item fills every table from the M1 (jlens and the controls; the other engines from 0.31's M1 runs until their versions change), states the M1 as the machine and the Ryzen as an `ab` check only, and folds the section "On an Apple M1" into the tables | planned |
| 2 | `filter + aggregate` with Python values on the M1 | Its 3 to 4% against 0.30.0, found by 0.31 #7, explained or removed | planned |

## In exploration: a regular expression engine of jlens's own

`jlens-regress` is a backtracker. A catastrophic pattern runs within the evaluation's limits
([ADR 0009](../adr/0009-bounded-regex.md)), but it still runs to them. Its positions are code
points, which jlens converts to UTF-16. On the engine's benchmark, V8 is 2 to 11 times faster.

An engine of jlens's own could find a match in linear time wherever the pattern allows it: an
automaton finds the match, and the backtracker is left only to extract its groups, or to run
the patterns that need it (backreferences, lookarounds). It would also have the interrupt and
UTF-16 positions built in, with no `unsafe`.

This is research, not a release. The work lives apart from `jlens-core`, and the corpus, the
differential test and the benchmark of 0.22 measure it. It ends in a decision: an ADR that adopts
the engine in place of `jlens-regress`, or drops it. Adopted, the swap becomes a release of its
own.

| # | Stage | Done when | Status |
|---|---|---|---|
| 1 | The linear-time path on the patterns it can take, with the interrupt and UTF-16 positions | Its share of the corpus's patterns and their answers, against V8's | |
| 2 | Every pattern: the backtracker for the rest | The corpus (19,108 cases) and `cargo xtask regex-differential` with 20,000 cases clean, or each difference recorded with its reason | |
| 3 | Speed and limits, against `jlens-regress` and V8 | The engine's benchmark and `interrupt_cost`; catastrophic patterns timed | |
| 4 | The decision | An ADR: the engine replaces `jlens-regress`, or is dropped, with the measurements | |

## Deferred: an API to keep, toward 1.0

After twenty releases, nothing says which parts of jlens are meant to last. `cargo
semver-checks` runs in CI, but before 1.0 it only asks for a new minor version, and
`jlens-core` exposes workspace plumbing (`stack`, `debug`, the AST's internals) next to its API.
This work decides what 1.0 will promise, breaks the API once to get there, and writes the
promise down. It takes the number of the next release when it starts. Applications built on jlens are separate projects: they use `jlens-core` as any
application does, and what they lack comes back here as general features.

**Deferred** until applications built on jlens have used it: what to keep stable is decided
from what they needed, missed or worked around, not guessed beforehand.

| # | Deliverable | Definition of done | Status |
|---|---|---|---|
| 1 | ADR 0015, what 1.0 promises: what is stable, hidden or `#[non_exhaustive]`, below | The ADR accepted, with the list of changes that #2 to #4 make | |
| 2 | `jlens-core` changes as ADR 0015 says, in one breaking batch | `cargo semver-checks` reports only the intended breaks; the changelog has a migration section; the crate guide's snippets updated | |
| 3 | `jlens-schema` and `jlens-lsp` changes as ADR 0015 says | As #2 | |
| 4 | The contracts outside Rust, as ADR 0015 says: the Wasm API, the Python package, the Java package, the JSON of audits and `jlens.json`; a test pins each, so that a change shows in review | Tests that fail on an unintended change of each contract | |
| 5 | A stability page in the book, from ADR 0015, and the rules for changing a stable item in `CONTRIBUTING.md` | `doc-check` green; the page linked from the introduction | |
| 6 | A source that fails with a JSONata code, below; a change to public traits, so it goes with #2's batch | The same input nested too deep fails with the same code from every input and package; a cycle stays `U1004` | |
| 7 | The release that breaks once. 1.0.0 follows when ADR 0015's criterion is met | The release checklist, with the migration notes in the release notes | |

**#1, what ADR 0015 decides.** Exploration: list the public items of `jlens-core`, `jlens-schema`
and `jlens-lsp` (rustdoc JSON or `cargo public-api`) and the contracts outside Rust: the Wasm API
(`types.d.ts`), the Python package, the CLI (ADR 0008), the JSON of audits and explanations,
`jlens.json`, the error codes. For each item: stable, hidden (`#[doc(hidden)]`, `pub(crate)`) or
`#[non_exhaustive]` where it may grow. What semver covers and what it does not (messages, `Display`,
`Debug`, performance, jsonata-js quirks), the MSRV policy, deprecations.

**#6, a source that fails with a JSONata code.** Found by 0.26 #9: `Source` and `Sources` fail with
a message, which the core always turns into `U1004`, so input nested more than 1,000 levels deep
fails with `U1001` as JSON text and with `U1004` as Java objects or a C API host's values, read as
the expression goes; the C API also gives one message for a cycle and for deep input, as it cannot
tell them apart. The error type of the reading methods carries a code (`U1004` by default, `U1001`
for nesting), and each source says which limit it met; in the C API, a node inside itself that the
host does not report (the Java `Reader` does) is seen as deep input. The WebAssembly package has the
same gap for its exceptions: it keeps the first one a level throws, which since 0.27 #6, where a
level may be read in case it is needed
([ADR 0010](../adr/0010-lazy-input.md#amendment-a-level-read-in-case-it-is-needed-027)), may not
be the level that stopped the evaluation. A change to public traits, so it goes with #2's batch. Done when:
the same input nested too deep fails with the same code from JSON text, a `Document`, Python,
JavaScript and Java values and a C API host; a cycle that the host or package detects stays `U1004`;
the exception rethrown in WebAssembly is that of the level that failed; tests in each package.

## Not planned

- **Asynchronous functions.** A host function that returns a promise fails with `U1002`, and
  jlens does not plan to change that. Supporting it needs an evaluator that can suspend, which
  would slow down every evaluation, and only WebAssembly would gain from it: a Rust host
  function can block. An application can fetch what its functions need before evaluating.
  Should real use call for it, the cheaper way is an `evaluateAsync` that stops at the first
  promise, awaits the pending ones and evaluates again with their results.
