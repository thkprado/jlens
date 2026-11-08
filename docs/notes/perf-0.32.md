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
