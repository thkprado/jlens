# Performance notes: 0.37

How the measurements of roadmap 0.37 were made: methods, traps and intermediate numbers that the
[performance page](../src/architecture/performance.md) and the guides leave out. The item numbers
are those of the plan, in [the roadmap](../src/architecture/roadmap.md).

## #4: the Java package without critical downcalls

**What changed on the path.** The three calls that read a buffer (its address, its length, its
release) were made with `Linker.Option.critical`, which skips the thread's transition out of Java
(0.26 #7, about 20 ns). Each may sweep the conversions of a closed document, and each locks or
frees, while it holds the garbage collector off; #4 made them ordinary downcalls. Since 0.26 #10
an evaluation replies into a scratch buffer, so the evaluation of a `Document` makes none of
them; an evaluation of JSON text makes two (the input buffer's address and its release), Java
objects as input two per evaluation, a host function's call three or more. #4 also finds a
thread's conversion of a document by its handle, with one atomic load, instead of through the
thread's cache of the table's objects.

**Method.** On the M1, `cargo xtask java-compare ab A.jar B.jar <part>`, one part at a time
(`documents`, `cases`, `threads`), 5 rounds in alternation, a JVM and a fresh copy of the native
library per run, engines `jlens` and `jlens-objects`. A is `abf9f6d` (the code of `d342dba`, the
commit before #4), B the same with #4's commit applied; each JAR built by `cargo xtask java
package -DskipTests` in its own clone. About 35 minutes in all. Logs: M1
`~/workspaces/logs/037-4/`.

**Results.** One difference marked (above 5%, with no run of one JAR reaching the other's): `path`
against a `Document`, 0.210 to 0.198 µs (-5.6%), the conversion found by handle. Over JSON text,
every case of `cases` is within ±0.8% and every row of `threads` within ±0.9%: the transitions
the two calls make again do not show beside the text's parsing. Java objects over JSON text read
from -0.7% to +3.9% in `documents` and `cases`, within their spreads (up to 9.9%); those over
converted objects read from -4.7% to +3.6%. Host functions are not in the harness, so their
calls were not timed.
