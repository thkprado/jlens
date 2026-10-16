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
# ANCHOR_END: options
else:
    raise AssertionError("no error")
