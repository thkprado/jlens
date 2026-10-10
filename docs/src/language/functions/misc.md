# Other functions

## `$type`

`$type(value)` returns the type of `value`: `"null"`, `"number"`, `"string"`, `"boolean"`,
`"array"`, `"object"` or `"function"`. It returns nothing for nothing.

```jsonata-example
[$type(1), $type("1"), $type(null), $type([]), $type({}), $type($type)]
=> ["number", "string", "null", "array", "object", "function"]
```

```jsonata-example
$type(missing)
=> undefined
```

## `$error`

`$error(message?)` stops the evaluation with error `D3137` and the given message.

```jsonata-example
bindings: {"limit": -1}
$limit >= 0 ? $limit : $error("The limit must not be negative")
=> error D3137
```

## `$assert`

`$assert(condition, message?)` stops the evaluation with error `D3141` and the message when
`condition` is false. When it is true, it returns nothing.

```jsonata-example
input: address.json
($assert(Age >= 18, "Adults only"); FirstName)
=> "Fred"
```

```jsonata-example
input: address.json
($assert(Age >= 30, "Too young"); FirstName)
=> error D3141
```

## `$eval`

`$eval(expression, context?)` parses and evaluates a JSONata expression given as a string. The
context value is `context` if given, the current context otherwise. A syntax error in
`expression` fails with `D3120`; an evaluation error, with `D3121`.

```jsonata-example
$eval("[1, $string(2), 3]")
=> [1, "2", 3]
```

```jsonata-example
input: address.json
$eval("FirstName & ' ' & Surname")
=> "Fred Smith"
```

```jsonata-example
$eval("a + b", {"a": 1, "b": 2})
=> 3
```

## `$clone`

`$clone(value)` returns a deep copy of an object or array. [Transforms](../programming.md#transforms)
use it to copy their input.

```jsonata-example
$clone({"a": [1, {"b": 2}]})
=> {"a": [1, {"b": 2}]}
```

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).
