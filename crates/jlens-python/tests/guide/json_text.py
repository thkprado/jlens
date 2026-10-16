from jlens import Expression

expr = Expression("items.sku")
assert expr.evaluate_json('{"items": [{"sku": "A1"}, {"sku": "B2"}]}') == '["A1","B2"]'
assert expr.evaluate_json("{}") is None  # undefined
