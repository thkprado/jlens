# Operators

## Arithmetic

| Operator | Meaning |
|---|---|
| `+` | Addition |
| `-` | Subtraction, and negation in prefix form (`-x`) |
| `*` | Multiplication |
| `/` | Division |
| `%` | Remainder, with the sign of the left operand |

Both operands must be numbers (error `T2001`, `T2002`), and a result that is not finite (such
as `1/0`) is an error (`D1001`). If either operand is nothing, the result is nothing.

```jsonata-example
5 + 2 * 3
=> 11
```

```jsonata-example
5 / 2
=> 2.5
```

```jsonata-example
-7 % 3
=> -1
```

```jsonata-example
input: invoice.json
Account.Order[0].Product[0].(Price * Quantity)
=> 68.9
```

```jsonata-example
"5" + 1
=> error T2001
```

```jsonata-example
missing + 1
=> undefined
```

Numbers are 64-bit floats, as in JavaScript, and results are printed the way JavaScript prints
them:

```jsonata-example
0.1 + 0.2
=> 0.30000000000000004
```

## Comparison

| Operator | Meaning |
|---|---|
| `=` | Equal: same type and value; arrays and objects are compared deeply |
| `!=` | Not equal |
| `<`, `<=`, `>`, `>=` | Order of numbers, or of strings (by Unicode code point) |
| `in` | The left value is one of the values on the right |

`=` and `!=` never fail: values of different types are simply not equal. The ordering operators
need two numbers or two strings (errors `T2009`, `T2010`). Comparing with nothing gives
`false`.

```jsonata-example
1 + 1 = 2
=> true
```

```jsonata-example
"1" = 1
=> false
```

```jsonata-example
{"a": [1, 2]} = {"a": [1, 2]}
=> true
```

```jsonata-example
"apple" < "banana"
=> true
```

```jsonata-example
"world" in ["hello", "world"]
=> true
```

```jsonata-example
"hello" in "hello"
=> true
```

```jsonata-example
input: address.json
"01962 001234" in Phone.number
=> true
```

## Boolean

`and` and `or` convert their operands to Booleans as [`$boolean`](functions/boolean.md#boolean)
does, and stop as soon as the result is known: the right side of `false and x` is never
evaluated. Negation is a function, [`$not`](functions/boolean.md#not).

```jsonata-example
input: library.json
library.books["Aho" in authors and price < 50].title
=> "Compilers: Principles, Techniques, and Tools"
```

The other book by Aho has no `price`, and comparing nothing with `50` is false.

```jsonata-example
0 or "yes"
=> true
```

```jsonata-example
$not(true and false)
=> true
```

## Strings

`&` joins strings. It is the only operator that converts its operands: non-strings become
strings as by [`$string`](functions/string.md#string), and nothing becomes the empty string.

```jsonata-example
input: address.json
FirstName & " " & Surname
=> "Fred Smith"
```

```jsonata-example
5 & 0 & true & missing
=> "50true"
```

## Conditionals

`condition ? then : else` evaluates `then` when the condition is true, and `else` otherwise.
The condition is converted to a Boolean. Without an `else` part, a false condition produces
nothing.

```jsonata-example
input: invoice.json
Account.Order.Product.(Price > 100 ? "premium" : "standard")
=> ["standard", "standard", "standard", "premium"]
```

```jsonata-example
false ? "yes"
=> undefined
```

## Defaults

| Operator | Result |
|---|---|
| `a ?: b` | `a` if it is true as a Boolean, else `b`. A fallback for missing *or* falsy values (`null`, `false`, `0`, `""`) |
| `a ?? b` | `a` if it exists (is not nothing), else `b`. A fallback only for missing values |

```jsonata-example
input: address.json
Other.Misc ?: "n/a"
=> "n/a"
```

```jsonata-example
input: address.json
Other.Misc ?? "n/a"
=> null
```

```jsonata-example
0 ?? 1
=> 0
```

## Chaining: `~>`

`value ~> $f(args)` calls `$f(value, args)`. Chains read left to right instead of inside out:

```jsonata-example
input: invoice.json
Account.Order.Product.(Price * Quantity) ~> $sum() ~> $round(1)
=> 336.4
```

```jsonata-example
"  Hello  World  " ~> $trim() ~> $uppercase()
=> "HELLO WORLD"
```

With functions on both sides, `~>` composes them into a new function; see
[Function composition](programming.md#function-composition). A
[regular expression](regex.md) on the right tests a string, which is useful in filters.

## Other operators

| Operator | Meaning | See |
|---|---|---|
| `:=` | Binds a variable | [Variables](programming.md#variables) |
| `..` | Integer range, in array constructors | [Ranges](construction.md#ranges) |
| `\|location\|update, delete\|` | Transform a copy of an object | [Transforms](programming.md#transforms) |
| `.`, `[...]`, `^(...)`, `#`, `@`, `%`, `*`, `**` | Path stages | [Paths](paths.md) |

## Precedence

From the tightest binding to the loosest:

| Operators | Associativity |
|---|---|
| Function call `f(...)`, filter `[...]`, `#`, `@` | left |
| `.` | left |
| Grouping `{...}` after a path | left |
| `*`, `/`, `%` | left |
| `+`, `-`, `&` | left |
| `=`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `~>`, `^(...)`, `?:`, `??` | left |
| `and` | left |
| `or` | left |
| `? :` | right |
| `:=` | right |

Unary minus binds more tightly than any binary operator. Parentheses override precedence:

```jsonata-example
(5 + 3) * 4
=> 32
```

```jsonata-example
1 < 2 and 2 < 3 or false
=> true
```
