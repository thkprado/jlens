# Performance notes: 0.32

How the measurements of roadmap 0.32 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) leaves out. The item numbers are those of
the plan in [the roadmap](../src/architecture/roadmap.md).

## Where an evaluation's time goes at 0.31.0 (before the plan)

The engines that compile an expression to their host's code are faster on most cases that do much
work per item. On the Apple M1 (0.31.0's tables), against an input converted once,
jsonata-jvm-compiler takes a third to a fifth of jlens's time on most of them, and an eighth on
`recursion`; jsonata2js, against jlens-wasm, a third to a fifth, and a tenth on `recursion`. The
profile below asks how much of an interpreter's time could go without compiling.

### Shares of the samples

At d55a99a (0.31.0), under Linux in WSL on the Ryzen 3700X, against a `Document`, about 50,000
samples a case. A share is inclusive when it counts every sample whose call chain holds the
function, self when only the innermost frame (inlined frames included) is counted.

| | `recursion` (508 µs) | shipping rule (81 µs) | index in a variable (0.5 µs) |
|---|---:|---:|---:|
| Variable lookup (`Frame::lookup`, `Vars::get`, `param`), inclusive | 9.6% | 0.2% | 1.0% |
| Bindings (`Vars::set`, `Name::new`), inclusive | 0% | 0.4% | 4.1% |
| `Rc<Frame>` made and dropped, inclusive | 6.4% | 0.1% | 4.7% |
| Guards of each node (`guardrails`, `stacker`, `psm`), self | 10.0% | 5.4% | 1.7% |
| The `?` on each node's `Result`, self | 11.0% | 12.4% | 4.8% |
| `eval_inner`, self | 30.6% | 20.6% | 12.1% |
| libc (allocation, copies), self | 4.7% | 10.3% | 22.2% |

- Fixed slots for variables would save at most a tenth of `recursion` and next to nothing of the
  shipping rule.
- What every node costs (the dispatch in `eval_inner`, the check of its `Result`, the guards)
  makes more than half of `recursion`, though `eval_inner`'s share also holds real work: the
  comparisons, the arithmetic, the values built. Of the guards, reading `stacker`'s thread-local
  stack limit alone is 2.2% of `recursion`.
- `Error` is a `Box` and `Value` 24 bytes, so the `?` is the test of each result, not a large
  copy.
- Removing half of the cost per node would make `recursion` about 1.3 times faster; the rest of
  the distance to the compilers is their host JIT inlining the generated code into its runtime.

### How it was measured

- A program outside the workspace with a path dependency on `jlens-core`: it compiles the
  expression, converts the input once to a `Document` and runs `evaluate_document_to(.., |_|
  Ok(()))` in batches of 64 for a given time. Release profile with `lto`, one codegen unit and
  `debug = 2`.
- WSL's `perf` has no libelf, so it cannot read symbols. The program is built non-PIE, so that
  its sampled addresses are its own: `RUSTFLAGS="-C force-frame-pointers=yes -C
  relocation-model=static -C link-arg=-no-pie"` with `--target x86_64-unknown-linux-gnu`, as
  without a target the flags reach the proc macros and break their link.
- `perf record -e cpu-clock:u -F 4999 --call-graph fp`, then `perf script -F ip,dso`. A script
  symbolizes each address of the chains with `addr2line -a -f -i -C` (a return address minus one,
  to land in its call), which gives the inlined frames too.
- Frame pointers, a non-PIE binary and the system allocator change the absolute times; the shares
  are what the profile is for.

## #3: cheaper guards for each node

Each node went through `stacker::maybe_grow`, which reads its thread-local stack limit (through
`get_or_init`) and calls `psm` for the stack pointer, and then through two tests of options, the
input's source and the cancellation token. Now:

- The evaluator keeps the stack's floor, the address below which a node must grow the stack:
  `crate::stack::floor()`, from `stacker::remaining_stack()` when the evaluation starts. A node
  compares the address of a local with it. Below it, a cold path runs the node through
  `stack::grow` and, while it runs, keeps the floor of the segment it runs on; it puts the
  caller's back when the node returns. An unknown limit gives a floor of `usize::MAX`, so every node
  takes the cold path, as every node went through `stacker` before. On WebAssembly the floor is
  the red zone, and below it the node fails with `D1011`, as before. Functions called back take
  the same test.
- One flag, set when the evaluation has a source or a cancellation token, stands for both tests.
- The steps, `D1011`, `U1003`, `U1004` and `U1006` come where they came: the checks run in the
  same order. A test recurses 20,000 levels on a 256 KiB thread, twice in an evaluation and
  through `$map`; with the cold path removed, it overflows the stack.

`cargo xtask ab` against f884cfc (the Ryzen) and 94dda35 (the M1), both 0.31.0's `jlens-core`,
10 rounds, medians:

| | Ryzen `document` | `evaluate` | `source` | M1 `document` | `evaluate` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| `recursion` | -10.7% | -11.3% | -11.6% | -16.3% | -17.0% | -15.1% |
| shipping rule | -3.1% | -4.4% | -2.0% | -7.0% | -6.5% | -2.2% |
| `regex` | -2.2% | -1.2% | -2.1% | -11.5% | -10.4% | -8.5% |
| `filter + aggregate` | -5.6% | -4.2% | -4.3% | -9.7% | -8.4% | -2.9% |
| `sort + map` | -5.4% | -5.1% | -4.0% | -4.3% | -4.5% | -4.8% |
| `number formatting` | -1.6% | +0.3% | -2.6% | +2.2% | +1.1% | +0.8% |

- Every other case is between -5.4% and +0.9%, except `transform` in the Ryzen's `evaluate` mode,
  +6.5% in the full run (its fastest round +1.3%) and -0.4% alone in 20 rounds: the case whose
  floor is widest.
- `number formatting` on the M1, alone in 20 rounds: +0.9% (fastest +1.2%), at the M1's floor; it
  evaluates few nodes per item, and it is faster on the Ryzen, so it is taken as layout.
- The two parts were not measured apart.

## #4: one frame fewer for each lambda call

Each call of a lambda made its frame with `Rc::new` and freed it when the call returned, and so
did each block that binds a variable. Now the evaluator keeps the frames whose scope ended with
nothing else holding them (`Rc::get_mut` succeeds: no closure, thunk or entry of the list of
cyclic frames refers to them), and the next call or block takes one of them before allocating.
Their bindings and parent are dropped when the scope ends, as before. The kept frames make a
list through their own `parent`, so the evaluator gains one field and the list never allocates;
it holds at most the frames that were once alive together, and the evaluator drops it one frame
at a time, since a list dropped through the parents would take the stack as deep as the
recursion went (the test of 20,000 levels on a 256 KiB thread overflows without that loop).

Two ways to keep them were measured first and left:

- In a `Vec`: an evaluation with one block (`index in a variable`) paid the list's allocation
  for a frame it never reused, +2 to 3.5% on both machines.
- In an array of 16 in the evaluator: on the M1, `path` +6 to 7% and `regex` +4 to 5% in every
  mode, cases that make no frame, from the 136 bytes the evaluator grew.

`cargo xtask ab` against 18e3f95 (the Ryzen) and the same tree on the M1, which has #3's
evaluator, 10 rounds, medians:

| | Ryzen `document` | `evaluate` | `source` | M1 `document` | `evaluate` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| `recursion` | -12.8% | -12.5% | -13.0% | -7.2% | -7.1% | -7.2% |
| shipping rule | -0.5% | -4.6% | -1.2% | +0.3% | +0.2% | -0.6% |
| `index in a variable` | -0.5% | +0.4% | +0.7% | +2.3% | +1.8% | +1.2% |
| `index into 10,000 items` | -2.0% | -0.7% | +1.3% | +0.9% | +1.6% | +1.1% |

- Every other case is between -4.0% and +1.8%, except `path` in the M1's `source` mode, +6.5% on
  the median and -1.3% on the fastest round.
- Alone in 20 rounds on the M1, `index in a variable` is +0.0% (`document`) and +0.4%
  (`evaluate`), and `path` -1.0% and +2.8% (fastest -0.8%); `index into 10,000 items` is +1.5%
  (`document`, about 5 ns) and +0.5%. That case makes no frame: the change reaches it only by the
  evaluator's one more field and the test of an empty list when it is dropped, so it is taken as
  layout.
- #3 and #4 together make `recursion` about 22% faster on the Ryzen and 22% on the M1 than
  0.31.0, so the series goes on to #5.

## #5: aggregates over a field

`$sum(items.price)` built the array of the prices, a `Vec` and an `Rc`, then gave it to the
function, whose signature checked each item once more; `$count(items[code = "x"])` built the
array of the items that passed. Now a call whose function is a variable naming `$sum`,
`$average`, `$max`, `$min` or `$count`, with one argument that is a path of field names
(the first may be a variable), checks that the variable holds the built-in function and reads
the items itself: the field of each item, or for `$count`, a filter that compares one field of
each item with a string literal. It builds nothing.

- The general path runs, from the start and with nothing spent, wherever it would do anything
  else: a value that is not a number (for `$count`, an array), an item that is an array, an
  array made by a constructor after the first step, more values than `max_array_length`, a
  depth within four levels of `max_depth`, a tracer, or a variable `$sum` bound to another
  function. So the errors (`T0412`, `D1001`, `U1001`, `D1011`) come from the general path.
- The steps are those the general path counts (ADR 0018): for `$sum(items.price)`, the
  function's variable, the path, the steps before the last and one for each item; for the
  count, four for each item (the comparison, the literal, the field's path and its step). They
  are spent once the items are read, and the cancellation is checked then. `steps.json` has
  the cases, which the audit counts through the general path.
- The test was first written in `eval_inner`, before the call: on the Ryzen, `recursion` was 1.6
  to 2.7% slower in every mode, alone in 20 rounds too, for a test that costs a few branches a
  call (about 2,000 calls a run, so far below 1 ns each). Moved to the start of `call`, it was
  +0.5% and +0.6%, at the floor: the size of `eval_inner`, not the work.

`cargo xtask ab` against 53744f1, 10 rounds, medians, with a case added for the run only
(`count of a literal`: `$count(items[type = 'electronic'])` over the 200 orders' items):

| | Ryzen `document` | `evaluate` | `source` | M1 `document` | `evaluate` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| `names over items` | -49.7% | -7.1% | -11.1% | -38.6% | -17.4% | -16.7% |
| `count of a literal` | -83.7% | -42.2% | -33.4% | -75.3% | -40.0% | -29.2% |
| shipping rule | -4.0% | +0.1% | -1.1% | -1.2% | -2.7% | -2.7% |

- Every other case is between -2.6% and +1.8%, except three that rerun alone at the floor:
  `object constructor` on the Ryzen, +5.1% in `document` (alone in 20 rounds +1.1%, and +1.8%
  in `source`), the case whose floor is widest and which calls no function; on the M1,
  `filter + aggregate` in `source`, +9.9% (alone -0.7%), and `recursion` in `document`, +2.4%
  on the median and -1.0% on the fastest round (alone in `source` -0.9%).
- `names over items` gains less with `evaluate` and `source`, where reading the input takes most
  of the case. The shipping rule calls `$sum` and `$count` three times.
- No case of the workload counts a filtered list, so `count of a literal` stays out of
  `cases.json`: a new case would need a column in every engine's table.

## #6: the test, the lookup after #3 to #5

#6 was to go ahead only if the lookup still took more than a tenth of `recursion`. The profile of
0.31.0 was repeated at 1f17557 (#5), with the same program and method, about 50,000 samples:

| `recursion`, shares of the samples | 0.31.0 | 1f17557 |
|---|---:|---:|
| Variable lookup (`Frame::lookup`, `Vars::get`, `param`), inclusive | 9.6% | 10.9% |
| of which `param`, the search of the lambda's parameters by name, inclusive | | 5.2% |
| Guards of each node (`guardrails`, `stack::above`, `stack::position`), self | 10.0% | 8.4% |
| The `?` on each node's `Result`, self | 11.0% | 11.0% |
| libc, self | 4.7% | 2.8% |

- #3 and #4 took time away from the rest of each call, so the lookup's share grew past the
  tenth, and #6 goes ahead. The program's own times (frame pointers, the system allocator) are
  not compared; `cargo xtask ab` measures each item.
- Half of it is `param`: a read of `$n` in the body of `function($n)` finds its frame at once,
  then compares `n` with each parameter's name (a `memcmp` call for one byte), and a read of `$f`
  compares `f` with `n` before going up to the block that binds it. Each call reads `$n` three
  times and `$f` twice.
- A slot for every variable would have to follow every way a frame is made (blocks, calls,
  tuples, `$eval`, bindings in a conditional) to be right. #6 takes the half the profile shows:
  a variable read in a lambda's body, with no scope between, knows its parameter's place from
  the compiled expression, and the frame confirms it at run time before it is used; anything
  else takes the general lookup.

## #6: a lambda's parameters read by their place

When an expression is compiled, a last pass gives each variable read in a lambda's body, outside
any lambda or block with a frame of its own, the address of that lambda's definition and the
index of the parameter of its name, or a mark that none has it (`ast::Param`, a private field
of `Attrs`). A read checks that the innermost frame is a call of that definition, with all its
parameters (not a partial application, which names those left to supply) and nothing bound
by the body beside them; then it takes the argument at that index, or for a name no parameter
has, looks it up from the frame above. Every other frame looks the name up as before.

- The address is that of the definition holding the read, which lives as long as the read does,
  so no other definition can have it while the read runs. A tree whose definitions are copied
  keeps the old address, which no call of the copy has: the reads take the general lookup.
- `$eval` compiles its own tree, whose reads are placed against its own lambdas; a tail call's
  thunk is a lambda without parameters, so its reads go to the frame above.
- `Attrs` was made with a struct expression before: a breaking change, so the version is 0.32.0
  from here on.

`cargo xtask ab` against fd2534c, 10 rounds, medians:

| | Ryzen `document` | `evaluate` | `source` | M1 `document` | `evaluate` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| `recursion` | -4.9% | -5.8% | -5.7% | -9.1% | -8.9% | -8.3% |
| shipping rule | -0.0% | -2.6% | +0.0% | +0.0% | +0.4% | -0.5% |

- Every other case is between -3.1% and +1.6%, except `transform` in the Ryzen's `evaluate`
  mode, +4.3% on the median and -0.6% on the fastest round, the case whose floor is widest, and
  on the M1 in `document`, `distinct items` +2.0% (alone in 20 rounds +1.5%, fastest +1.4%) and
  `index in a variable` +1.0% (alone +2.0%, fastest +1.0%). Neither reads a variable in a
  lambda: on their path the change adds the test of one field, far below their 5 to 160 ns, so
  they are taken as layout, as in #4.
- #3 to #6 together make `recursion` about 27% faster on the Ryzen and 29% on the M1 than
  0.31.0.

## #10: a value's tag a word wide

0.27 found `path` stalling about seven times an item on a store the processor could not forward
to a load. The Ryzen counts these loads (`ls_bad_status2.stli_other`, raw event `r0224` under
`perf` in WSL); sampled on that event, the probe of #6 at 160719e shows the cause:

- `Value` had a byte for its tag. Where a value is moved after it was built in parts, as in
  `Ok(value)`, `Some(value)` or a value `clone()` returned, LLVM copies the tag byte and the
  bytes after it apart: the byte, then 16 bytes at offset 1, or 8, 4, 2 and 1 bytes from offset
  1. The next copy reads 16 bytes from offset 0, across both stores, and waits for them to reach
  the cache.
- It is not the path's alone: every node's `Result<Value>` is written that way on return, which
  is part of the 11% the profile of #6 gives to the `?` on each node's result. `recursion` met
  about 107,000 such loads an evaluation, `filter + aggregate` about 7,400.

Two changes were tried:

- **The path step alone.** The step that looks a field up in each item built an `Option<Value>`,
  copied in parts. Without it, `items.quantity` met 510 such loads an evaluation instead of
  1,930; on Windows, 3% faster in `document` and `source`, nothing in `evaluate`, and
  `names over items` 3% slower in `evaluate`. With the next change, it added 2 to 4% to
  `items.quantity` and took 2% from `recursion` in every mode, so it was dropped.
- **A tag a word wide** (`#[repr(u64)]` on `Value`). Every part of a value now starts on a word,
  so that a copy in parts and a copy whole meet on the same edges. `recursion` meets 7,600 such
  loads an evaluation instead of 107,000, `filter + aggregate` 1,300 instead of 7,400, and the
  probe in WSL times them 14% and 22% faster. The value stays three words, and `Option<Value>`
  and `Result<Value>` too, as the tag keeps its niche; a test holds the three sizes.

`cargo xtask ab` against 160719e, 10 rounds, medians:

| | Ryzen `document` | `evaluate` | `source` | M1 `document` | `evaluate` | `source` |
|---|---:|---:|---:|---:|---:|---:|
| `recursion` | -8.8% | -7.6% | -7.6% | -4.8% | -5.4% | -5.1% |
| shipping rule | -16.3% | -7.5% | -7.1% | -13.1% | -10.6% | -5.7% |
| `filter + aggregate` | -16.3% | -6.1% | -7.6% | -9.7% | -7.2% | -5.5% |
| `group-by` | -9.0% | -5.5% | -8.5% | -9.6% | -8.7% | -6.1% |
| object constructor | -5.1% | -4.5% | -2.3% | -9.8% | -10.2% | -11.3% |
| index in a variable | +0.0% | -1.3% | -1.3% | -13.0% | -10.7% | -9.4% |
| `path` | -1.3% | -1.3% | +0.4% | +0.0% | +0.4% | -4.5% |
| `names over items` | -0.2% | +1.3% | -2.7% | +1.8% | +0.2% | +0.8% |

- On the Ryzen, every other case is between -9.0% (`strings` in `document`) and +1.1%; none is
  slower past the floor. On the M1, the indexes into 10,000 items are 8 to 10% faster too.
- One case is slower: `names over items` on the M1 in `document`, +1.8%, +1.9% alone in 20
  rounds and +2.0% with functions and blocks aligned on both sides, so not placement; 1.68 to
  1.71 µs for 200 items. Its loop (#5's aggregate) only looks a field up and matches its
  variant; no cause was found, and it is left against the gains elsewhere.
- The robustness properties pass with `PROPTEST_CASES=100000`; the change is to the layout only.
- 0.27's stall in `path` is this one: `path` itself, a single object, is within 1.3% on the
  Ryzen and 0 to 4.5% faster on the M1.

## #7: every package

Each package at 2a1ece0 (#3 to #6, #10, #14, #15) against 0.31.0, on the M1: `python.py ab`
twice, `cargo xtask java-compare ab` once, and for the WebAssembly package `compare.mjs --pkg
--engines jsonata-js`, four processes a build, alternated, each on a new copy of its package.

**Python**, the two comparisons, 5 rounds each:

| | `evaluate` | `document` |
|---|---:|---:|
| `recursion` | -26.4%, -28.2% | -27.5%, -28.1% |
| shipping rule | -10.5%, -11.1% | -20.7%, -20.9% |
| `names over items` | -8.7%, -10.2% | -35.1%, -34.6% |
| `filter + aggregate` | -9.5%, -9.8% | -17.7%, -17.5% |
| object constructor | -11.3%, -12.4% | -15.1%, -15.1% |
| `group-by` | -6.1%, -7.8% | -11.6%, -10.6% |
| `tuples` | -6.9%, -4.9% | -8.9%, -8.5% |

Every other case is 3 to 10% faster or within 1.4%: `regex over a long text`, `distinct items`,
`path` and the index into 10,000 items against a `Document`, which evaluate few nodes. None is
slower.

**Java**, 5 rounds; "converted" is an input converted once, as a `Document`:

| | converted | JSON text | objects, converted | objects, JSON text |
|---|---:|---:|---:|---:|
| `recursion` | -30.4% | -26.8% | -27.7% | -23.2% |
| shipping rule | -21.8% | -12.1% | -7.4% | -5.4% |
| `names over items` | -35.6% | -3.4% | -6.7% | -1.4% |
| `filter + aggregate` | -19.7% | -6.7% | -7.0% | -3.0% |
| `group-by` | -12.5% | -5.7% | -7.8% | -2.7% |
| `$sum(items.price)`, 1,000 items | -41.1% | -2.8% | -5.7% | -1.2% |

- Reading JSON text or Java objects takes most of an evaluation's time where little is
  evaluated, so the gains shrink there; the threads evaluate 1 to 4% more a second.
- Slower: `$count(items)` on a converted document, +7.6% (0.306 against 0.330 µs), marked. #5's
  aggregate did every check of its shape, the lookup of `$count` and a copy of the input before
  finding that the input, an object, is not the array a field of each item needs; the general
  path then looked `$count` up again. It now tells that from the input before the lookup.
  Natively on the M1, with the case added for the run, `$count(items)` against a `Document` took
  234 ns at 0.31.0, 251 ns at 2a1ece0 and 234 ns with the change. Against 2a1ece0, 10 rounds in
  each mode, it is 3.0 to 8.2% faster, and no other case is slower by more than 1.0%.
- `$` converted, +6.7% and +8.9%, and the 100,000 items read from JSON text as Java objects,
  +4.6 to 8.6%, are unmarked, with spreads of 10 to 25%: the first converts the whole document
  back to Java, the second is read whole, and neither changed.

**WebAssembly**, the medians of four processes:

| | `evaluate` | `evaluateJson` | `evaluateDocument` | jsonata-js, the control |
|---|---:|---:|---:|---:|
| `names over items` | -5.2% | -1.8% | -26.9% | +1.9% |
| `recursion` | -2.1% | -2.7% | -4.8% | +2.9% |
| `group-by` | -1.8% | -2.6% | -2.6% | +4.8% |
| `distinct items` | -0.4% | -1.4% | -2.6% | +1.6% |

- Every other number is within 3%, as jsonata-js's own column moves by up to 4.8% from one
  process to the next; `path` in `evaluateDocument`, +7.7%, is 0.7 µs printed to a tenth.
- Only #5's aggregate reaches the package as it reaches the others. On `wasm32`, 0.31.0 already
  guarded each node with one comparison of an address, without `stacker`, so #3 had nothing to
  remove there; the rest of the series gives `recursion` 5%, where #4, #6 and #10 give about
  19% natively on the M1. Why was not looked into: the package is not a priority of this plan.

## #16: why WebAssembly gained little

#7 found the series giving WebAssembly's `recursion` about 5%, against about 19% natively.

### The profile

`recursion` against a `Document` (`evaluateDocument`) in Node.js 24.19 on the Ryzen, `node
--cpu-prof --cpu-prof-interval 100`, about 20,000 samples. The module is the package's
(`wasm-release`, `wasm-opt -Os`) with `wasm-opt -g`, which keeps the functions' names; V8 gives
no line within a function, so the shares are of functions after inlining, self:

| `recursion`, shares of the samples | f418ca9 | with #16 |
|---|---:|---:|
| `eval_inner` | 54.7% | 63.4% |
| `eval`, a function of its own | 9.6% | inlined |
| `apply_inner`, `call`, `apply` | 16.0% | 21.7% |
| `variable` and `Frame::lookup` | 9.1% | 11.5% |
| The allocator (`dlmalloc`'s `malloc` and `free`, `deallocate`, `__rust_dealloc`) | 6.2% | 0.1% |
| The drop of a `Vec<Value>` | 1.5% | 0% |

- Two costs that native builds do not pay. The allocator: each call allocates its list of
  arguments, freed when its frame is released, and `dlmalloc` costs more than glibc's, which
  took 2.8% of the native profile at #5 for every allocation and copy. And `eval`: the module
  is built with `opt-level = "s"`, whose inliner left it a call for every node with its
  checks of the tracer and the stack and its result written to memory.
- #4 took the frame's allocation away, but not the list's; #10's stall needs a 16-byte load,
  which `wasm32` without SIMD does not make.

### The change

- An evaluation keeps up to 16 argument lists. A frame released at the end of a lambda's call
  gives its list back, emptied, and a call of a lambda takes one before evaluating its
  arguments. A built-in function's list goes with its arguments, so it is made as before. A
  function called back by a built-in one (`$map`) gives a list back without taking one, hence
  the bound. A first version let every call take a list; on the M1 it measured as the final
  one, the shipping rule and `filter + aggregate` 2 to 3% slower unaligned in `document` both
  times.
- `eval` and `eval_grow` are `#[inline(always)]` on `wasm32` alone. Natively it made
  `filter + aggregate` 8.7% and the shipping rule 5.5% slower in `document` on the Ryzen.
- In Node on the Ryzen, quick runs of one process: 525 µs at f418ca9, about 504 with the lists,
  about 500 with the inlining, 455 with both, more than the sum of the two.
- The module grows by 3 KiB gzipped (531 to 534). The depth tests of the WebAssembly suite
  (`D1011` at the default 500, 999 levels under `maxDepth: 1000`) pass.

### Results

**WebAssembly**, on the M1, `compare.mjs --pkg --engines jsonata-js`, four processes a build,
alternated, each on a new copy of its package, medians:

| | `evaluate` | `evaluateJson` | `evaluateDocument` | jsonata-js, the control |
|---|---:|---:|---:|---:|
| `recursion` | -15.2% | -13.4% | -15.6% | -0.5% |
| shipping rule | -3.1% | -4.0% | -7.4% | -0.9% |
| `filter + aggregate` | -4.1% | -2.0% | -9.1% | -1.5% |
| object constructor | -3.6% | -1.8% | -3.8% | +0.6% |
| `tuples` | -1.0% | -2.5% | -4.0% | +0.5% |

- Every other case is up to 4% faster or within 1.4%, but `path` and `names over items` in
  `evaluateDocument`, +7.7% and +4.1%: 0.6 to 0.7 µs and 3.7 to 3.8 µs, printed to a tenth, as
  in #7. With #7's -4.8%, `recursion` in `evaluateDocument` is about 20% faster than 0.31.0,
  as natively.
- These packages hold the lists and the inlining; the fix found by the property tests below
  changes only what `Expression::reads` gives for three functions.

**Natively**, `cargo xtask ab` against f418ca9, 10 rounds, medians, on the M1:

| | `document` | `evaluate` | `source` | aligned `document` | aligned `source` |
|---|---:|---:|---:|---:|---:|
| `recursion` | -4.1% | -5.8% | -6.0% | -7.4% | -8.8% |
| shipping rule | +2.7% | -0.6% | +2.4% | +1.2% | +0.8% |
| `filter + aggregate` | +2.6% | +1.2% | +1.2% | +1.3% | +0.8% |
| `sort + map` | +0.1% | +0.5% | +5.9% | +0.9% | +2.8% |

- Aligned is `-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5`
  on both builds. What stays past the M1's floor aligned is `sort + map` in `source`, whose
  evaluation is that of `document`, +0.9%, and `path` in `document`, 96 to 99 ns, which calls
  nothing: taken as placement.
- macOS's allocator is cheap, so the M1 gains less than WebAssembly. On the Ryzen under
  Windows, with the lists taken by every call (before they were kept to lambdas), `recursion`
  was 12 to 16% faster in each mode; `names over items` in `evaluate`, +3.0% (+3.9% alone), was
  +0.5% aligned. The Ryzen was in use during part of those runs.

### Found by the property tests

`PROPTEST_CASES=100000` found `($y := $shuffle(a, $x); 1)` giving `T0410` with the argument
as its `value` from `evaluate` and without it from a `Document`, at f418ca9 as well. An argument
past the parameters of `$append`, `$reverse` or `$shuffle` fails with `T0410`, whose `value`
is that argument, but the analysis of reads followed it as part of the result, as it follows the
others, so the bindings were cut to what the rest read. Such an argument is now read whole, as
`$merge`'s was in 0.31 #8.

## #2: `filter + aggregate` with Python values against 0.30.0

0.31 #7 found the case 3 to 4% slower than 0.30.0 on the M1 with Python values, and nowhere
else, without a cause. `python.py ab` of 0.30.0 against a71efdd (#3 to #6, #10, #14 to #17), on
the M1, twice, 5 rounds each, Python 3.14.10:

| | `evaluate` | `document` |
|---|---:|---:|
| `filter + aggregate` | -9.5%, -9.7% | -25.2%, -26.0% |
| `recursion` | -29.7%, -31.4% | -30.5%, -31.1% |
| shipping rule | -10.8%, -10.2% | -33.0%, -35.2% |
| `names over items` | -11.3%, -11.0% | -43.5%, -43.5% |
| object constructor | -13.1%, -13.7% | -20.2%, -20.6% |
| `tuples` | -11.8%, -12.3% | -18.5%, -19.7% |

- `filter + aggregate` with Python values takes 38.6 to 38.7 µs, against 42.7 to 42.8 µs at
  0.30.0: removed, and well past it. The cause of 0.31's +3 to 4% was not found. In #7 the same
  M1 gave 0.31.0 42.2 µs, against 0.30.0's 42.7 to 42.8 µs here: in separate runs, so not a
  proof, but no sign of it either.
- Every other case is faster or within 1.6% (`regex over a long text`, which evaluates few
  nodes); `strings` with Python values -6.4% and -3.2%.

## #19: where the allocations of `tuples` go

`tuples`, `items#$n@$i[$n % 2 = 0]{$i.type: $sum($i.(unitPrice * quantity))}`, made 1,237
allocations against a `Document` of the workload's 200 items at 0.31.0 (0.31 #3). Counted again at
a35cfd4 as in [0.27 #14](perf-0.27.md#the-tuples-of-a-path-027-14), with a global allocator that
records the size and the `jlens-core` functions of each allocation, one evaluation without its
JSON result: still 1,237. The path has one step, `items`, with a position and a focus, so it
makes 200 tuples, of which the filter keeps 100, in two groups.

| Where | Allocations |
|---|---:|
| Each tuple's object | 201 |
| Each tuple's fields: the one field of the tuple before (`@`, the document), copied with room for the step's three | 200 |
| The filter's frame for each tuple, and its list of bindings | 400 |
| The group-by's frame for each tuple it reads a key from, and its list | 200 |
| The group-by's copy of each tuple, made only to read its fields into its group's | 100 |
| The rest: the sequences, the groups, the vectors grown, the `$sum` calls | 136 |

A tuple that refers to the one before instead of copying its fields, the item's candidate,
saves no allocation: the new tuple still needs a list of its own for the fields its step adds,
and the copy is that list, made at its final size since 0.31 #3. With more steps the copy holds
more fields, but stays one allocation; sharing would save bytes copied, not an allocation, and
would change every reader of a tuple. The item stops there.

The frames are the largest part, 600 of 1,237: the filter and the group-by bind each tuple's fields
in a frame of their own, which ends with the predicate or the key, as a call's frame ends with
the call. #4 keeps such a frame for the next call, and #16 a call's list of arguments; neither
reaches these frames, made by `frame_from_tuple`. Together with the group-by's copy, which can read
each tuple in place, 700 allocations of the 1,237 are candidates: #20.
