from jlens import Expression

expr = Expression("customer.email")
assert expr.evaluate({"customer": {}}) is None  # undefined
assert expr.evaluate({"customer": {"email": None}}) is None  # null
# To tell them apart, ask in the expression:
assert Expression("$exists(customer.email)").evaluate({"customer": {}}) is False
