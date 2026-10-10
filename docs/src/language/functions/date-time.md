# Date and time functions

JSON has no date type, so timestamps are strings, usually in ISO 8601 format, or numbers of
milliseconds since the Unix epoch (1970-01-01T00:00:00Z). These functions convert between the
two, with picture strings for other formats. [Dates and times](../date-time.md) describes the
picture string syntax.

## `$now`

`$now(picture?, timezone?)` returns the time the evaluation started, as an ISO 8601 string in
UTC, or formatted as `$fromMillis` would format it. Every call in one evaluation returns the
same time.

```jsonata-example
$now() = $now()
=> true
```

```jsonata-example
$now() ~> $contains(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/)
=> true
```

## `$millis`

`$millis()` returns the time the evaluation started, in milliseconds since the epoch. Like
`$now()`, it is the same throughout one evaluation.

```jsonata-example
$millis() = $toMillis($now())
=> true
```

## `$fromMillis`

`$fromMillis(millis, picture?, timezone?)` formats a number of milliseconds as a timestamp. It
uses ISO 8601 without a picture, and an XPath
[`fn:format-dateTime`](https://www.w3.org/TR/xpath-functions-31/#func-format-dateTime) picture
string otherwise. `timezone`, written `±HHMM`, shifts the time from UTC.

```jsonata-example
$fromMillis(1510067557121)
=> "2017-11-07T15:12:37.121Z"
```

```jsonata-example
$fromMillis(1510067557121, "[M01]/[D01]/[Y0001] [h#1]:[m01][P]")
=> "11/07/2017 3:12pm"
```

```jsonata-example
$fromMillis(1510067557121, "[H01]:[m01]:[s01] [z]", "-0500")
=> "10:12:37 GMT-05:00"
```

```jsonata-example
$fromMillis(1510067557121, "[FNn], [D1o] [MNn] [Y]")
=> "Tuesday, 7th November 2017"
```

## `$toMillis`

`$toMillis(timestamp, picture?)` parses a timestamp into milliseconds since the epoch. Without a
picture it expects ISO 8601. With one, it parses according to the picture. A timestamp that
does not match fails with `D3110`.

```jsonata-example
$toMillis("2017-11-07T15:07:54.972Z")
=> 1510067274972
```

```jsonata-example
$toMillis("10/12/2018", "[D]/[M]/[Y]") ~> $fromMillis("[Y]-[M01]-[D01]")
=> "2018-12-10"
```

A date and time without a time zone is read as UTC. jsonata-js reads it in the machine's local
time zone, so its results depend on where it runs; see [Compatibility](../compatibility.md).

```jsonata-example
differs: jsonata-js reads the time in the local time zone
$toMillis("2018-12-10T10:00:00")
=> 1544436000000
```

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).
