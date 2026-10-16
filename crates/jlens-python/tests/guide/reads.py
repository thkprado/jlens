from jlens import Expression, union_reads

RATES = {"BR": 0.1, "PT": 0.23}

# ANCHOR: reads
expr = Expression("$sum(items[type = $kind].(price * quantity)) * $rate(country)")
reads = expr.reads({"rate": lambda country: RATES[country]})
assert reads == {
    "input": {"country": True, "items": {"price": True, "quantity": True, "type": True}},
    "bindings": {"kind": True, "rate": True},
}

union = union_reads([reads, Expression("customer.name").reads()])
assert union["input"]["customer"] == {"name": True}
# ANCHOR_END: reads
