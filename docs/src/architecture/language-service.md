# Schema inference and the language service

The `jlens-schema` crate has four modules:

| Module | Role |
|---|---|
| `shape.rs` | The `Shape` model, incremental inference, overrides |
| `jsonschema.rs` | Conversion to and from JSON Schema 2020-12 |
| `typing.rs` | Static typing of expressions against shapes |
| `service.rs` | Completion, diagnostics, hover and signature help |

## Inference

A `Shape` describes all the values found at one place in the documents. `Shape::observe` folds
one more value into it:

- **Types.** Each value adds its JSON type. A number written without a fraction is an
  `integer`; one with a fraction or an exponent turns the shape into `number`. The
  `float_roundtrip` feature of `serde_json` preserves that distinction.
- **Objects.** Every field gets a sub-shape, and every object observed increments a counter.
  `required` is `field observations == objects observed`, and `presence` is the ratio of the
  two.
- **Arrays.** All items of all arrays observed here fold into one `items` shape.
- **Scalars:**
  - numbers update the minimum and the maximum;
  - strings narrow a common format;
  - up to ten distinct scalar values are tracked. They become an enumeration once at least two
    distinct values exist and some value repeats, which keeps unique identifiers out of
    enumerations.

Inference never looks back at earlier samples, so `Schema::observe` can process a stream of
documents in constant memory per distinct field.

## Static typing

A type `Ty` is a set of shapes plus a `partial` flag. The flag means that something unknown
contributed: a user function, an object constructor, or a missing schema. A shape that refers
to definitions of its schema, as the recursive `$ref`s of a JSON Schema do
([ADR 0024](../adr/0024-recursive-schemas.md)), is replaced in the set by the shapes it stands
for when a type is made of it, so each shape of a type is read with the definitions of its
schema, and a walk over every shape below (`**`) visits each once. `Walker::walk`
computes the type of each node and applies the evaluator's context rules:

- A **path** evaluates each step against the *elements* of the previous step's result. Elements
  are the shapes with arrays replaced by their items, recursively, which is the flattening the
  evaluator applies.
- **Stages** of a step (predicates and index bindings) see the step's elements. A group-by sees
  the elements of the path's result.
- `@$var` binds the step's elements and leaves the context unchanged. `#$i` binds a number.
- `$x := e` binds `x` to the type of `e` until the end of the enclosing block.
- **Lambdas** are walked with their parameters bound to unknown types. When a lambda is the
  callback of `$map`, `$filter`, `$single`, `$sort` or `$reduce`, its parameters get the element
  type of the array argument.
- **Calls** to built-ins that return their first argument (`$sort`, `$filter`, `$reverse`,
  `$distinct`, `$shuffle`, `$single`, `$clone`, `$sift`) keep its type. `$append` returns the
  union of its arguments' types. Other built-ins return scalars, except those whose structure is
  opaque, such as `$map` and `$lookup`, which return unknown types. A name bound outside the
  expression (a declared variable or host function) is not a built-in, and its calls return
  unknown types.

A type also tells whether its value may be undefined, by the span of the operand that makes it
so; nothing is warned by this alone. The context a node is walked against is always there, as
each item of a path is. A field step is undefined where the objects may lack the field (it is
not `required`), an item may not be an object, or the value it steps over may be an empty
array; `*` and `**` likewise. A path is undefined where one of its steps is, a filter may keep
nothing, or a step before the last gives an array, which may be empty. The array of the last
step is the result, empty or not, unless an earlier step gave several items, whose arrays are
flattened into a sequence, undefined when empty. A path that starts with an array constructor
is never flattened. Arithmetic, order comparisons and negation are undefined where an operand
is; `=`, `!=`, `in`, `and`, `or` and `&` are always there, and so is an array constructor. A
condition without `else` is undefined where its condition is false, `?:` where its fallback is,
and `??` where both sides are. A built-in's result follows a table in `typing.rs`, which a test
checks covers the catalog: never undefined (`$exists`, `$count`), undefined where its first
arguments are (most, as in jsonata-js), or also for some values (`$lookup`, `$filter`,
`$keys`, and `$max`, `$min` and `$average` of an array that may be empty). An array whose
`Shape::min_items` is at least 1 (`minItems`, given by inference to an array no sample had
empty, `x-jlens-min-items-inferred`) is never empty. A value computed from a value not fully
known (an operand of arithmetic or an order comparison, the argument of a built-in) is not
fully known either: it may be undefined.

A type also tells a sequence from an array. `Ty::sequence` is the span of the operand that may
make the value several values gathered in an array: a step that runs over several items (after
a step that gave arrays, but for one whose filters are all numbers, `items[0]`), a filter that
may keep several, `$filter`, `$keys`. `Ty::form` tells whether the value is the shapes
themselves or an array of them (an array constructor, `$sort`, `$append`, `$split`), or either.
Paths read the items of both. A filter's result is the items of its value, one level down (not
every level, as a step's context is). A path with a tuple stream (`@$v`, `#$i`) gives what its
steps run against, which the walker does not follow: its type is unknown. A built-in's result
has the type of its signature (`n`, `s`, `b`).

`LanguageService::result_type` turns the root's type into a `ResultType`: the JSON types it may
have (an array for a sequence or an array form, and the values of a sequence), and the spans
that make it undefined or a sequence. With an expected type declared
(`LanguageService::with_result_type`), the diagnostics add `W0212` (never one of the types,
over the whole expression), `W0213` (another type too: at the sequence's operand when the other
type is only the array a sequence makes, over the whole expression otherwise) and `W0214`
(undefined, at its cause). A text with a syntax error gets none. The suite test checks every
result the typing knows against the result of the case (1,047 of 1,401 expressions are typed,
805 always defined), and a property does the same for random expressions against their sample.

While walking, the walker reports *events* to a visitor:
- a field reference, with the context it is looked up in;
- a variable reference, with its type and the scope.

The services are all visitors over the same walk.

## Completion

Completion has to work on text that does not parse yet, so it proceeds in four steps:

1. **Lexical check.** If the cursor is inside a string or a comment, nothing is suggested. If it
   is inside a backquoted name, a quoted field name is completed.
2. **The word at the cursor** becomes the replacement range. If the previous token ends an
   operand (a name, a literal, or a closing bracket) and is not `and`, `or` or `in`, and there is
   whitespace before the cursor, operators are suggested instead.
3. **The placeholder trick.** The word is replaced with a sentinel identifier
   (`__jlens_cursor__`, or `$__jlens_cursor__` for variables), and the text is parsed with error
   recovery. The parse succeeds whenever the expression before the cursor is well formed,
   because the sentinel completes it. Anything broken after the cursor becomes an error node.
4. **The walk** finds the sentinel's event. For a field, it yields the context type, whose
   fields are the suggestions. It also yields the scope, whose variables are suggested with the
   declared host functions and the built-in functions.

The two field paths that matter most, `items.` and `items[`, are therefore handled by the same
machinery as `$map(items, function($i){ $i.`.

## Diagnostics, hover and signature help

- **Diagnostics** combine the recovering parser's errors and annotation warnings with `W0201`
  for every field reference whose context is fully known (`Ty::is_known_objects`) but lacks the
  field. A Levenshtein distance of at most 2 picks the "did you mean" suggestion. The walk also
  reports every call of a `$name` and every name it binds (`:=`, parameters, `@`, `#`); the calls
  are checked after the walk, because a function may call a name bound after it, or itself.
  Calls of unknown names get `W0202`, and calls whose argument count the signature rejects get
  `W0203`: parameters marked `?` or `-` are optional, and `+` accepts any number.
- The call events carry the type of each argument. Literals and operators have a JSON type
  (`Ty::literal`), schema fields have theirs, and anything else is unknown. `Ty::symbols` turns
  a fully known type into signature symbols, keeping the items of arrays apart, and the
  signature is matched as jsonata-js matches it (greedy, with backtracking), except that an
  argument whose type is unknown matches any parameter. A path may yield a single item of an
  array, so an array also matches what its items match. When no match exists, `W0205` names the
  argument jsonata-js would report in `T0410` or `T0412`. Undefined is left out: a field that is
  absent passes any signature, but one that is present cannot.
- When variables are declared, the declarations are taken to be complete, and references to
  names that are neither declared, bound anywhere in the expression, nor functions get `W0204`.
- The comparison events (`=`, `!=`, `in`) carry the left-hand side's type and the right-hand
  side's node, as for value completion. When every shape of that type declares an enumeration,
  directly or through its arrays' items, a literal on the right outside it gets `W0206`; values
  are compared as JSONata compares them, so `2.0` is `2`. `Shape::enumeration_inferred` marks an
  enumeration inferred from samples (in JSON Schema, `x-jlens-enum-inferred`), which never warns,
  and so does a type that may come from something unknown, or an enumeration that holds an array
  or an object. A literal with a predicate, `"a"[false]`, is not checked. The nearest string
  value, by the same Levenshtein distance, is suggested for a string. A shape with an open domain
  (`Shape::domain`, `x-jlens-domain`) never warns either.
- Without a declared enumeration, a literal on the right whose JSON type the left-hand side never
  has gets `W0210`, unless the left-hand side is a literal too. The operand events (`<`, `<=`,
  `>`, `>=` and arithmetic) carry both sides' nodes and types. A side whose present types, an
  array's replaced by its items, include none the operator takes gets `W0207` (no order) or
  `W0209` (not a number), the operand of a negation `W0211`, and two orderable sides with no type in common get `W0208`. A type that
  is not fully known, or holds arrays of arrays, is not checked. The suite test runs these over
  the official suite, which has one expected warning: a comparison guarded by `$type`, which
  the typing does not read.
- An operand that may be several values gets `W0215`: its type is a sequence (`Ty::sequence`,
  the span of the step over an array, the filter that may keep several items, `*` over an object
  of more than one field, `**`, `$keys`), or its known types include an array, which a path
  gives whole even of one item. The comparison events also carry the right-hand side's node and
  type for it: `=` and `!=` warn only where the other side is one value, `in` only on the left.
  A filter of a value that holds one value (no array, no sequence) keeps it or nothing, and a
  number filter anywhere among a step's filters keeps one item. An operand that is never there,
  with no shapes, is not warned about. A service test checks every operand built from a set of
  steps and filters against documents of one and two items: the warning is reported exactly
  where the operand evaluates to an array on one of them.
- **Value requests**: where values are completed, the first shape of the left-hand side (or of
  its arrays' items) with a domain becomes a `ValueRequest` beside the items. Its path is found
  by identity in the schema's tree of shapes, iteratively; a variable's shape has none. Its
  conditions come from the same probe: before the walk, an iterative search of the tree finds
  the comparison whose right-hand side is the placeholder and goes up through `and`, `or` and
  parentheses around one expression, keeping the comparisons (`=`, `in`) of each `and`'s other
  operand; the walk then types their left-hand sides, matched by address, and a field of the
  schema compared with literals becomes a `ValueCondition`. The service never fetches anything:
  the request is all it returns.
- **Hover** keeps the innermost field or variable event whose span contains the offset, and
  renders its type and documentation as Markdown.
- **Signature help** scans backwards from the cursor for the unmatched `(` of a call, ignoring
  strings and comments and balancing brackets. It counts the top-level commas to find the
  active parameter, and looks the function up among the declared host functions, then in the
  built-in catalog.
