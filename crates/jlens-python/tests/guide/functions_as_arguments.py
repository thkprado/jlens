from jlens import Expression

stock = {("A1", 1): None, ("A1", 2): 7}


def lookup(sku, attempt):
    if stock[(sku, attempt)] is None:
        raise TimeoutError
    return stock[(sku, attempt)]


# ANCHOR: functions_as_arguments
def retry(step, times):
    for attempt in range(1, times + 1):
        try:
            return step(attempt)
        except Exception:
            if attempt == times:
                raise


expr = Expression("$retry(function($i) { $lookup(sku, $i) }, 3)")
assert expr.evaluate({"sku": "A1"}, {"retry": retry, "lookup": lookup}) == 7
# ANCHOR_END: functions_as_arguments
