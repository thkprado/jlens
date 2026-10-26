"""The Python package: the cases of the WebAssembly suite (crates/jlens-wasm/tests/suite.mjs)
that apply to it, and the conversions of Python values."""

import collections
import copy
import datetime
import enum
import gc
import json
import pathlib
import platform
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

import pytest

import jlens
from jlens import Document, Expression, JlensError

ORDER = {
    "customer": {"name": "Acme Stores"},
    "items": [
        {"sku": "A1", "price": 10, "quantity": 2},
        {"sku": "B2", "price": 2.5, "quantity": 4},
    ],
}


def raises(code, call, *args, **kwargs):
    with pytest.raises(JlensError) as info:
        call(*args, **kwargs)
    assert info.value.code == code
    return info.value


def deep_list(levels):
    value = 1
    for _ in range(levels):
        value = [value]
    return value


def test_version():
    assert isinstance(jlens.__version__, str)
    assert jlens.__version__.count(".") == 2


def test_evaluate():
    expr = Expression("$sum(items.(price * quantity))")
    assert expr.evaluate(ORDER) == 30
    assert expr.source == "$sum(items.(price * quantity))"
    assert repr(expr) == "Expression('$sum(items.(price * quantity))')"
    assert Expression("items[price > 5].sku").evaluate(ORDER) == "A1"
    assert Expression("{'n': $count(items)}").evaluate(ORDER) == {"n": 2}


def test_undefined_and_null():
    assert Expression("nothing").evaluate(ORDER) is None
    assert Expression("null").evaluate() is None
    assert Expression("nothing").evaluate_json(json.dumps(ORDER)) is None
    assert Expression("null").evaluate_json() == "null"
    # No input: `$` is undefined.
    assert Expression("$exists($)").evaluate() is False
    assert Expression("$exists($)").evaluate(None) is False


def test_input_values():
    everything = Expression("$")
    value = {"s": "é𝄞", "i": 7, "f": 1.5, "t": True, "n": None, "l": [1, [2]], "d": {"k": {}}}
    assert everything.evaluate(value) == value
    assert everything.evaluate((1, (2, 3))) == [1, [2, 3]]
    assert everything.evaluate(collections.OrderedDict(b=1, a=2)) == {"b": 1, "a": 2}
    assert list(everything.evaluate({"b": 1, "a": 2})) == ["b", "a"]
    # Not finite numbers become null, as in JSON.
    assert everything.evaluate([float("nan"), float("inf")]) == [None, None]
    # An int beyond 64 bits becomes the nearest float.
    assert Expression("$ + 0").evaluate(2**70) == float(2**70)
    # A lone surrogate, which UTF-8 cannot hold, becomes U+FFFD.
    assert everything.evaluate("a\ud800") == "a�"
    # Subclasses of str, int and float are read as their base type.
    class Text(str):
        pass

    class Level(enum.IntEnum):
        HIGH = 3

    assert everything.evaluate([Text("x"), Level.HIGH, True]) == ["x", 3, True]


def test_result_numbers():
    everything = Expression("$")
    assert type(Expression("2.0").evaluate()) is int
    assert type(Expression("1.5").evaluate()) is float
    assert type(Expression("1e20").evaluate()) is int
    assert type(Expression("1e21").evaluate()) is float
    assert everything.evaluate(2**53) == 2**53
    assert type(Expression("-0").evaluate()) is int
    # Integers beyond 64 bits.
    assert Expression("[9e18, 1e19, -1e19]").evaluate() == [9 * 10**18, 10**19, -(10**19)]


def test_results_share_their_keys():
    expression = Expression("[1..3].{'id': $, 'name': 'n' & $}")
    document = Document.from_json("null")
    for result in (expression.evaluate(), expression.evaluate_document(document)):
        assert result == [{"id": n, "name": f"n{n}"} for n in (1, 2, 3)]
        first, last = (list(item) for item in (result[0], result[2]))
        assert all(a is b for a, b in zip(first, last))


def test_unsupported_values_fail_where_they_are_read():
    error = raises("U1004", Expression("a").evaluate, {"a": {1, 2}})
    assert "Object of type set is not JSON serializable" in str(error)
    error = raises("U1004", Expression("a").evaluate, {"a": {1: 2}})
    assert "keys must be str" in str(error)
    raises("U1004", Expression("$").evaluate, datetime.date(2024, 1, 2))
    # A level the expression does not reach is not read.
    assert Expression("b").evaluate({"a": {"set": {1, 2}}, "b": 3}) == 3
    # A path step reads its field alone: the other fields of the dict are read when it is
    # needed whole.
    assert Expression("b").evaluate({"a": {1, 2}, "b": 3, 4: 5}) == 3
    raises("U1004", Expression("$keys($)").evaluate, {"a": {1, 2}, "b": 3})
    raises("U1004", Expression("[b, $]").evaluate, {"b": 3, 4: 5})


def test_fields_read_alone_then_whole():
    order = {"o": {"p": {"x": 1}, "q": 2, "r": [3]}}
    keys = Expression("($p := o.p; [$join($keys(o)), o.p = $p, $count(o.r)])")
    assert keys.evaluate(order) == ["pqr", True, 1]
    assert Expression("[o.q, o, o.missing, o.q]").evaluate(order) == [2, order["o"], 2]


def test_items_read_alone_then_whole():
    # An index reads the items it picks: the item that cannot be read is not reached.
    for items in ([{"code": 1}, {1, 2}, 3], ({"code": 1}, {1, 2}, 3)):
        order = {"items": items}
        assert Expression("items[0].code").evaluate(order) == 1
        assert Expression("($i := -1; items[$i])").evaluate(order) == 3
        assert Expression("[items[5], items[-5], $count(items)]").evaluate(order) == [3]
        raises("U1004", Expression("items[1]").evaluate, order)
        raises("U1004", Expression("$sum(items.code)").evaluate, order)
    # An item read alone is the value the array holds when it is read whole.
    order = {"items": [{"code": 1}, [2], 3]}
    same = Expression("($a := items[0]; $b := items[1]; {'a': $a = items[0], 'all': items, 'b': $b})")
    assert same.evaluate(order) == {"a": True, "all": order["items"], "b": [2]}
    # A cycle through an item read alone.
    looped = []
    looped.append(looped)
    raises("U1004", Expression("$[0]").evaluate, looped)
    # A list made shorter after its length was read: the item past its end is undefined.
    items = [1, 2]
    shorten = Expression("($n := $count(items); $pop(); [$n, items[1]])")
    shorten.register_function("pop", lambda: items.pop())
    assert shorten.evaluate({"items": items}) == [2]


def test_fields_found_whatever_str_their_keys_are():
    class Key(str):
        pass

    name = "".join(["na", "me"])  # not interned
    inputs = [{"name": 1}, json.loads('{"name": 1}'), {name: 1}, {Key("name"): 1}]
    # The first expression names its keys; the second reads its input whole, so it names none.
    for expression in (Expression("name"), Expression("{'n': name, 'k': $keys($)}.n")):
        assert [expression.evaluate(i) for i in inputs] == [1, 1, 1, 1]
        assert expression.evaluate({"other": 1}) is None


def test_cycles_fail_cleanly():
    cyclic = {"a": 1}
    cyclic["self"] = cyclic
    error = raises("U1004", Expression("$").evaluate, cyclic)
    assert "Circular reference detected" in str(error)
    looped = []
    looped.append(looped)
    raises("U1004", Expression("$count(**)").evaluate, looped)
    # A cycle through several levels.
    outer = {"n": 1}
    outer["child"] = {"items": [{"parent": outer}]}
    raises("U1004", Expression("$string($)").evaluate, outer)
    raises("U1004", Expression("child.items.parent").evaluate, outer)
    assert Expression("$count(child.items)").evaluate(outer) == 1
    # A cycle is found when the level that holds it is read: below the levels read, it is not.
    assert Expression("a").evaluate({"a": 1, "b": cyclic}) == 1
    # The same value twice is not a cycle.
    shared = [1, 2]
    assert Expression("$").evaluate({"x": shared, "y": shared}) == {"x": shared, "y": shared}


def test_deep_values_fail_cleanly():
    deep = deep_list(100_000)
    for source in ["$", "$string($)", "$count(**)"]:
        raises("U1001", Expression(source).evaluate, deep)
    assert Expression("$count($)").evaluate(deep) == 1
    with pytest.raises(ValueError):
        Document(deep)
    with pytest.raises(ValueError):
        Expression("$").evaluate(None, {"deep": deep})
    assert Expression("$").evaluate(deep_list(900)) == deep_list(900)


def test_input_is_not_modified():
    value = {"a": [1, 2], "b": {"c": 3}}
    before = copy.deepcopy(value)
    Expression("$ ~> |$|{'a': 'changed', 'z': 1}|").evaluate(value)
    Expression("$ ~> |b|{'c': 4}|").evaluate(value)
    assert value == before


def test_evaluate_json():
    expr = Expression("items.sku")
    assert expr.evaluate_json(json.dumps(ORDER)) == '["A1","B2"]'
    assert Expression("$x").evaluate_json(None, {"x": [1]}) == "[1]"
    with pytest.raises(ValueError):
        expr.evaluate_json("{")


def test_bindings():
    expr = Expression("$limit + $count(items)")
    assert expr.evaluate(ORDER, {"limit": 10}) == 12
    assert expr.evaluate(ORDER, bindings={"limit": 1}) == 3
    assert Expression("$x").evaluate(None, {"x": None}) is None
    assert Expression("$x.y").evaluate(None, {"x": {"y": (1, 2)}}) == [1, 2]
    with pytest.raises(TypeError):
        expr.evaluate(ORDER, [("limit", 1)])
    with pytest.raises(TypeError):
        expr.evaluate(ORDER, {"limit": {1, 2}})
    with pytest.raises(TypeError):
        expr.evaluate(ORDER, {1: 2})
    cyclic = []
    cyclic.append(cyclic)
    with pytest.raises(ValueError, match="Circular reference detected"):
        expr.evaluate(ORDER, {"limit": cyclic})


def test_syntax_errors():
    error = raises("S0203", Expression, "items[")
    assert isinstance(error, Exception)
    assert error.position == 6
    error = raises("S0101", Expression, '"abc')
    assert error.value is None


def test_evaluation_errors():
    error = raises("T2001", Expression("customer.name + 1").evaluate, ORDER)
    assert error.value == "Acme Stores"
    assert error.token == "+"
    assert str(error) == 'The left side of the "+" operator must evaluate to a number'


def test_offsets_count_code_points():
    # 𝄞 is one code point and two UTF-16 code units: the offsets equal those of the ASCII
    # expression with one character in its place.
    for source, ascii in [('"𝄞b" + 1', '"ab" + 1'), ("'é𝄞' & $x(", "'ab' & $x(")]:
        try:
            Expression(source).evaluate()
        except JlensError as error:
            got = (error.position, error.start, error.end)
        try:
            Expression(ascii).evaluate()
        except JlensError as error:
            want = (error.position, error.start, error.end)
        assert got == want
    error = raises("T2001", Expression('"𝄞b" + 1').evaluate)
    source = '"𝄞b" + 1'
    assert source[error.start : error.end] == source
    assert source[error.position - 1] == "+"


def test_guardrails():
    loop = Expression("($f := function($n){ $f($n + 1) }; $f(0))")
    raises("D1012", loop.evaluate, timeout=0.02)
    backtrack = Expression("$contains($, /^(a+)+$/)")
    started = time.monotonic()
    raises("D1012", backtrack.evaluate, "a" * 30 + "!", timeout=0.05)
    assert time.monotonic() - started < 2
    deep = Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(100))")
    raises("D1011", deep.evaluate, max_depth=100)
    assert deep.evaluate() == 100
    raises("D1011", Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(1000000))").evaluate)


def test_size_limits():
    long = Expression("$pad('', 100, 'x')")
    raises("U1001", long.evaluate, max_string_length=50)
    assert len(long.evaluate(max_string_length=100)) == 100
    raises("U1001", Expression("[1..100]").evaluate, max_array_length=50)
    shared = Expression("($x := {'s': $pad('', 40, 'x')}; {'l': $x, 'r': $x})")
    raises("U1001", shared.evaluate, max_result_size=80)
    raises("U1001", shared.evaluate_json, max_result_size=80)
    assert len(shared.evaluate(max_result_size=200)["r"]["s"]) == 40


def test_clock_and_seed():
    clock = Expression("$now()")
    when = datetime.datetime(2024, 1, 2, 3, 4, 5, tzinfo=datetime.timezone.utc)
    iso = "2024-01-02T03:04:05.000Z"
    millis = int(when.timestamp() * 1000)
    assert clock.evaluate(now=millis) == iso
    assert clock.evaluate(now=when) == iso
    assert clock.evaluate(now=iso) == iso
    random = Expression("[1..5].$random()")
    assert random.evaluate(seed=7) == random.evaluate(seed=7)
    assert random.evaluate(seed=7) != random.evaluate(seed=8)


def test_invalid_options():
    expr = Expression("1")
    with pytest.raises(TypeError):
        expr.evaluate(timeot=1)
    with pytest.raises(ValueError):
        expr.evaluate(timeout=-1)
    with pytest.raises(TypeError):
        expr.evaluate(max_depth="10")
    with pytest.raises(ValueError):
        expr.evaluate(max_depth=-1)
    with pytest.raises(TypeError):
        expr.evaluate(seed=1.5)
    with pytest.raises(ValueError):
        expr.evaluate(now="not a date")
    with pytest.raises(TypeError):
        expr.evaluate(now=[1])
    assert expr.evaluate(timeout=None, max_depth=None) == 1


def test_options_of_every_evaluate_method():
    expr = Expression("[1..5].$random()")
    document = Document(None)
    values = expr.evaluate(seed=7)
    assert expr.evaluate_json(seed=7) == json.dumps(values, separators=(",", ":"))
    assert expr.evaluate_document(document, seed=7) == values
    assert expr.evaluate_with_audit(seed=7, level="off")["result"] == values
    deep = Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(100))")
    for call in (
        deep.evaluate,
        deep.evaluate_json,
        lambda **options: deep.evaluate_document(document, **options),
    ):
        assert call(max_depth=None) in (100, "100")
        raises("D1011", call, max_depth=100)
        with pytest.raises(TypeError):
            call(timeot=1)
        with pytest.raises(TypeError):
            call(max_depth="10")
    assert deep.evaluate_with_audit(max_depth=100)["error"]["code"] == "D1011"
    with pytest.raises(TypeError):
        expr.evaluate_with_audit(timeot=1)
    with pytest.raises(TypeError):
        expr.evaluate(None, None, 1)


def test_step_limit_takes_the_steps_rust_counts():
    # The table of exact counts of jlens-core (ADR 0018): each expression succeeds with its
    # count and fails with U1006 with one step less, whatever reads the input, and an audit
    # reports that many.
    path = pathlib.Path(__file__).parents[2] / "jlens-core" / "tests" / "steps.json"
    table = json.loads(path.read_text(encoding="utf-8"))
    document = Document(table["input"])
    text = json.dumps(table["input"])

    def check(expr, steps, bindings=None):
        expr.evaluate(table["input"], bindings, max_steps=steps)
        for call in (
            lambda **options: expr.evaluate(table["input"], bindings, **options),
            lambda **options: expr.evaluate_json(text, bindings, **options),
            lambda **options: expr.evaluate_document(document, bindings, **options),
        ):
            error = raises("U1006", call, max_steps=steps - 1)
            assert error.value == steps - 1, expr.source
        audited = expr.evaluate_with_audit(table["input"], bindings)
        assert audited["audit"]["header"]["steps"] == steps, f"{expr.source}: audited"

    checked = 0
    for group in table["groups"]:
        for source, steps in group["counts"]:
            check(Expression(source), steps)
            checked += 1
    assert checked > 20, "the whole table"
    # A host function's calls back spend the budget too: the call, its name and its
    # arguments, then each call back, a step and the body's 3 nodes.
    check(Expression("$twice(function($x){ $x + 1 }, 1)"), 12, {"twice": lambda f, x: f(f(x))})
    loop = Expression("$sum([1..1000].($ * 2))")
    assert loop.evaluate_with_audit(max_steps=100)["error"]["code"] == "U1006"
    with pytest.raises(ValueError):
        loop.evaluate(max_steps=-1)
    with pytest.raises(TypeError):
        loop.evaluate(max_steps=1.5)


def test_document():
    document = Document(ORDER)
    expr = Expression("$sum(items.price)")
    assert expr.evaluate_document(document) == 12.5
    assert expr.evaluate_document(document, {"x": 1}, max_depth=50) == 12.5
    assert Expression("$").evaluate_document(document) == ORDER
    assert Expression("$").evaluate_document(Document.from_json('{"a": [1]}')) == {"a": [1]}
    assert Expression("$").evaluate_document(Document(None)) is None
    with pytest.raises(ValueError):
        Document.from_json("{")
    with pytest.raises(TypeError):
        Document({"a": object()})
    cyclic = {}
    cyclic["a"] = cyclic
    with pytest.raises(ValueError, match="Circular reference detected"):
        Document(cyclic)
    raises("T2001", Expression("customer.name + 1").evaluate_document, document)


def test_documents_are_released():
    expr = Expression("$count(items)")
    for _ in range(100):
        assert expr.evaluate_document(Document(ORDER)) == 2
    gc.collect()


def test_threads():
    document = Document({"items": list(range(1000))})
    expr = Expression("$sum(items)")
    text = json.dumps({"items": list(range(1000))})

    def work(i):
        return (
            expr.evaluate_document(document),
            expr.evaluate_json(text),
            expr.evaluate({"items": list(range(i))}),
        )

    with ThreadPoolExecutor(8) as pool:
        results = list(pool.map(work, range(64)))
    for i, (from_document, from_json, from_values) in enumerate(results):
        assert from_document == 499500
        assert from_json == "499500"
        assert from_values == sum(range(i))


LOADED_ON_A_THREAD_THAT_ENDS = """
import threading

def load():
    import jlens
    assert jlens.Expression("1 + 1").evaluate(None) == 2

thread = threading.Thread(target=load)
thread.start()
thread.join()
import jlens
expr = jlens.Expression("$sum(items)")
results = []
for i in range(8):
    thread = threading.Thread(target=lambda: results.append(expr.evaluate({"items": [1, 2]})))
    thread.start()
    thread.join()
assert results == [3] * 8, results
"""


def test_loaded_on_a_thread_that_ends():
    # mimalloc 3 crashed here on glibc, at a later thread's first allocation (roadmap 0.25 #11).
    # In a new interpreter, since this one has the package loaded already.
    run = subprocess.run([sys.executable, "-c", LOADED_ON_A_THREAD_THAT_ENDS], capture_output=True)
    assert run.returncode == 0, run.stderr.decode(errors="replace")


def test_evaluation_lets_other_threads_run():
    # evaluate_json detaches from the interpreter: a Python thread makes progress meanwhile.
    loop = Expression("($f := function($n){ $f($n + 1) }; $f(0))")
    ticks = []
    stop = threading.Event()

    def tick():
        while not stop.is_set():
            ticks.append(1)
            time.sleep(0.001)

    thread = threading.Thread(target=tick)
    thread.start()
    try:
        raises("D1012", loop.evaluate_json, timeout=0.3)
    finally:
        stop.set()
        thread.join()
    assert len(ticks) > 5


@pytest.mark.skipif(
    not hasattr(sys, "_is_gil_enabled") or sys._is_gil_enabled(),
    reason="free-threaded Python only",
)
def test_free_threaded_input_changed_by_another_thread():
    # The input must not change during an evaluation, but a change never crashes.
    data = {"items": [{"v": i} for i in range(100)]}
    stop = threading.Event()

    def mutate():
        while not stop.is_set():
            data["items"].append({"v": 0})
            data["items"].pop()
            data["x"] = 1
            del data["x"]

    thread = threading.Thread(target=mutate)
    thread.start()
    try:
        for _ in range(200):
            try:
                Expression("$sum(items.v)").evaluate(data)
            except JlensError as error:
                assert error.code == "U1004"
    finally:
        stop.set()
        thread.join()


SALES_ORDER = {
    "number": "SO-12345",
    "customer": {"login": "alice.walker99", "name": "Acme Stores"},
    "items": [
        {"code": "1", "type": "apparel", "unitPrice": 10000.0, "quantity": 2},
        {"code": "2", "type": "electronic", "unitPrice": 25000.0, "quantity": 1},
        {"code": "3", "type": "electronic", "unitPrice": 12000.0, "quantity": 1},
    ],
}

RULE = """(
  $total := $sum(items.(unitPrice * quantity));
  /** @label "Authorized customer" @message "The customer must be approved." */
  customer.login in ["alice.walker99"]
  and
  /** @label "Minimum total" */
  $total > $minimum
)"""


class RateError(Exception):
    code = "ACME01"


def test_host_functions_in_the_bindings():
    def collect(*args):
        return list(args)

    bindings = {
        "add": lambda a, b: a + b,
        "args": collect,
        "types": lambda *args: [type(a).__name__ for a in args],
        "two": lambda v, i: [v, i],
        "pair": lambda: [{"a": 1}, {"a": 2}],
        "nothing": lambda: None,
        "rate": 0.5,
    }

    def ev(source, input=None):
        return Expression(source).evaluate(input, bindings)

    assert ev("$add(1, 2)") == 3
    assert ev("$add(price, $rate)", {"price": 2}) == 2.5
    assert ev("$args(1, 'a', missing, {'k': [1]})", {}) == [1, "a", None, {"k": [1]}]
    assert ev("$types($add, function($x) { $x }, 1)") == ["Function", "Function", "int"]
    assert ev("$pair().a") == [1, 2]
    assert ev("$nothing()") is None
    assert ev("$exists($nothing())") is False
    # The arity, which higher-order functions use, counts the parameters without a default.
    assert ev("$map([5, 6], $args)") == [[5], [6]]
    assert ev("$map([5, 6], $two)") == [[5, 0], [6, 1]]
    assert ev("$add(?, 1)(41)") == 42
    assert ev("$type($add)") == "function"
    assert ev("$sum([1])", {}) == 1
    assert Expression("$sum([1])").evaluate(None, {"sum": lambda *a: "mine"}) == "mine"
    assert Expression("$f()").evaluate_json("{}", {"f": lambda: {"a": 1}}) == '{"a":1}'
    document = Document({"a": 2})
    assert Expression("$f(a)").evaluate_document(document, {"f": lambda x: x * 2}) == 4
    # Methods, classes and built-in functions are callables too.
    assert Expression("$upper('a')").evaluate(None, {"upper": str.upper}) == "A"
    assert Expression("$len('abc')").evaluate(None, {"len": len}) == 3


def test_register_function():
    expr = Expression("$tax(100) + price.$tax()")
    expr.register_function("tax", lambda amount, rate: amount * (rate or 0.25), "<n-n?:n>")
    assert expr.evaluate({"price": 8}) == 27
    raises("T1006", Expression("$tax('x')").evaluate)
    typed = Expression("$tax('x')")
    typed.register_function("$tax", lambda a: a, "<n:n>")
    error = raises("T0410", typed.evaluate)
    assert (error.position, error.token) == (5, "tax")
    # The bindings take precedence; registering again replaces the function.
    assert expr.evaluate({"price": 8}, {"tax": lambda *a: 1}) == 2
    expr.register_function("tax", lambda *a: 3)
    assert expr.evaluate({"price": 8}) == 6
    with pytest.raises(JlensError) as info:
        expr.register_function("f", lambda: 1, "<n<s>:n>")
    assert info.value.code.startswith("S04")
    with pytest.raises(TypeError):
        expr.register_function("f", 1)
    # Usable from other threads, given a large enough stack on musl (see the Python guide).
    musl = platform.libc_ver()[0] == "musl"
    previous = threading.stack_size(8 << 20) if musl else 0
    try:
        with ThreadPoolExecutor(4) as pool:
            assert list(pool.map(lambda p: expr.evaluate({"price": p}), range(8))) == [6] * 8
    finally:
        threading.stack_size(previous)


def test_exceptions_of_host_functions_are_raised_again():
    failing = RateError("The rate service is down")

    def rate():
        raise failing

    expr = Expression("1 + $rate()")
    with pytest.raises(RateError) as info:
        expr.evaluate(None, {"rate": rate})
    assert info.value is failing
    assert (info.value.position, info.value.token) == (10, "rate")
    with pytest.raises(RateError):
        expr.evaluate_json(None, {"rate": rate})
    with pytest.raises(RateError):
        expr.evaluate_document(Document({}), {"rate": rate})

    def interrupt():
        raise KeyboardInterrupt

    # An exception that is not an Exception goes through too.
    with pytest.raises(KeyboardInterrupt):
        expr.evaluate(None, {"rate": interrupt})
    # An exception wrapped by $eval is the error of $eval.
    raises("D3121", Expression("$eval('$rate()')").evaluate, None, {"rate": rate})
    audited = expr.evaluate_with_audit(None, {"rate": rate})
    assert (audited["error"]["code"], audited["error"]["message"]) == (
        "ACME01",
        "The rate service is down",
    )
    audited = expr.evaluate_with_audit(None, {"rate": lambda: {}["x"]})
    assert audited["error"]["code"] == "KeyError"
    # A result that cannot be converted fails like an exception.
    cyclic = {}
    cyclic["self"] = cyclic
    with pytest.raises(ValueError, match="the result of \\$rate"):
        expr.evaluate(None, {"rate": lambda: cyclic})
    with pytest.raises(TypeError):
        expr.evaluate(None, {"rate": lambda: {1, 2}})
    # Expressions evaluated by a host function have their own functions and exceptions.
    inner = Expression("$g()")
    nested = {"f": lambda: inner.evaluate(None, {"g": lambda: 20}) + 1}
    assert Expression("$f() * 2").evaluate(None, nested) == 42
    with pytest.raises(RateError) as info:
        Expression("$f()").evaluate(None, {"f": lambda: inner.evaluate(None, {"g": rate})})
    assert info.value is failing
    assert Expression("$add(1, 2)").evaluate(None, {"add": lambda a, b: a + b}) == 3


def test_asynchronous_host_functions_fail_with_u1002():
    async def fetch():
        return 1

    error = raises("U1002", Expression("$fetch()").evaluate, None, {"fetch": fetch})
    assert error.token == "fetch"
    assert "$fetch" in str(error)


def test_host_functions_call_the_functions_they_receive():
    def safe(f):
        try:
            return f()
        except JlensError as error:
            return error.code

    bindings = {
        "apply": lambda f, *args: f(*args),
        "shape": lambda f: [type(f).__name__, f.arity],
        "safe": safe,
        "give": lambda f: f(lambda: 1, [2]),
        "add": lambda a, b: a + b,
    }

    def ev(source, more=None, **options):
        return Expression(source).evaluate(None, {**bindings, **(more or {})}, **options)

    assert ev("$apply(function($x, $y) { $x + $y }, 1, 2)") == 3
    assert ev("$apply($uppercase, 'a')") == "A"
    assert ev("$apply($substring(?, 1), 'abc')") == "bc"
    assert ev("$apply($uppercase ~> $trim, ' a ')") == "A"
    assert ev("$apply($add, 1, 2)") == 3
    assert ev("($n := 5; $apply(function() { $n + 1 }))") == 6
    assert ev("$shape(function($a, $b) { 0 })") == ["Function", 2]
    assert ev("$apply($string)") == "null"
    assert ev("$give(function($f, $a) { {'f': $type($f), 'a': $a} })") == {"a": [2]}
    assert ev("$apply(function() { $uppercase })") is None
    # None stands for undefined.
    assert Expression("$f(function($x) { $exists($x) })").evaluate(None, {"f": lambda f: f(None)}) is False
    twice = Expression("$twice(function($n) { $n * 3 }, 2)")
    twice.register_function("twice", lambda f, x: f(f(x)), "<fx:x>")
    assert twice.evaluate() == 18
    # Errors are raised with their codes; the host function may catch them or let them through.
    assert ev("$safe(function() { $error('no') })") == "D3137"
    error = raises("D3137", ev, "$apply(function() { $error('no') })")
    assert str(error) == "no"
    audited = Expression("$apply(function() { 1 + 'a' })").evaluate_with_audit(None, bindings)
    assert (audited["error"]["code"], audited["error"]["position"]) == ("T2002", 23)
    failing = RateError("down")

    def rate():
        raise failing

    with pytest.raises(RateError) as info:
        ev("$apply($rate)", {"rate": rate})
    assert info.value is failing
    raises("D1011", ev, "($f := function($n) { $apply($f, $n + 1) }; $f(0))", max_depth=50)
    # Called after the host function returned, it raises U1005.
    kept = []

    def keep(f):
        kept.append(f)
        return f(1)

    assert ev("$keep(function($x) { $x + 1 })", {"keep": keep}) == 2
    error = raises("U1005", kept[0], 1)
    assert error.token == "keep"

    # Called from another thread while the host function runs: U1005 too.
    def other_thread(f):
        with ThreadPoolExecutor(1) as pool:
            return pool.submit(safe, f).result()

    assert ev("$other(function() { 1 })", {"other": other_thread}) == "U1005"
    # Called again while it runs, through another host function: an error, not a crash.
    stash = []

    def outer(f):
        stash.append(f)
        return f()

    reentry = {"outer": outer, "inner": lambda: stash[0]()}
    raises("U1001", ev, "$outer(function() { $inner() })", reentry)
    # Called during an evaluation that lets other threads run.
    expr = Expression("$apply(function($x) { $x * 2 }, a)")
    assert expr.evaluate_json('{"a": 4}', bindings) == "8"
    assert expr.evaluate_document(Document({"a": 5}), bindings) == 10
    assert ev("$apply($add, 2, 3)") == 5


def test_evaluate_with_audit():
    expr = Expression(RULE)
    audit = expr.evaluate_with_audit(SALES_ORDER, {"minimum": 60000})
    assert audit["result"] is False
    assert audit["error"] is None
    assert audit["audit"]["header"]["level"] == "decisions"
    assert audit["audit"]["header"]["result"] is False
    assert audit["audit"]["root"]
    failed = [line["text"] for line in audit["explanation"]["lines"] if line["passed"] is False]
    assert any(text.startswith("Minimum total") for text in failed), failed
    assert audit["explanation_text"].startswith("result: false")
    assert "✘ Minimum total" in audit["explanation_text"]

    full = expr.evaluate_with_audit(
        SALES_ORDER, {"minimum": 1}, level="full", limits={"max_nodes": 5}
    )
    assert full["result"] is True
    assert (full["audit"]["nodeCount"], full["audit"]["truncated"]) == (5, True)
    nodes, stack = 0, [full["audit"]["root"]]
    while stack:
        nodes += 1
        stack.extend(stack.pop().get("children", []))
    assert nodes == 5
    off = expr.evaluate_with_audit(SALES_ORDER, {"minimum": 1}, level="off")
    assert off["audit"].get("root") is None

    failing = Expression("$x.y + 1").evaluate_with_audit(None, {"x": {"y": "a"}})
    assert failing["error"]["code"] == "T2001"
    assert failing["error"]["value"] == "a"
    assert failing["result"] is None
    limited = Expression("[1..100]").evaluate_with_audit(max_array_length=50)
    assert limited["error"]["code"] == "U1001"

    with pytest.raises(ValueError):
        expr.evaluate_with_audit(level="everything")
    with pytest.raises(ValueError):
        expr.evaluate_with_audit(limits={"max_nodez": 1})
    with pytest.raises(TypeError):
        expr.evaluate_with_audit(limits={"max_nodes": "1"})
    with pytest.raises(TypeError):
        expr.evaluate_with_audit(timeot=1)
    cyclic = []
    cyclic.append(cyclic)
    with pytest.raises(ValueError, match="Circular reference detected"):
        expr.evaluate_with_audit(cyclic)


def test_audit_spans_count_code_points():
    source = "'\U0001d11e' = 'x' or /** @label \"B\" */ $b"
    audit = Expression(source).evaluate_with_audit(None, {"b": True})
    spans = []

    def walk(node):
        spans.append((node["source"], node["span"]["start"], node["span"]["end"]))
        for child in node.get("children", []):
            walk(child)

    walk(audit["audit"]["root"])
    comparison = next(span for span in spans if span[0] == "'\U0001d11e' = 'x'")
    assert comparison[1:] == (0, 9)
    assert source[comparison[1] : comparison[2]] == comparison[0]

    # A captured value with a field named `span` is left as it is.
    def outputs(node):
        yield node.get("output")
        for child in node.get("children", []):
            yield from outputs(child)

    span = {"span": {"start": 7, "end": 9}}
    captured = Expression("('\U0001d11e'; $x)").evaluate_with_audit(None, {"x": span}, level="full")
    assert span in list(outputs(captured["audit"]["root"]))


def test_reads_and_union_reads():
    expr = Expression("$sum(items[type = $kind].(unitPrice * quantity))")
    assert expr.reads() == {
        "input": {"items": {"quantity": True, "type": True, "unitPrice": True}},
        "bindings": {"kind": True},
    }
    # A function the analysis does not know may read the context of its call: the whole input.
    priced = Expression("price * $rate(country)")
    assert priced.reads()["input"] is True
    # A Python function does not receive it: given the bindings, the call reads its arguments.
    assert priced.reads({"rate": lambda country: 0.1})["input"] == {"country": True, "price": True}
    # Even when a binding the analysis does not know is used as a value.
    assert Expression("$rate(country) + $kind").reads({"rate": lambda country: 0.1}) == {
        "input": {"country": True},
        "bindings": {"kind": True, "rate": True},
    }
    priced.register_function("rate", lambda country: 0.1)
    assert priced.reads()["input"] == {"country": True, "price": True}
    # A `-` parameter takes the context when the argument is missing; `n` needs no fields of it.
    doubled = Expression("price.$double()")
    doubled.register_function("double", lambda n: n * 2, "<n-:n>")
    assert doubled.reads()["input"] == {"price": {}}
    assert Expression("$eval(rule)").reads()["everyBinding"] is True

    union = jlens.union_reads(
        [Expression("a.b + $x.y").reads(), Expression("$count(c) + $exists(a)").reads()]
    )
    assert union == {"input": {"a": {"b": True}, "c": {}}, "bindings": {"x": {"y": True}}}
    assert jlens.union_reads(({"input": {"a": {"b": True}}, "bindings": {}}, {"input": True})) == {
        "input": True,
        "bindings": {},
    }
    assert jlens.union_reads([]) == {"bindings": {}}
    with pytest.raises(TypeError):
        jlens.union_reads({})
    for bad in [[{"input": False}], [{"inputs": True}], [1]]:
        with pytest.raises(ValueError):
            jlens.union_reads(bad)
    with pytest.raises(TypeError):
        Expression("a").reads([])


def test_from_json_and_evaluate_json_skip_the_fields_not_read():
    text = '{"a": {"b": 1, "c": 2}, "d": [{"e": 1, "f": 2}], "g": "x"}'
    whole = Expression("$")
    cut = Document.from_json(text, {"a": {"b": True}, "d": {"e": True}})
    assert whole.evaluate_document(cut) == {"a": {"b": 1}, "d": [{"e": 1}]}
    counted = Document.from_json(text, Expression("$count(d)").reads()["input"])
    assert whole.evaluate_document(counted) == {"d": [{}]}
    for read in [True, None]:
        assert whole.evaluate_document(Document.from_json(text, read)) == json.loads(text)
    # Another thread parses the text again, cut the same way.
    with ThreadPoolExecutor(1) as pool:
        assert pool.submit(whole.evaluate_document, cut).result() == {"a": {"b": 1}, "d": [{"e": 1}]}
    with pytest.raises(ValueError):
        Document.from_json(text, {"a": False})
    # Skipped fields are checked as JSON syntax only.
    with pytest.raises(ValueError):
        Document.from_json('{"a": 1, "z": [}', {"a": True})
    assert whole.evaluate_document(Document.from_json('{"a": 1, "z": 1e400}', {"a": True})) == {
        "a": 1
    }
    with pytest.raises(ValueError):
        Document.from_json('{"a": 1, "z": 1e400}')

    # evaluate_json cuts its input by the expression's reads().
    assert Expression("a.b").evaluate_json('{"a": {"b": 1}, "z": 1e400}') == "1"
    with pytest.raises(ValueError):
        Expression("$keys($)").evaluate_json('{"z": 1e400}')
    constant = Expression("1 + 1")
    assert constant.evaluate_json('{"z": 1e400}') == "2"
    with pytest.raises(ValueError):
        constant.evaluate_json("[1,")
