# ANCHOR: options
from jlens import Expression, JlensError

clock = Expression("[$now(), $random()]")
first = clock.evaluate(now="2024-01-02T03:04:05Z", seed=7)
assert first[0] == "2024-01-02T03:04:05.000Z"
assert clock.evaluate(now="2024-01-02T03:04:05Z", seed=7) == first

loop = Expression("($f := function($n) { $f($n + 1) }; $f(0))")
try:
    loop.evaluate(timeout=0.05)  # seconds
except JlensError as error:
    assert error.code == "D1012"

nested = Expression("$sum($map([1..1000], function($i) { $i }))")
try:
    nested.evaluate(max_steps=1000)  # the same on every machine
except JlensError as error:
    assert (error.code, error.value) == ("U1006", 1000)
# ANCHOR_END: options

for expr, options in ((loop, {"timeout": 0.05}), (nested, {"max_steps": 1000})):
    try:
        expr.evaluate(**options)
    except JlensError:
        pass
    else:
        raise AssertionError("no error")
