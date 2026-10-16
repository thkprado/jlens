from jlens import Expression

expr = Expression("$sum(items[type = $kind].(price * qty))")
order = {
    "items": [
        {"type": "book", "price": 12.5, "qty": 2},
        {"type": "pen", "price": 1.5, "qty": 10},
    ]
}
assert expr.evaluate(order, {"kind": "book"}) == 25
