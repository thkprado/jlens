import threading
from concurrent.futures import ThreadPoolExecutor

from jlens import Expression

expr = Expression("$tax(price)")
expr.register_function("tax", lambda amount: amount / 4)
# ANCHOR: musl_threads
threading.stack_size(8 << 20)  # 8 MiB, the default with glibc
with ThreadPoolExecutor(4) as pool:
    taxes = list(pool.map(lambda p: expr.evaluate({"price": p}), [8, 12]))
# ANCHOR_END: musl_threads
threading.stack_size(0)
assert taxes == [2, 3]
