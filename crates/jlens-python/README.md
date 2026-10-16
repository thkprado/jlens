# jlens

[JSONata](https://jsonata.org/) for Python, with an engine written in Rust. It filters, queries
and transforms JSON documents, and it can **explain** how a result was reached: which
comparisons passed or failed, and which items a filter rejected.

```sh
pip install jlens
```

```python
from jlens import Expression

expr = Expression("$sum(items[type = $kind].(price * qty))")
expr.evaluate({"items": [{"type": "a", "price": 2.5, "qty": 4}]}, {"kind": "a"})  # 10
```

jlens passes the official JSONata 2.2.2 test suite, and its package is typed. Wheels cover
CPython 3.10 and later, and free-threaded Python 3.15 and later, on Linux, macOS and Windows.
The package is in beta until jlens 1.0.

## API

| Name | Purpose |
|---|---|
| `Expression(source)` | Compiles an expression; raises a `JlensError` on syntax errors |
| `expr.evaluate(input=None, bindings=None, **options)` | Evaluates against Python values, read as the expression reaches them; `None` when the result is undefined or `null` |
| `expr.evaluate_json(text=None, bindings=None, **options)` | The same with JSON text in and out |
| `Document(value)`, `Document.from_json(text)` | A document converted once, for `expr.evaluate_document(document, bindings=None, **options)`; shareable between threads |
| `expr.evaluate_with_audit(input=None, bindings=None, *, level, limits, **options)` | Result, audit tree and explanation; errors are reported, not raised |
| `expr.register_function(name, implementation, signature=None)` | Makes a Python callable available as `$name` to every evaluation |

Callables in the bindings are host functions too. The options are `timeout` (seconds),
`max_depth`, `max_string_length`, `max_array_length`, `max_result_size`, `now` and `seed`.
Errors are `JlensError` exceptions with the JSONata error `code`, plus `position`, `start`,
`end`, `token` and `value`; offsets count code points, like `str` indexes.

The full guide is the [Python chapter](https://thkprado.github.io/jlens/guides/python.html) of
the [jlens book](https://thkprado.github.io/jlens/).

## License

MIT. The code and documentation ported or adapted from JSONata are also under the MIT license;
see `LICENSE`.
