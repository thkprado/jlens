# 0008 — A machine contract for the CLI

**Status:** Accepted

## Context

Scripts, CI jobs and coding agents run `jlens` and read what it prints. Today they parse output
written for people:

- `jlens eval` prints the result as JSON, but an undefined result prints nothing, which looks
  the same as a failure that printed nothing. Errors go to standard error as one line of text,
  with the position inside the message.
- With `--ndjson`, failures are text on standard error, with a line number in the message; the
  results on standard output carry no line number at all, so a consumer cannot pair a result
  with its input line once a line has failed.
- `eval` reads standard input when no `-i` is given. Started from a terminal by mistake, it
  waits forever, which an agent cannot tell from a slow evaluation.
- Results of `$now()`, `$millis()`, `$random()` and `$shuffle()` change from run to run, so
  the same command cannot be replayed to check a result.

Choices to make:

1. **How output becomes machine-readable.** Detect whether standard output is a terminal and
   switch format (as `ls` and `git` do), or add an explicit flag.
2. **The shape of a result.** The bare value, as today, or a value wrapped in an object that
   also says whether the evaluation succeeded.
3. **Where errors go** in machine mode: standard error, as today, or standard output next to
   the results.

## Decision

### `--json`: one envelope per evaluation

`jlens eval --json` prints one compact JSON object, the **envelope**, per evaluation, on one
line of standard output:

| Outcome | Envelope |
|---|---|
| A result | `{"ok":true,"result":…}` |
| An undefined result | `{"ok":true,"undefined":true}` |
| An error | `{"ok":false,"error":{"code":…,"message":…,"position":…,"line":…,"column":…}}` |

- `undefined` is a separate field because `null` is a valid result.
- `code` is the JSONata error code (`S0202`, `T2001`, `D1012`, `U1001`…), the one stable thing
  to match on. `message` is for people and may change.
- `position` is the JSONata position of the error, a UTF-16 offset into the expression, as in
  jsonata-js. `line` and `column` are 1-based and point at the same place, with the column
  counted in characters, as the human report prints it. The three fields are absent when the
  error has no location.
- With `--ndjson`, every envelope also carries `"line"`, the number of its input line, so
  results and errors can be paired with their input. Blank lines are skipped and produce no
  envelope.
- With `--filter`, a matching document is the `result`. Documents that do not match produce no
  envelope; errors still do.
- An input document that is not valid JSON is reported in an envelope too, with code `U2001`.
- `--json` implies `--compact`. It cannot be combined with `--raw-output`, `--explain` or
  `--audit`, which have their own output formats.
- Fields are only ever added to the envelope, never renamed or removed. Consumers must ignore
  fields they do not know.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Every evaluation succeeded |
| `1` | The expression has errors: a syntax error, an evaluation error, or (with `--ndjson`) at least one line that failed |
| `2` | Usage or I/O problem: bad arguments, an unreadable file, a single input document that is not valid JSON, a terminal on standard input |

The codes are the same with and without `--json`; only the output format changes.

### Standard output is data, standard error is for people

- Standard output carries only data: results, envelopes, diagnostics with `--json`, catalogs.
- Standard error carries text for people: rendered diagnostics, warnings, usage errors.
- In `--json` mode the envelope is complete: an expression error is in the envelope on
  standard output, and nothing needs to be read from standard error.
- Problems with exit code 2 are reported on standard error only, and standard output stays
  empty. They happen before any evaluation, and some of them (bad arguments) are detected
  before `jlens` knows which format was asked for. A single input document that is not valid
  JSON is the exception: in `--json` mode it is also reported as an envelope.

### Never wait on a terminal

`eval` without `-i` or `--no-input` fails with exit code 2 when standard input is a terminal,
instead of waiting for a document that will never come.

### The default stays human

Without `--json`, output is unchanged: results printed like `jq`, diagnostics rendered with the
source line and a caret. The format never depends on whether standard output is a terminal, so
a command behaves the same when an agent runs it and when a person copies it into a shell.

### Related commands

- `jlens check --json` prints one compact JSON array of diagnostics, each with `file`, `line`
  and `column` next to the code, message and byte span. `--deny-warnings` makes warnings fail
  with exit code 1.
- `jlens functions --json` and `jlens errors --json` print their catalogs on standard output.
- `jlens eval --now` and `--seed` fix the clock and the random generator, so that the same
  command, expression and input print the same bytes.

## Consequences

- Machine mode is one flag that a script or an agent passes on purpose. Nothing switches
  format behind its back, and the human output can keep improving without breaking anyone.
- A consumer reads one line per evaluation, decides on `ok`, and matches errors by `code`. It
  never parses messages or standard error.
- Envelopes cost a few bytes per result. Pipelines that want bare values, such as
  `jlens eval --ndjson | jq`, keep using the default output.
- `U2001` is a new code, for input rather than expressions. It joins the error catalog, and
  so the generated error reference.
- The contract is public: changing it takes a new ADR, and the envelope only grows.
