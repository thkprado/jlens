# ANCHOR: host_exceptions
from jlens import Expression


class RateError(Exception):
    code = "RATE01"  # how evaluate_with_audit reports it


def rate(region):
    raise RateError(f"no rate for {region}")


expr = Expression("$rate(region) * 2")
try:
    expr.evaluate({"region": "west"}, {"rate": rate})
except RateError as error:
    assert (error.position, error.token) == (6, "rate")  # added by jlens
# ANCHOR_END: host_exceptions
else:
    raise AssertionError("no error")
