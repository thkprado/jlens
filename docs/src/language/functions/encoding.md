# Encoding functions

## `$base64encode`

`$base64encode(str)` encodes a string in base 64. Each character is taken as one byte, so the
string must be in the Latin-1 range (`\u0000` to `ÿ`). Encode other text with
`$encodeUrlComponent` first, if needed.

```jsonata-example
$base64encode("myuser:mypass")
=> "bXl1c2VyOm15cGFzcw=="
```

## `$base64decode`

`$base64decode(str)` decodes base 64 into a string, reading the bytes as UTF-8.

```jsonata-example
$base64decode("bXl1c2VyOm15cGFzcw==")
=> "myuser:mypass"
```

## `$encodeUrlComponent`

`$encodeUrlComponent(str)` percent-encodes the UTF-8 bytes of every character that is not
allowed in a URL component. It behaves like JavaScript's `encodeURIComponent`.

```jsonata-example
$encodeUrlComponent("?x=test")
=> "%3Fx%3Dtest"
```

## `$encodeUrl`

`$encodeUrl(str)` percent-encodes a whole URL, keeping the characters that have a meaning in
URLs (`:/?#[]@!$&'()*+,;=`). It behaves like JavaScript's `encodeURI`.

```jsonata-example
$encodeUrl("https://mozilla.org/?x=шеллы")
=> "https://mozilla.org/?x=%D1%88%D0%B5%D0%BB%D0%BB%D1%8B"
```

## `$decodeUrlComponent`

`$decodeUrlComponent(str)` reverses `$encodeUrlComponent`. Malformed escapes fail with `D3140`.

```jsonata-example
$decodeUrlComponent("%3Fx%3Dtest")
=> "?x=test"
```

## `$decodeUrl`

`$decodeUrl(str)` reverses `$encodeUrl`.

```jsonata-example
$decodeUrl("https://mozilla.org/?x=%D1%88%D0%B5%D0%BB%D0%BB%D1%8B")
=> "https://mozilla.org/?x=шеллы"
```

---

Adapted from the JSONata documentation (© IBM Corp. and JSONata contributors, MIT license).
