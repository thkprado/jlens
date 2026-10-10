# Programming

JSONata is a functional language. Values are immutable, functions are values, and everything
is an expression.

## Blocks

Parentheses group expressions separated by `;`. The expressions are evaluated in order, and
the block's value is the value of the last one. A block is also a scope for variables.

```jsonata-example
(1 + 2; "ignored"; 10 * 2)
=> 20
```

## Variables

A name that starts with `$` is a variable. `$name := value` binds a variable, and its value is
the bound value. The binding is visible in the rest of the block and in nested blocks, but not
outside the block.

```jsonata-example
input: invoice.json
Account.Order.Product.(
  $total := Price * Quantity;
  $tax := $total * 0.2;
  $round($total + $tax, 2)
)
=> [82.68, 26, 165.36, 129.59]
```

```jsonata-example
($x := 1; ($x := 2); $x)
=> 1
```

Predefined variables:
- `$` is the context value; see [Paths](paths.md#the-context--and-).
- `$$` is the root of the input.
- The built-in functions (`$sum`, `$string`, …) are variables too, so a binding can replace one
  in its scope.

Applications can define more variables when they evaluate an expression: *bindings*, such as
`jlens eval --bindings`, `Bindings` in Rust, or the `bindings` argument in JavaScript.

```jsonata-example
bindings: {"limit": 100, "rate": 0.2}
$limit * (1 + $rate)
=> 120
```

## Functions

### Calling functions

A function is called with its arguments in parentheses:

```jsonata-example
$substring("hello world", 0, 5)
=> "hello"
```

Many built-in functions use the context value when their first argument is omitted (their
signature marks that argument with `-`). That makes them usable as path steps:

```jsonata-example
input: invoice.json
Account.Order.OrderID.$uppercase()
=> ["ORDER103", "ORDER104"]
```

### Defining functions

`function($a, $b) { body }` defines a function. `λ` can be written instead of `function`.
Functions are values: bind them to variables, pass them to other functions, return them.

```jsonata-example
(
  $volume := function($l, $w, $h) { $l * $w * $h };
  $volume(10, 10, 5)
)
=> 500
```

```jsonata-example
λ($x) { $x * 2 }(21)
=> 42
```

Missing arguments are undefined inside the function, and extra arguments are ignored.

### Signatures

An optional signature after the parameter list declares the types of the arguments and of the
result. Arguments are then checked when the function is called (errors `T0410` and `T0412`).

```jsonata-example
(
  $repeat := function($s, $n)<sn:s> { $join([1..$n].($s)) };
  $repeat("ab", 3)
)
=> "ababab"
```

```jsonata-example
(
  $repeat := function($s, $n)<sn:s> { $join([1..$n].($s)) };
  $repeat(3, "ab")
)
=> error T0410
```

A signature is `<params:result>`, with one symbol per parameter:

| Symbol | Type |
|---|---|
| `b` | Boolean |
| `n` | number |
| `s` | string |
| `l` | `null` |
| `a` | array; `a<n>` is an array of numbers |
| `o` | object |
| `f` | function |
| `(sn)` | a choice: string or number |
| `u` | `(bnsl)`: any simple value |
| `j` | `(bnsloa)`: any JSON value |
| `x` | any value, functions included |

These modifiers can follow a symbol:
- `?` makes the parameter optional.
- `+` takes one or more arguments of the type.
- `-` takes the context value when the argument is missing.

A single value is accepted where an array is expected. For example, `$sum` has the signature
`<a<n>:n>` and `$length` has `<s-:n>`. The [function reference](functions.md) gives the
signature of every built-in function.

### Recursion and tail calls

A function bound to a variable can call itself through it:

```jsonata-example
(
  $factorial := function($n) { $n <= 1 ? 1 : $n * $factorial($n - 1) };
  $factorial(10)
)
=> 3628800
```

Each nested call uses stack, and evaluation stops with `D1011` past a depth limit (see
[Guardrails](../architecture/evaluator.md#guardrails)). A call in *tail position*, the last
thing a function does, does not nest. jlens runs it as a loop, so tail-recursive functions can
iterate any number of times:

```jsonata-example
(
  $sumTo := function($n, $acc) { $n = 0 ? $acc : $sumTo($n - 1, $acc + $n) };
  $sumTo(100000, 0)
)
=> 5000050000
```

### Closures

A function remembers the variables and the context value of the place where it was defined:

```jsonata-example
(
  $multiplier := function($k) { function($x) { $x * $k } };
  $triple := $multiplier(3);
  [$triple(2), $triple(5)]
)
=> [6, 15]
```

```jsonata-example
input: invoice.json
Account.(
  $account := function() { $.`Account Name` };
  Order[OrderID = 'order104'].Product.{"account": $account(), "product": `Product Name`}
)
=> [
  {"account": "Firefly", "product": "Bowler Hat"},
  {"account": "Firefly", "product": "Cloak"}
]
```

### Higher-order functions

Functions that take or return functions are called higher-order. The library has several:
[`$map`](functions/higher-order.md#map), [`$filter`](functions/higher-order.md#filter),
[`$reduce`](functions/higher-order.md#reduce), [`$sift`](functions/higher-order.md#sift),
[`$single`](functions/higher-order.md#single) and [`$sort`](functions/array.md#sort).

```jsonata-example
(
  $twice := function($f) { function($x) { $f($f($x)) } };
  $twice(function($y) { $y + 3 })(7)
)
=> 13
```

### Partial application

Calling a function with `?` in place of some arguments returns a new function that takes the
missing ones:

```jsonata-example
(
  $first5 := $substring(?, 0, 5);
  $first5("Hello, World")
)
=> "Hello"
```

```jsonata-example
(
  $firstN := $substring(?, 0, ?);
  $firstN("Hello, World", 3)
)
=> "Hel"
```

### Function composition

When both sides of `~>` are functions, the result is their composition: a function that applies
the left one, then the right one.

```jsonata-example
(
  $normalize := $trim ~> $uppercase;
  $normalize("   Some   Words   ")
)
=> "SOME WORDS"
```

```jsonata-example
input: address.json
(
  $first5Capitalized := $substring(?, 0, 5) ~> $uppercase(?);
  $first5Capitalized(Address.City)
)
=> "WINCH"
```

## Conditional logic

The [conditional operator](operators.md#conditionals) `? :` and the
[default operators](operators.md#defaults) `?:` and `??` cover if/then/else. Nested
conditionals read like an `else if` chain:

```jsonata-example
input: invoice.json
Account.Order.Product.(
  Price > 100 ? "premium" :
  Price > 30 ? "standard" :
  "budget"
)
=> ["standard", "budget", "standard", "premium"]
```

## Transforms

The transform operator `|location|update|` or `|location|update, delete|` produces a function.
That function copies its argument and, in the copy, changes every object that `location`
matches:

- `update` must evaluate to an object. Its fields are merged into each matched object, as
  [`$merge`](functions/object.md#merge) does. `update` is evaluated against the matched object.
- `delete` (optional) names the fields to remove: a string or an array of strings.

The original value is not modified.

```jsonata-example
input: invoice.json
(
  $copy := $ ~> |Account.Order.Product|{"Price": $round(Price * 1.2, 2)}, ["SKU", "Description"]|;
  $copy.Account.Order[0].Product
)
=> [
  {"Product Name": "Bowler Hat", "ProductID": 858383, "Price": 41.34, "Quantity": 2},
  {"Product Name": "Trilby hat", "ProductID": 858236, "Price": 26, "Quantity": 1}
]
```

Updates see the original values. Here `Total` uses the price before the 20% increase:

```jsonata-example
{"Price": 10, "Quantity": 3} ~> |$|{"Price": Price * 1.2, "Total": Price * Quantity}|
=> {"Price": 12, "Quantity": 3, "Total": 30}
```

A transform is a function, so it can be bound to a variable and reused, or chained with other
transforms. An update that would make an object contain itself is an error (`U1001`) in jlens;
see [Compatibility](compatibility.md).

## Comments and annotations

`/* ... */` comments are ignored. jlens reads documentation comments (`/** ... */`) placed before
a subexpression as [audit annotations](annotations.md): labels and messages that explain the
checks of a rule.
