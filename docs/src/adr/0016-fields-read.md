# 0016 — The fields an expression reads

**Status:** Accepted.

## Context

An application that evaluates rules builds their input first, often from a database or a
service. Knowing which fields a rule reads, it could fetch only those, and show which rules
depend on a field. Roadmap 0.18 asks jlens to answer from the expression alone, for one
expression or for several evaluated on the same input, and later to skip, when it parses JSON
text, the fields no evaluation will read.

[ADR 0010](0010-lazy-input.md) already reads an input lazily, one level or one field at a time.
That serves an input the application holds in memory. It cannot tell the application, before
the input exists, what to fetch, and it reads JSON text whole.

Three facts bound the design:

- **JSONata is dynamic.** A field name can be computed (`$lookup($, $name)`), a value can reach
  a function whose body is unknown, and `$eval` runs an expression given as a string. The
  answer is an upper bound: everything the evaluation may read, and as little more as the
  analysis can manage.
- **A wrong answer is silent.** A field left out reads as undefined, and the rule gives another
  result without failing. The analysis needs a test over every expression jlens has, not a few
  examples.
- **The analysis knows no types.** `items.quantity` reads `quantity` in `items` when `items` is
  an object, and in each of its items, at any depth, when it is an array: the path step maps
  over arrays (`lookup` in jsonata-js). The analysis cannot tell which, so its answer must hold
  for both.

## Decision

### 1. What the analysis returns

`Reads` holds what an expression reads of its input and of each binding: a `Read` for the input
(none when the expression does not read it), a `Read` for each binding it reads, by name, and a
flag, *every binding*, that `$eval` sets (decision 7). A `Read` says what is needed of a value,
whatever its type:

- **`Whole`**: the value as it is.
- **`Fields`**, a map from field names to their own `Read`: of an object, only these fields; of
  an array, its items, each needing the same `Fields`, at any depth; of a scalar, the scalar.
  An empty map needs the value but none of its fields: what `$count`, `$exists` and `$type`
  read.

A field absent from a map is not read. The items of an array share the node of the array
because a path step does not distinguish them; the analysis never tracks indices.

Its JSON form, which the WebAssembly and Python packages and the CLI use, writes `Whole` as
`true` and `Fields` as an object:

```text
$sum(items[type = "book"].(price * quantity)) * (1 - $discount)

{"input": {"items": {"type": true, "price": true, "quantity": true}},
 "bindings": {"discount": true}}
```

**Cutting** a value by a `Read` keeps what it says: `Whole` keeps the value; `Fields` keeps an
object's listed fields, each cut by its own `Read`, cuts each item of an array by the same
`Fields`, and keeps a scalar. **The guarantee**: an expression evaluated on its input and
bindings cut by its `Reads` gives the same result, or the same error code, as on the whole
ones. Truthiness is the reason a test reads its value whole: cutting every field of an object
makes it `{}`, which is false.

### 2. The union

`Reads` and `Read` have a union: what either reads. `Whole` absorbs everything; two `Fields`
merge their maps, uniting the `Read`s of a field present in both; the input and each binding
unite the same way, and the flag is or-ed. The union is commutative, associative and
idempotent. An input cut by the union of several expressions' `Reads` gives each of them its
result, since each `Read` of the union contains that expression's own. The analysis stays per
expression; an application unites the results it needs, and decision 9's parse takes a result,
not an expression, so that one `Document` serves several rules.

### 3. How the analysis works

It walks the processed tree once (`Expression::ast`, where `%` slots, stages and groups are
resolved), as an abstract evaluation. The abstract value of an expression is a set of
*places*: the input or a binding, followed by field names. Reading field `f` of a value
extends each of its places by `f`; *using* a value, as an operand, an argument or the result,
marks its places `Whole`. Literals and computed values (a sum, a string, a new object) have no
places: whatever they were computed from was marked when they were computed. The context `$`
starts as the input's place, `$$` is always that place, a local variable has the places of its
value, and a variable bound nowhere in the expression that is not a built-in is a binding.
The value of the whole expression is used.

The places stay finite: they grow by one name per step of the tree, and decision 6 cuts every
cycle through functions. Recursion goes through `stack::grow`, and the parser's cap of 1,000
levels bounds the depth.

| Construct | Reads | Places of its value |
|---|---|---|
| A literal: string, number, boolean, `null`, regex | — | none |
| `$` | — | the context's |
| `$$` | — | the input's |
| `$x`, bound in the expression | — | those of its value |
| `$x`, bound nowhere and not a built-in | — | the binding `x` |
| A field name `f` | — | the context's, each extended by `f` |
| A path `a.b.c` | each step with the previous step's value as context | the last step's |
| A filter `[p]` | `p`, with the item as context, used | the step's |
| A sort `^(k, …)` | each key, with the item as context, used | the step's |
| A group `e{k: v}` | `k`, with the item as context, used; `v`, with the group (the places of `e`) as context, used | none |
| `*`, `**` | the context, whole | none |
| `%` | — | decision 4 |
| `-x`, `+ - * / %`, comparisons, `&`, `in`, `..`, `and`, `or` | the operands, used | none |
| `c ? t : e` | `c`, used | the union of `t`'s and `e`'s |
| `a ?: b` | `a`, used | the union of `a`'s and `b`'s |
| `a ?? b` | `a`, needed (an empty `Fields`) | the union of `a`'s and `b`'s |
| `[a, b, …]` | — | the union of the items' |
| `{k: v, …}` | keys and values, used | none |
| `(a; b; …)` | each expression, in a scope of its own | the last expression's |
| `$x := v` | `v` | `v`'s, which `$x` takes |
| A function call, `x ~> f(…)` | decision 5 | decision 5 |
| `function(…) {…}`, a partial application | decision 6 | none |
| A transform `\| p \| u, d \|` | decision 6 | none |

### 4. `@`, `#` and `%`

- **`step@$v`** gives `$v` the places of the step's value; the next step keeps the step's
  context, as the evaluator does.
- **`step#$i`** gives `$i` no places: it is a number.
- **`%`** has the places of the context of the step that binds its slot: the evaluator binds
  the slot's label to that context (`ancestor_label`). `Order.Product.{"order": %.OrderID}`
  reads `OrderID` of each order, not the order whole. The roadmap listed `%` among the
  constructs that read whole; following the slot costs no more than following a variable.

### 5. Functions

A function call uses its arguments and returns a value without places, except for the
built-ins below. A built-in whose signature takes the context when an argument is missing
(`-`) reads the context as that argument.

| Function | Reads | Places of its value |
|---|---|---|
| `$count`, `$exists`, `$type` | the argument, needed (an empty `Fields`) | none |
| `$lookup(x, "k")`, with a literal key | — | `x`'s, each extended by `k` |
| `$append`, `$reverse`, `$shuffle`, `$zip`, `$merge` | — | the union of the arguments' |
| `$filter(a, λ)`, `$single(a, λ)`, `$single(a)` | `λ`'s body, its value used | `a`'s |
| `$map(a, λ)` | `λ`'s body | those of `λ`'s body |
| `$sort(a, λ)` | `λ`'s body, its value used | `a`'s |
| `$reduce(a, λ, init)` | `init` and `λ`'s body, used | none |
| `$reduce(a, λ)` | `λ`'s body, used | `a`'s |
| `$eval` | decision 7 | none |
| Every other built-in, `$lookup` with a computed key among them | the arguments, used | none |
| A binding: a host function | the arguments and the context, used | none |
| Another function: a local variable, a computed value | the arguments, used | none |

Here `λ` is a lambda written as the argument. Its body is analysed where the call is, with the
call's context, and its parameters take places: the item parameter of `$filter`, `$single`,
`$map` and `$reduce`, both parameters of `$sort`, and the array parameter, have the places of
`a`; the index and `$reduce`'s accumulator have none, which is sound because what flows into
them is used. Without `init`, `$reduce` starts from the first item and returns it when it is
the only one, so its accumulator and its value have the places of `a`. When the function
argument is anything else, `a` is used whole.

A host function can read the context of its call (`HostCall::input`), so a call to a binding
uses the context: `$tax(price)` at the top of an expression reads the whole input, and
`price.$tax()` reads `price`. A call through a local variable or a computed value uses the
context only when a binding may have become that function (a binding used as a value) or a
function of the expression takes its context (`-` in a lambda's signature, or such a built-in
taken as a value); the analysis decides once it has seen the whole expression.

A call to a binding reads the whole input when it stands at the top of an expression, and
`Whole` absorbs the rest of a union: one such rule would keep a whole document for every rule
evaluated on it. So an application can mark a host function as not reading the context of its
call. `HostCall::input` then gives nothing, so the mark is a guarantee rather than a promise,
and the analysis, given the bindings (`Expression::reads_with`), reads only the arguments of a
call to a marked function. Without the mark a host function reads its context, as before.
Making that the default instead would change the API, a question for its stability (roadmap
0.19).

*Amended with the implementation (roadmap 0.18, item 2): the rows for a binding and for
`$reduce` without `init` were found then.*

Every built-in not in the table falls in the default row, which is always sound: a new built-in
needs no entry to be safe, only to be precise. The table lives next to the catalog in
`functions/`.

A binding or host function named after a built-in shadows it during the evaluation, but the
analysis takes the name as the built-in's. The guides say so: such an application cannot cut
its input by the analysis for the rows above.

### 6. Lambdas and transforms

A lambda that is not the argument of decision 5's table is analysed once, where it is defined:
its parameters have no places, its context is the context of its definition (as the evaluator
binds it), and its body's value is used. Its calls use their arguments. A value passed to a
function the analysis does not follow is therefore read whole, and the function's body adds
only what it reads by other ways: the context of its definition, `$$`, outer variables,
bindings. This holds for recursion without a fixed point.

A transform is a function: what it is applied to is used (its copy is made from the whole
value), and its pattern, update and delete are analysed as a lambda's body, with a context of
no places.

### 7. Where it gives up

The analysis reads a value whole where it cannot follow it: `*` and `**`, `$keys`, `$string`,
`$each`, `$sift`, `$spread`, `$lookup` with a computed key, a value passed to a function other
than decision 5's lambdas (a lambda bound to a variable, a host function, a transform), the
values of a constructed object, and the result.

`$eval` gives up on everything: the expression it parses can read any variable of the
environment in which it is called, the bindings included. Its `Reads` is the input whole, every
binding the expression names whole, and the *every binding* flag, which tells the application
to keep all its bindings.

### 8. The API

- **Rust:** a module `jlens_core::reads` with `Read` and `Reads`; `Expression::reads()`, and
  `Expression::reads_with(&Bindings)`, which knows the host functions marked as not reading
  their context (decision 5);
  `union` on both; `Read::cut` on a `serde_json::Value`; accessors for the input, a binding and
  the flag; `Serialize` and `Deserialize` to the JSON form. The exact names are settled with
  the implementation, and `cargo-semver-checks` reviews them.
- **WebAssembly:** `Expression.reads(bindings?): Reads` and `unionReads(reads: Reads[]): Reads`,
  with `type Read = true | { [field: string]: Read }` and `interface Reads { input?: Read;
  bindings: Record<string, Read>; everyBinding?: true }`. A JavaScript function cannot read the
  context of its call (`this` is `undefined`), so the package marks every one, and `reads` takes
  the bindings, as `reads_with` does.

  *Amended with the implementation (roadmap 0.18, item 3): `reads` takes the bindings.*
- **Python:** `Expression.reads(bindings=None)`, a `dict` in the JSON form, and
  `jlens.union_reads(reads)`, with the stubs typing both. A Python function does not receive the
  context of its call, so the package marks every one, as the WebAssembly package does.

  *Amended with the implementation (roadmap 0.18, item 3): `reads` takes the bindings.*
- **CLI:** `jlens reads`, given an expression or one or more `--file`s, prints the JSON form on
  stdout: one `Reads` for one expression, an object keyed by file for several, or their union
  with `--union`. It follows [ADR 0008](0008-cli-machine-contract.md): a parse error fails as
  `check` does.

### 9. Parsing only the fields read

Roadmap 0.18, item 4, gives `Document::parse` a `Read` (one expression's, or a union's) and
skips, while it parses, the fields it does not list, building the document cut by it. It is
kept only if the parse of wide documents measures faster; the performance page records the
outcome either way.

## Alternatives not taken

- **Reusing the language service's typing** (`jlens-schema`). It types an expression against a
  schema to complete and check it, where being approximate is harmless; it lives above the
  core, and decision 9 needs the analysis in the core.
- **Recording what evaluations read**, through the tracer of [ADR 0004](0004-audit-tracer.md),
  on sample inputs. It gives a lower bound, the opposite of what cutting needs: an input unlike
  the samples reads fields they never reached.
- **Separate nodes for an array and its items, or for indices.** The path step treats an array
  and its items alike, and the analysis knows no types; two nodes would always be merged.
- **A flat list of dotted paths.** Field names may hold dots and any character, and the tree
  gives such a list easily.
- **Following every lambda into each of its calls**, with a cap on recursion. More precise for
  functions bound to variables, at the cost of an analysis that grows with the calls. It can
  come later if rules ask for it.

## Consequences

- The paths, comparisons, filters and aggregations rules are mostly made of are exact.
  Expressions that use `*`, `**`, `$string`, `$eval` or their own functions on input values
  read much of it whole.
- The analysis follows the evaluator: a new construct, or a change in how one reads its
  operands, must change it. The default for functions keeps new built-ins sound. The test is
  the suite: every case evaluated on its input and bindings cut by its `Reads` gives the same
  result or error code. Property tests check that random expressions never panic, that deep
  ones stay within the nesting cap, the laws of the union, and that an input cut by the union
  of several results gives each expression its result.
- An application that shadows a built-in with a binding or a host function cannot rely on the
  rows of decision 5 for it.
- The analysis runs on demand, not when an expression compiles; its cost is linear in the tree
  times the places it carries.
- Implementation, in order: the analysis, the cut and the Rust API, with the tests above
  (roadmap 0.18, item 2); the WebAssembly and Python packages and the CLI (item 3); the pruned
  parse (item 4).
