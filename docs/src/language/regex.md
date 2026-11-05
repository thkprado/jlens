# Regular expressions

A regular expression is written between slashes, optionally followed by flags:

```text
/pattern/flags
```

| Flag | Meaning |
|---|---|
| `i` | Ignore case |
| `m` | Multi-line: `^` and `$` match at line breaks |
| `u` | Unicode mode: `\p{…}` and the other escapes of [Unicode text](#unicode-text). A jlens extension |
| `v` | Unicode mode with operations on classes and properties of strings. A jlens extension |

The flags come in any order, each at most once; `u` and `v` exclude each other, as in
JavaScript. jsonata-js knows only `i` and `m`, and rejects an expression with `u` or `v`
(`S0202`), so an expression that uses them runs in jlens alone.

The syntax and semantics are those of JavaScript regular expressions (ECMAScript). They include
character classes, quantifiers (greedy and lazy), groups, backreferences, lookahead and
lookbehind, and Unicode escapes. jlens uses the [regress](../adr/0002-regress.md) engine,
which follows the ECMAScript specification. An invalid pattern fails with `S0302`.

Like JavaScript's, the engine backtracks, so some patterns take exponential time on inputs they
almost match: `/^(a+)+$/` on thirty `a` followed by `!` tries half a billion ways to split the
run. When the evaluation has a timeout or a cancellation token, a match stops with `D1012` or
`U1003` like any other step ([ADR 0009](../adr/0009-bounded-regex.md)). Set a timeout when
the expressions come from users.

## Unicode text

A pattern matches characters, in any script. Without a flag, it can name a letter outside ASCII
only by writing it: `\w` is `[A-Za-z0-9_]`, `\d` is `[0-9]` and `\b` sits between those
characters and the others, as in JavaScript. The flags `u` and `v` add the escapes of Unicode
properties:

| Escape | Matches |
|---|---|
| `\p{L}`, `\p{Lu}`, `\p{Ll}` | A letter, an uppercase letter, a lowercase letter, of any script |
| `\p{N}`, `\p{Nd}` | A number, a decimal digit, of any script |
| `\p{M}` | A combining mark, such as an accent written apart from its letter |
| `\p{P}`, `\p{S}`, `\p{Z}` | Punctuation, a symbol, a separator |
| `\p{Script=Greek}`, `\p{sc=Cyrl}` | A character of a script (`Latin`, `Greek`, `Cyrillic`, `Han`, `Arabic`…) |
| `\p{Script_Extensions=Han}`, `\p{scx=Hira}` | A character used in a script, those shared between scripts included |
| `\P{…}` | A character without the property |
| `\u{1F600}` | A character by its code point |

```jsonata-example
differs: jsonata-js rejects the flag u (S0202)
$match("Café, Ωμέγα, Москва, 東京", /\p{L}+/u).match
=> ["Café", "Ωμέγα", "Москва", "東京"]
```

`\w`, `\d` and `\b` stay ASCII with `u` and `v`. A word in any script is `[\p{L}\p{N}_]+`:

```jsonata-example
differs: jsonata-js rejects the flag u (S0202)
{
  "ascii": $match("naïve café", /\w+/u).match,
  "words": $match("naïve café", /[\p{L}\p{N}_]+/u).match
}
=> {"ascii": ["na", "ve", "caf"], "words": ["naïve", "café"]}
```

A text can write an accented letter as one character, `é`, or as a letter followed by a
combining mark, `e` and U+0301; both look the same. `\p{L}` matches the letter, not the mark,
so a pattern for words in such text takes the marks too, `[\p{L}\p{M}]+`:

```jsonata-example
differs: jsonata-js rejects the flag u (S0202)
[$match("Café", /\p{L}+/u).match, $match("Café", /[\p{L}\p{M}]+/u).match]
=> ["Cafe", "Café"]
```

`i` ignores case in any script, with or without `u`. With `u` or `v` it follows Unicode's case
folding, so that `/k/iu` also matches the Kelvin sign `K` (U+212A), which JavaScript's default
mode keeps apart.

The flag `v` adds operations on classes: `--` takes the characters of one class out of another,
`&&` keeps those in both, and classes nest. It also adds the properties of strings, which match
a sequence of characters, such as `\p{RGI_Emoji}` for an emoji with its skin tone, a flag or a
family:

```jsonata-example
differs: jsonata-js rejects the flag v (S0202)
$match("Ωmega, Ωμέγα, Омега", /[\p{L}--\p{Script=Latin}]+/v).match
=> ["Ω", "Ωμέγα", "Омега"]
```

```jsonata-example
differs: jsonata-js rejects the flag v (S0202)
$match("ok 👍🏽 🇧🇷", /\p{RGI_Emoji}/v).[match, index]
=> [["👍🏽", 3], ["🇧🇷", 8]]
```

The positions count UTF-16 code units, as in JavaScript: `👍🏽` is two characters outside the
Basic Multilingual Plane, four units. With `v`, the characters `( ) [ ] { } / - \ |` are escaped
inside a class, and doubled punctuation such as `&&` or `!!` is reserved, so a pattern written
for `u` can be a syntax error (`S0302`) with `v`:

```jsonata-example
differs: jsonata-js rejects the flags u and v (S0202)
[$match("a-b", /[a-]+/u).match, $match("a-b", /[a\-]+/v).match]
=> ["a-", "a-"]
```

## Functions that take a regular expression

- [`$contains`](functions/string.md#contains) tests whether a string matches.
- [`$match`](functions/string.md#match) returns the matches, with positions and groups.
- [`$replace`](functions/string.md#replace) replaces matches, with references to groups.
- [`$split`](functions/string.md#split) splits a string around matches.

```jsonata-example
input: address.json
Email.address[$contains($, /@my-work\./)]
=> ["fred.smith@my-work.com", "fsmith@my-work.com"]
```

```jsonata-example
$replace("2024-03-15", /(\d+)-(\d+)-(\d+)/, "$3/$2/$1")
=> "15/03/2024"
```

## Matching in predicates

A regular expression is a function: applied to a string, it returns the first match, or
nothing. With the chain operator `~>`, that makes a compact test in a filter:

```jsonata-example
input: invoice.json
Account.Order.Product[`Product Name` ~> /hat/i].`Product Name`
=> ["Bowler Hat", "Trilby hat", "Bowler Hat"]
```

## Matcher functions

Called directly, a regular expression returns an object describing the first match:
- `match`: the matched text;
- `start` and `end`: its positions;
- `groups`: the captured texts;
- `next`: a function that returns the next match, or nothing.

```jsonata-example
(
  $matcher := /[a-z]*an[a-z]*/i;
  $m := $matcher("A man, a plan, a canal, Panama!");
  [$m.match, $m.start, $m.end, $m.next().match]
)
=> ["man", 2, 5, "plan"]
```

`$contains`, `$match`, `$replace` and `$split` only rely on this contract. Any function that
returns such objects can therefore stand in for a regular expression, for example a lambda
that implements a custom matching rule.

Positions in match objects are character offsets as JavaScript counts them, in UTF-16 code
units. A character outside the Basic Multilingual Plane counts as two.

---

Adapted from the JSONata documentation (MIT license).
