# 0018 — A deterministic step limit

**Status:** Accepted.

## Context

An application that evaluates expressions written by others needs a ceiling on what one
evaluation may cost. jlens has three ceilings, and none is both general and reproducible:

- `max_depth` (`D1011`) stops a deep recursion, and the size limits (`U1001`) stop a huge
  value, the same way on every machine. Loops over small arrays nested a few times pass both:
  `$sum($map([1..1000], function($i) { $sum($map([1..1000], function($j) { $i * $j })) }))`
  builds no array longer than 1,000 items and runs a million multiplications.
- The timeout (`D1012`) stops them, but it reads the clock. The same expression on the same
  input passes on a fast machine and fails on a slow one, or passes natively and fails in
  WebAssembly. A replay, a test that compares two platforms, or a quota charged to a user
  cannot rely on it.
- A cancellation token (`U1003`) is driven by the application, so it is as reproducible as
  whatever drives it. Counting the `enter` events of a tracer and cancelling after *n* of them
  is today's workaround; it forces the traced evaluation, which is slower (ADR 0004).

The evaluator already has the place where such a limit belongs. `Evaluator::guardrails` in
`eval.rs` checks the depth, the input's read failures, the timeout and the token. It runs for
each node the general path evaluates (`eval_inner`, where a tracer gets its `enter` event), and
for each function a built-in or a host function calls back (`Evaluator::callback`). A few fast
paths skip evaluating a node, and so skip the check, but only when no tracer is set:

- a plain field looked up in each item of a path step (`Evaluator::step`), which calls
  `guardrails` in place of the step's evaluation;
- `=` and `!=` with a string literal, which calls `literal_guardrails` in place of the
  literal's evaluation;
- a literal key of a group (`Evaluator::group` and the single-item object constructor), which
  checks nothing for the key;
- a constant predicate (`filter_constant`), evaluated against the first item only instead of
  once per item.

A prototype that decrements a counter at each check measured no cost (`cargo xtask ab`: every
case between 3.4% faster and 1.3% slower than `HEAD`, which is noise). The same prototype found
that a timeout costs a clock read at every step: `$sum($map([1..300000], function($x) { $x * 2 +
1 }))` took 189 ms with `jlens eval --timeout` against 127 to 153 ms without (Windows).

Choices to make:

1. What a step is, so that the count is the same on every machine, with or without a tracer,
   whatever the optimizations.
2. What a regular expression costs, since one match can backtrack for years (ADR 0009).
3. The error, and what the limit does not bound.
4. Whether counts may change between versions, and whether an evaluation reports its count.
5. How the counter changes the timeout.

## Decision

`Options::max_steps: Option<u64>` (builder `Options::max_steps(n)`), with no default: like the
timeout, a limit an application does not set changes nothing.

1. **A step is a check of the evaluation's limits on the general path.** Concretely:
   - each node the general path evaluates: one call to `eval_inner`, which is one `enter`
     event for a tracer;
   - each function called back through `Evaluator::callback`: by a built-in that takes a
     function (`$map`, `$filter`, `$sort`, `$replace` with a function, …) and by a host
     function through its `Callable` (ADR 0011). The body of a lambda called back then counts
     its own nodes;
   - one step for each check that the regular expression backtracker makes (point 2).

   The fast paths charge the steps they skip, so that the count is the one the general path
   would take:
   - the field looked up in each item charges one step, as the evaluation of the `Name` node
     would (it does today, through `guardrails`);
   - `=` and `!=` with a string literal charge one step for the literal (they do today,
     through `literal_guardrails`);
   - a literal key of a group charges one step for each item it is looked up for;
   - a constant predicate charges, after its one evaluation, the steps of that evaluation
     once more for each further item. The predicate gives the same value for each item and
     reads nothing from it, so its count is the same for each.

   `$eval` runs in the same evaluator, so the text it evaluates spends the same budget, and
   parsing that text costs no step. A tail call run by the trampoline counts the nodes of the
   body it evaluates, like any other evaluation. A budget of *n* allows exactly *n* steps: the
   evaluation that takes *n* steps succeeds with `max_steps(n)` and fails with
   `max_steps(n - 1)`. A fast path that charges several steps at once fails when fewer remain,
   so the outcome at any budget is the general path's.

2. **A regular expression costs one step per 1,024 steps of the backtracker.** The backtracker
   of `jlens-regress` already counts its own steps and calls an interrupt check every
   `INTERRUPT_INTERVAL` (1,024) of them, counting afresh for each search (ADR 0009). With a
   step limit set, every match takes the interruptible path, as it does with a timeout or a
   token, and each check charges one step. A search that backtracks less than 1,024 times
   costs nothing beyond the node that called it. The backtracker's count depends only on the
   pattern, the text and the start, so it is the same on every platform (`memchr` finds the
   same positions with or without SIMD). A catastrophic pattern therefore stops under a step
   limit alone, about every 10 µs of matching per step charged (ADR 0009's measurements).

3. **The error is `U1006`**, with the limit as `value`: "The evaluation exceeded its budget of
   {{value}} steps". It is a new code because an application must tell an exhausted budget from
   a size limit (`U1001`, which carries only a message) without reading the message, as it
   already can for `D1011` and `D1012`; `U1005` is taken. The rule "limits use `U1001`" of
   `AGENTS.md` becomes "size limits use `U1001`; the step limit uses `U1006`". The error's
   position is the node being evaluated when the budget ran out, which, after a fast path
   charged in bulk, may be an enclosing node, and may differ with a tracer. The code and value
   are what is reproducible.

   **What the limit does not bound** is the work inside one step, which the size limits
   already bound: a range (`[1..n]`, up to `max_array_length` items), `$sort` without a
   function (its comparisons), `$string` and the conversion of the result, deep equality,
   `$join`, `$reverse`, `$distinct`, reading the input and parsing a `Document`. The work a host
   function does in its own code is the application's to bound; only its calls back count.
   Under a step limit, the time of one evaluation is bounded by the steps times the largest
   step, not by the steps alone.

   Reproducible across platforms means within the other limits: the default `max_depth` is
   50,000 natively and 500 in WebAssembly, and the stack can run out at different depths. An
   application that needs the same outcome everywhere sets `max_depth` too, and the guides say
   so. With a timeout and a step limit set, whichever is reached first stops the evaluation;
   only `U1006` is reproducible.

4. **Counts change only on purpose, and are reported by audits.**
   - The table test of exact counts (#2) makes any change visible in review. A version that
     changes counts says so in its changelog, under its own heading, with the cases that
     changed. Before 1.0 that is not a breaking change; 0.26 decides what 1.0 promises.
     An upgrade of `jlens-regress` can change the counts of regular
     expressions, and is reviewed the same way.
   - An evaluation does not return its count: the `evaluate*` methods keep their return types
     until 0.26 settles the API. The audit header gets a `steps` field, the steps the audited
     evaluation took, which equal the untraced count by point 1. An application measures an
     expression with an audit and sets its budget from that; the Wasm and Python audit types
     gain the field with their `maxSteps` / `max_steps` (#4, #5). Whether 1.0 returns the count
     of every evaluation, for a quota that charges what was used, is a question for 0.26.

5. **The counter paces the timeout.** The cancellation token, an atomic load, is still read at
   every step. The clock is read only every *N* steps of the same counter. Proposed: a mask
   on the remaining count. Settled in #3: the counter counts down to the next checkpoint,
   the end of the budget or, with a timeout, the next read of the clock, and the rest of the
   budget waits in a reserve. A step then costs one comparison, as without a timeout, and the
   checkpoint is the branch already taken when the budget runs out. A mask was measured too:
   the fast paths spend several steps at once and can step over every multiple of *N*, and
   testing for that on each step made two cases of the workload 5% slower. *N* is 256.
   Target: the `$map` above with a timeout within 2% of the time without one, natively and in
   WebAssembly; measured, 0.8% natively (47% before) and 1.3% in WebAssembly (149% before).
   A timeout then fires up to *N* steps late, which is at most *N* times the largest step;
   regular expressions keep reading the clock at each of their checks.

## Alternatives considered

- **Counting only the general path,** with the fast paths free. Simpler, but the count would
  depend on the tracer (a traced evaluation takes the general path) and on every future
  optimization: the audit could not measure the budget, and a faster release would change
  every count.
- **Weighted steps,** charging a range or a sort in proportion to its size, closer to CPU time.
  The weights are arbitrary, every built-in would need one, and the size limits already bound
  the work inside a step. A count of evaluations is simple to explain and to keep exact.
- **The tracer workaround,** counting `enter` events in the application. It already works but
  forces the slower traced path, misses callbacks of built-ins and regular expressions, and
  every binding would reimplement it.
- **Reusing `U1001`** with a message, or `D1012`. An application could not tell a budget from
  a size limit, or from a clock, without matching messages, which `AGENTS.md` forbids.
- **Returning the count from every evaluation** now. It changes the signatures of every
  `evaluate*` method in Rust, Wasm and Python before 0.26 decides the API.
- **Fuel from the runtime,** as `wasmtime` meters WebAssembly instructions. It exists only for
  one runtime, not natively nor in browsers, and instruction counts change with the compiler.

## Consequences

- A loop that the depth and size limits let through stops with `U1006` at the same step on
  every machine, natively and in WebAssembly, with or without a tracer, including inside a
  catastrophic regular expression.
- Without a limit the untraced path does one more decrement and comparison per step, which the
  prototype measured as noise; #2 confirms it with `cargo xtask ab`, against `HEAD` and with
  `--b-max-steps`, no case more than 2% slower.
- With a step limit set, matches take the interruptible path of `jlens-regress`, measured in
  ADR 0009 as within the noise except one backreference case (10% slower).
- The fast paths gain a charge each, and every future fast path must charge what it skips:
  the test that runs every case of the suite with and without a tracer and compares the counts
  catches one that does not.
- jsonata-js has no such limit; the compatibility page lists `U1006` next to the timeout.
- `U1006` joins `error.rs` and the generated error page; `AGENTS.md` changes as point 3 says.
