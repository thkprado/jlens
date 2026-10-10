# Boolean functions

## `$boolean`

`$boolean(arg)` converts a value to a Boolean. The same rules apply wherever JSONata needs a
Boolean: in predicates, with `and`, `or`, `?:` and in conditions.

| Argument | Result |
|---|---|
| a Boolean | unchanged |
| a string | `false` if empty |
| a number | `false` if `0` |
| `null` | `false` |
| an array | `true` if any item converts to `true` |
| an object | `false` if empty |
| a function | `false` |
| nothing | nothing (`false` in conditions) |

```jsonata-example
[$boolean("0"), $boolean(0), $boolean([0, ""]), $boolean([0, 1]), $boolean({})]
=> [true, false, false, true, false]
```

## `$not`

`$not(arg)` converts `arg` to a Boolean and negates it.

```jsonata-example
[$not(true), $not(""), $not([1])]
=> [false, true, false]
```

```jsonata-example
input: library.json
library.books[$not("Kernighan" in authors)].title
=> ["Structure and Interpretation of Computer Programs", "Compilers: Principles, Techniques, and Tools"]
```

## `$exists`

`$exists(arg)` returns `true` if `arg` produces a value, even `null` or `false`, and `false` if
it produces nothing.

```jsonata-example
input: address.json
[$exists(Other.Misc), $exists(Other.Nothing), $exists(Phone[type = 'fax'])]
=> [true, false, false]
```

---

Adapted from the JSONata documentation (MIT license).
