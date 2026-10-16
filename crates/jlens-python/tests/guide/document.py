from jlens import Expression

order = {"items": [{"price": 10, "qty": 2}, {"price": 2.5, "qty": 4}]}
# ANCHOR: document
from jlens import Document

rules = [Expression("$count(items) > 1"), Expression("$sum(items.(price * qty)) > 25")]
document = Document(order)  # or Document.from_json(text)
assert [rule.evaluate_document(document) for rule in rules] == [True, True]
# ANCHOR_END: document
