# Numeric functions

Numbers are 64-bit floats. A function whose result would not be a finite number fails with
`D3060` (`$sqrt`) or `D3061` (`$power`).

## `$number`

`$number(arg)` converts a value to a number:

| Argument | Result |
|---|---|
| a number | unchanged |
| a string in JSON number syntax | that number |
| a string starting with `0x`, `0o` or `0b` | a hexadecimal, octal or binary integer |
| `true`, `false` | `1`, `0` |
| anything else | error `D3030` |

```jsonata-example
$number("5")
=> 5
```

```jsonata-example
$number("0x12")
=> 18
```

```jsonata-example
["1", "2.5", "1e3"].$number()
=> [1, 2.5, 1000]
```

```jsonata-example
$number("five")
=> error D3030
```

## `$abs`

`$abs(n)` returns the absolute value of `n`.

```jsonata-example
$abs(-5)
=> 5
```

## `$floor`

`$floor(n)` rounds `n` down to an integer.

```jsonata-example
[$floor(5.8), $floor(-5.3)]
=> [5, -6]
```

## `$ceil`

`$ceil(n)` rounds `n` up to an integer.

```jsonata-example
[$ceil(5.3), $ceil(-5.3)]
=> [6, -5]
```

## `$round`

`$round(n, precision?)` rounds `n` to `precision` decimal places (0 by default). A negative
`precision` rounds to tens, hundreds, and so on. Halves round to the even neighbour (banker's
rounding), as in IEEE 754 and financial calculations.

```jsonata-example
[$round(123.456), $round(123.456, 2), $round(123.456, -1)]
=> [123, 123.46, 120]
```

```jsonata-example
[$round(11.5), $round(12.5), $round(125, -1)]
=> [12, 12, 120]
```

## `$power`

`$power(base, exponent)` raises `base` to `exponent`.

```jsonata-example
[$power(2, 8), $power(2, -2), $power(2, 0.5)]
=> [256, 0.25, 1.4142135623730951]
```

```jsonata-example
$power(-1, 0.5)
=> error D3061
```

## `$sqrt`

`$sqrt(n)` returns the square root of `n`, which must not be negative.

```jsonata-example
$sqrt(16)
=> 4
```

## `$random`

`$random()` returns a pseudo-random number in [0, 1). It is not suitable for cryptography.

```jsonata-example
$random() < 1
=> true
```

## `$formatNumber`

`$formatNumber(n, picture, options?)` formats a number with an XPath
[`fn:format-number`](https://www.w3.org/TR/xpath-functions-31/#func-format-number) picture
string:

| Picture character | Meaning |
|---|---|
| `0` | a mandatory digit |
| `#` | an optional digit |
| `.` | the decimal separator |
| `,` | the grouping separator |
| `%`, `‰` | multiply by 100 or 1,000 and show the sign |
| `e` | the exponent separator (scientific notation) |
| `;` | separates the picture for positive numbers from the one for negative numbers |

`options` replaces the characters used, for example `{"decimal-separator": ",",
"grouping-separator": "."}`. The other options are `zero-digit`, `percent`, `per-mille`,
`minus-sign`, `infinity`, `NaN`, `exponent-separator` and `pattern-separator`.

```jsonata-example
$formatNumber(12345.6, "#,###.00")
=> "12,345.60"
```

```jsonata-example
$formatNumber(1234.5678, "00.000e0")
=> "12.346e2"
```

```jsonata-example
[$formatNumber(34.555, "#0.00;(#0.00)"), $formatNumber(-34.555, "#0.00;(#0.00)")]
=> ["34.56", "(34.56)"]
```

```jsonata-example
$formatNumber(0.14, "01%")
=> "14%"
```

```jsonata-example
$formatNumber(1234567.891, "#.##0,00", {"decimal-separator": ",", "grouping-separator": "."})
=> "1.234.567,89"
```

## `$formatBase`

`$formatBase(n, radix?)` writes the integer part of `n` in base `radix`, from 2 to 36 (10 by
default).

```jsonata-example
[$formatBase(100, 2), $formatBase(2555, 16)]
=> ["1100100", "9fb"]
```

## `$formatInteger`

`$formatInteger(n, picture)` writes an integer with an XPath
[`fn:format-integer`](https://www.w3.org/TR/xpath-functions-31/#func-format-integer) picture:

| Picture | Output |
|---|---|
| `0`, `001`, `#,##0` | decimal digits, with a minimum width and grouping separators |
| `w`, `W`, `Ww` | words, in lower, upper or title case |
| `i`, `I` | Roman numerals |
| `a`, `A` | letters: `a`…`z`, `aa`… |
| suffix `;o` | ordinal: `1st`, `first` |

```jsonata-example
$formatInteger(2789, "w")
=> "two thousand, seven hundred and eighty-nine"
```

```jsonata-example
[$formatInteger(1999, "I"), $formatInteger(28, "A"), $formatInteger(3, "w;o")]
=> ["MCMXCIX", "AB", "third"]
```

```jsonata-example
$formatInteger(1234567, "#,##0")
=> "1,234,567"
```

## `$parseInteger`

`$parseInteger(str, picture)` reads an integer written as `$formatInteger` writes it.

```jsonata-example
$parseInteger("twelve thousand, four hundred and seventy-six", "w")
=> 12476
```

```jsonata-example
$parseInteger("12,345,678", "#,##0")
=> 12345678
```

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).
