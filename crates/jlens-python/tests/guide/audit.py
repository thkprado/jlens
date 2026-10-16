from jlens import Expression

expr = Expression('/** @label "Big order" */ $sum(items.(price * qty)) > $limit')
audited = expr.evaluate_with_audit(
    {"items": [{"price": 2.5, "qty": 4}, {"price": 1, "qty": 3}]},
    {"limit": 20},
    level="decisions",
    limits={"max_nodes": 1000},
)
assert audited["result"] is False
assert audited["error"] is None
assert audited["explanation_text"] == (
    "result: false\n✘ Big order — $sum(items.(price * qty)) > $limit → false (13 > 20)\n"
)
root = audited["audit"]["root"]
assert (root["label"], root["operands"], root["outcome"]) == ("Big order", [13, 20], "false")
