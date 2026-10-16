from jlens import Expression

rates = {"north": 1.5, "south": 2}
order = {"region": "south", "items": [{"weight": 2}, {"weight": 3}]}


# ANCHOR: host_functions
def shipping(weight, region):
    return weight * rates[region]


expr = Expression("$shipping($sum(items.weight), region)")
assert expr.evaluate(order, {"shipping": shipping}) == 10

# As in jsonata-js: available to every evaluation, with an optional signature.
expr.register_function("shipping", shipping, "<ns:n>")
assert expr.evaluate(order) == 10
# ANCHOR_END: host_functions
