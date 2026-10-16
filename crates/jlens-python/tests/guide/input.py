# ANCHOR: input
from jlens import Expression, JlensError

order = {"customer": {"name": "Acme"}, "log": {"seen": {1, 2}}}  # a set is not JSON
assert Expression("customer.name").evaluate(order) == "Acme"  # `log` is never read
try:
    Expression("log.seen").evaluate(order)
except JlensError as error:
    assert error.code == "U1004"
# ANCHOR_END: input
else:
    raise AssertionError("no error")
