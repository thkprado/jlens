# Dates and times

JSON has no date type. Timestamps are usually strings in [ISO 8601](https://en.wikipedia.org/wiki/ISO_8601)
format (`"2018-12-10T13:45:00.000Z"`) or numbers of milliseconds since the Unix epoch. JSONata
works with both:

| Function | Purpose |
|---|---|
| [`$now()`](functions/date-time.md#now) | The evaluation time, as ISO 8601 or formatted |
| [`$millis()`](functions/date-time.md#millis) | The evaluation time, in milliseconds |
| [`$fromMillis(ms, picture?, timezone?)`](functions/date-time.md#frommillis) | Milliseconds to a formatted timestamp |
| [`$toMillis(timestamp, picture?)`](functions/date-time.md#tomillis) | A timestamp to milliseconds |

The evaluation time is captured once, when the evaluation starts. Every `$now()` and `$millis()`
in an expression returns that same value:

```jsonata-example
input: invoice.json
(
  $start := $now();
  $total := $sum(Account.Order.Product.(Price * Quantity));
  $now() = $start
)
=> true
```

Durations are differences between millisecond values:

```jsonata-example
($toMillis("2018-12-10T18:00:00Z") - $toMillis("2018-12-10T09:30:00Z")) / 3600000
=> 8.5
```

## Picture strings

Pictures describe other formats, both for formatting (`$fromMillis`, `$now`) and for parsing
(`$toMillis`). The syntax is that of XPath
[`fn:format-dateTime`](https://www.w3.org/TR/xpath-functions-31/#rules-for-datetime-formatting).
Text is copied as is, and *components* in square brackets stand for parts of the timestamp:

| Component | Meaning | Default presentation |
|---|---|---|
| `[Y]` | year | `1` |
| `[M]` | month of the year | `1` |
| `[D]` | day of the month | `1` |
| `[d]` | day of the year | `1` |
| `[F]` | day of the week | `n` (name) |
| `[W]` | ISO week of the year | `1` |
| `[w]` | week of the month | `1` |
| `[X]`, `[x]` | ISO week-numbering year, month of the week-numbering year | `1` |
| `[H]` | hour, 0–23 | `1` |
| `[h]` | hour, 1–12 | `1` |
| `[P]` | am or pm | `n` |
| `[m]` | minutes | `01` |
| `[s]` | seconds | `01` |
| `[f]` | fractions of a second | `1` |
| `[Z]` | time zone offset, `+01:00` | `01:01` |
| `[z]` | time zone as `GMT+01:00` | `01:01` |

A *presentation modifier* after the component letter chooses how it is written:

| Modifier | Output |
|---|---|
| `1`, `01`, `001` | decimal digits, padded to the width shown |
| `#1` | digits with no padding (`[h#1]`) |
| `N`, `n`, `Nn` | the name, in upper, lower or title case (months, days, am/pm) |
| `w`, `W`, `Ww` | the number in words |
| `I`, `i` | Roman numerals |
| `o` after the presentation | an ordinal: `1st`, `first` |
| `,min-max` | the minimum and maximum width (`[MNn,3-3]` gives `Jan`) |

Literal square brackets are written `[[` and `]]`.

```jsonata-example
$fromMillis(1544450400000, "[D01]/[M01]/[Y0001] [H01]:[m01]")
=> "10/12/2018 14:00"
```

```jsonata-example
$fromMillis(1544450400000, "[FNn], [D1o] [MNn] [YI]")
=> "Monday, 10th December MMXVIII"
```

```jsonata-example
$fromMillis(1544450400000, "[MNn,3-3] [D], [h]:[m01] [P]")
=> "Dec 10, 2:00 pm"
```

```jsonata-example
$fromMillis(1544450400000, "[Y]-[M01]-[D01]T[H01]:[m01]:[s01][Z]", "+0530")
=> "2018-12-10T19:30:00+05:30"
```

A timezone that is not a number is an error:

```jsonata-example
differs: jsonata-js writes "NaN" in every component
$fromMillis(1544450400000, "[Y]", "CET")
=> error D3150
```

### Parsing

`$toMillis` parses with the same pictures. Parts missing from the picture take default values:
the start of the year, the month or the day.

```jsonata-example
$toMillis("10/12/2018", "[D]/[M]/[Y]") ~> $fromMillis("[M]/[D]/[Y]")
=> "12/10/2018"
```

```jsonata-example
$toMillis("Monday, 10th December 2018", "[FNn], [D1o] [MNn] [Y]")
=> 1544400000000
```

```jsonata-example
$toMillis("2018-12-10 14:00 +01:00", "[Y]-[M]-[D] [H]:[m] [Z]") ~> $fromMillis()
=> "2018-12-10T13:00:00.000Z"
```

---

Adapted from the JSONata documentation (MIT license).
