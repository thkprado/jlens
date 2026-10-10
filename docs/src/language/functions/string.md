# String functions

Most string functions use the context value when their first argument is omitted, so they
work as path steps: `Account.Order.OrderID.$uppercase()`. Positions and lengths count
characters (Unicode code points), except where noted.

## `$string`

`$string(arg, prettify?)` converts a value to a string:
- strings are returned unchanged;
- functions become the empty string;
- other values are written as JSON.

With `prettify` true, objects and arrays are indented. A number that is not finite is an error
(`D3001`).

```jsonata-example
$string(5)
=> "5"
```

```jsonata-example
[1..3].$string()
=> ["1", "2", "3"]
```

```jsonata-example
$string({"a": [1, true, null]})
=> "{\"a\":[1,true,null]}"
```

Numbers are written with at most 15 significant digits, as in jsonata-js:

```jsonata-example
$string(0.1 + 0.2)
=> "0.3"
```

## `$length`

`$length(str)` returns the number of characters in `str`.

```jsonata-example
$length("Hello World")
=> 11
```

```jsonata-example
$length("𝄞é")
=> 2
```

## `$substring`

`$substring(str, start, length?)` returns the characters of `str` from position `start`
(from 0), at most `length` of them. A negative `start` counts from the end.

```jsonata-example
$substring("Hello World", 3)
=> "lo World"
```

```jsonata-example
$substring("Hello World", 3, 5)
=> "lo Wo"
```

```jsonata-example
$substring("Hello World", -4, 2)
=> "or"
```

## `$substringBefore`

`$substringBefore(str, chars)` returns the part of `str` before the first occurrence of
`chars`, or all of `str` if `chars` does not occur.

```jsonata-example
$substringBefore("Hello World", " ")
=> "Hello"
```

## `$substringAfter`

`$substringAfter(str, chars)` returns the part of `str` after the first occurrence of `chars`,
or all of `str` if `chars` does not occur.

```jsonata-example
$substringAfter("Hello World", " ")
=> "World"
```

## `$uppercase`

`$uppercase(str)` converts `str` to upper case.

```jsonata-example
$uppercase("Hello World")
=> "HELLO WORLD"
```

## `$lowercase`

`$lowercase(str)` converts `str` to lower case.

```jsonata-example
$lowercase("Hello World")
=> "hello world"
```

## `$trim`

`$trim(str)` normalizes whitespace in three steps:
1. Tabs, carriage returns and line feeds become spaces.
2. Runs of spaces become one space.
3. Leading and trailing spaces are removed.

```jsonata-example
$trim("   Hello  \n World  ")
=> "Hello World"
```

## `$pad`

`$pad(str, width, char?)` pads `str` to at least `|width|` characters with `char` (a space by
default). A positive `width` pads on the right, a negative one on the left.

```jsonata-example
$pad("foo", 5)
=> "foo  "
```

```jsonata-example
$pad("foo", -5, "#")
=> "##foo"
```

```jsonata-example
$formatBase(35, 2) ~> $pad(-8, "0")
=> "00100011"
```

## `$contains`

`$contains(str, pattern)` tests whether `str` contains `pattern`, which is either a string or a
[regular expression](../regex.md).

```jsonata-example
$contains("abracadabra", "bra")
=> true
```

```jsonata-example
$contains("Hello World", /wo/i)
=> true
```

```jsonata-example
input: address.json
Phone[$contains(number, /^077/)].type
=> "mobile"
```

## `$split`

`$split(str, separator, limit?)` splits `str` into an array of strings around `separator`,
which is a string or a regular expression. An empty string separator splits `str` into
characters. `limit` caps the number of parts.

```jsonata-example
$split("so many words", " ")
=> ["so", "many", "words"]
```

```jsonata-example
$split("so many words", " ", 2)
=> ["so", "many"]
```

```jsonata-example
$split("too much, punctuation. hard; to read", /[ ,.;]+/)
=> ["too", "much", "punctuation", "hard", "to", "read"]
```

## `$join`

`$join(strings, separator?)` joins an array of strings, with `separator` (empty by default)
between them. Every item must be a string.

```jsonata-example
$join(["a", "b", "c"])
=> "abc"
```

```jsonata-example
$split("too much, punctuation. hard; to read", /[ ,.;]+/, 3) ~> $join(", ")
=> "too, much, punctuation"
```

## `$match`

`$match(str, pattern, limit?)` returns the matches of a regular expression in `str`. Each
match is an object with three fields:
- `match`: the matched text;
- `index`: its position;
- `groups`: the texts captured by the parenthesized groups.

`limit` caps the number of matches. A single match is returned as a single object, following
the [sequence](../overview.md#sequences) rules.

```jsonata-example
$match("ababbabbcc", /a(b+)/)
=> [
  {"match": "ab", "index": 0, "groups": ["b"]},
  {"match": "abb", "index": 2, "groups": ["bb"]},
  {"match": "abb", "index": 5, "groups": ["bb"]}
]
```

```jsonata-example
$match("ababbabbcc", /a(b+)/, 1).groups
=> ["b"]
```

## `$replace`

`$replace(str, pattern, replacement, limit?)` replaces the occurrences of `pattern` (a string or
a regular expression) in `str`. `limit` caps the number of replacements.

A string `replacement` with a regex `pattern` can refer to what was matched:
- `$0` is the whole match;
- `$1`, `$2`, … are the captured groups (empty when there is no such group);
- `$$` is a literal `$`.

`replacement` can also be a function. It receives each match, as `$match` describes it, and
returns the replacement string.

```jsonata-example
$replace("John Smith and John Jones", "John", "Mr")
=> "Mr Smith and Mr Jones"
```

```jsonata-example
$replace("John Smith and John Jones", "John", "Mr", 1)
=> "Mr Smith and John Jones"
```

```jsonata-example
$replace("John Smith", /(\w+)\s(\w+)/, "$2, $1")
=> "Smith, John"
```

```jsonata-example
$replace("265USD", /([0-9]+)USD/, "$$$1")
=> "$265"
```

```jsonata-example
(
  $celsius := function($m) { ($number($m.groups[0]) - 32) * 5 / 9 & "C" };
  $replace("temperature = 68F today", /(\d+)F/, $celsius)
)
=> "temperature = 20C today"
```

---

Adapted from the JSONata documentation (MIT license).
The functions that format numbers as strings are on the [numeric functions](numeric.md) page.
