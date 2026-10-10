# Regular expressions

A regular expression is written between slashes, optionally followed by flags:

```text
/pattern/flags
```

| Flag | Meaning |
|---|---|
| `i` | Ignore case |
| `m` | Multi-line: `^` and `$` match at line breaks |

The syntax and semantics are those of JavaScript regular expressions (ECMAScript). They include
character classes, quantifiers (greedy and lazy), groups, backreferences, lookahead and
lookbehind, and Unicode escapes. jlens uses the [regress](../adr/0002-regress.md) engine,
which follows the ECMAScript specification. An invalid pattern fails with `S0302`.

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
