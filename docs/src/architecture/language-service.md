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
contributed: a user function, an object constructor, or a missing schema. `Walker::walk`
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
- **Hover** keeps the innermost field or variable event whose span contains the offset, and
  renders its type and documentation as Markdown.
- **Signature help** scans backwards from the cursor for the unmatched `(` of a call, ignoring
  strings and comments and balancing brackets. It counts the top-level commas to find the
  active parameter, and looks the function up among the declared host functions, then in the
  built-in catalog.
