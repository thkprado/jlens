"""The Python package: the cases of the WebAssembly suite (crates/jlens-wasm/tests/suite.mjs)
that apply to it, and the conversions of Python values."""

import collections
import copy
import datetime
import enum
import gc
import json
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


def test_unsupported_values_fail_where_they_are_read():
    error = raises("U1004", Expression("a").evaluate, {"a": {1, 2}})
    assert "Object of type set is not JSON serializable" in str(error)
    error = raises("U1004", Expression("a").evaluate, {"a": {1: 2}})
    assert "keys must be str" in str(error)
    raises("U1004", Expression("$").evaluate, datetime.date(2024, 1, 2))
    # A level the expression does not reach is not read.
    assert Expression("b").evaluate({"a": {"set": {1, 2}}, "b": 3}) == 3


def test_cycles_fail_cleanly():
    cyclic = {"a": 1}
    cyclic["self"] = cyclic
    error = raises("U1004", Expression("$").evaluate, cyclic)
    assert "Circular reference detected" in str(error)
    looped = []
    looped.append(looped)
    raises("U1004", Expression("$count(**)").evaluate, looped)
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
