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
