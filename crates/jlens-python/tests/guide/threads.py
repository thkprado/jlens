from concurrent.futures import ThreadPoolExecutor

from jlens import Document, Expression

expr = Expression("$sum(items)")
documents = [Document.from_json(f'{{"items": [{i}, {i}]}}') for i in range(100)]
with ThreadPoolExecutor(8) as pool:
    totals = list(pool.map(expr.evaluate_document, documents))
assert totals[:3] == [0, 2, 4]
