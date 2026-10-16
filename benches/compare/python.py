"""Compares jlens with the other JSONata implementations for Python, on ../cases.json.

The implementations all install a module named `jsonata`, except jlens, so each one lives in
its own virtual environment; this script runs in each of them:

    python python.py speed  jlens=<python> jsonata-python=<python> jsonata=<python> jsonata-rs=<python>
    python python.py limits jlens=<python> jsonata-python=<python> jsonata=<python> jsonata-rs=<python>

where <python> is the interpreter of the environment that holds that package (`jlens`,
`jsonata-python`, `jsonata` or `jsonata-rs`). Only the standard library is needed.

`speed` prints, for each case, the median time per evaluation over several rounds, with the
input as Python values; for jlens, also against a `Document` converted once. It first checks
that each implementation gives jlens's result.

`limits` runs each case of LIMITS in a process of its own, so that a crash ends that process
only, and prints what came out: a result, an exception, the process killed by a signal, or
still running after LIMIT_WALL seconds.
"""

import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
LIMIT_WALL = 30  # seconds before a limits case is killed


# --- The implementations -------------------------------------------------------------------


class Jlens:
    def __init__(self):
        import jlens

        self.jlens = jlens
        self.version = _version("jlens")

    def compile(self, source):
        return self.jlens.Expression(source)

    def evaluate(self, compiled, input, timeout=None, max_depth=None):
        return compiled.evaluate(input, timeout=timeout, max_depth=max_depth)

    def error(self, error):
        if isinstance(error, self.jlens.JlensError):
            return f"`{error.code}`"
        return None


class JsonataPython:
    """jsonata-python, a port of jsonata-java in pure Python."""

    def __init__(self):
        import jsonata

        self.jsonata = jsonata
        self.version = _version("jsonata-python")

    def compile(self, source):
        return self.jsonata.Jsonata(source)

    def evaluate(self, compiled, input, timeout=None, max_depth=None):
        if timeout is None and max_depth is None:
            return compiled.evaluate(input)
        frame = compiled.create_frame()
        frame.set_runtime_bounds(_ms(timeout), max_depth)
        return compiled.evaluate(input, frame)

    def error(self, error):
        if isinstance(error, self.jsonata.JException):
            return f"`{error.error}`"
        return None


class JsonataWrapper:
    """jsonata, which runs jsonata-js in the Duktape JavaScript engine."""

    def __init__(self):
        import jsonata

        self.context = jsonata.Context()
        self.version = _version("jsonata")

    def compile(self, source):
        # It pastes the expression into JavaScript source between single quotes.
        return source.replace("\\", "\\\\").replace("'", "\\'").replace("\n", "\\n").replace("\r", "")

    def evaluate(self, compiled, input, timeout=None, max_depth=None):
        if timeout is not None or max_depth is not None:
            raise Unsupported
        return self.context(compiled, input)

    def error(self, error):
        if isinstance(error, ValueError) and '"code"' in str(error):
            try:
                return f"`{json.loads(str(error))['code']}`"
            except (ValueError, KeyError):
                return None
        return None


class JsonataRs:
    """jsonata-rs, a port of jsonata-java in Rust."""

    def __init__(self):
        import jsonata

        self.jsonata = jsonata
        self.version = _version("jsonata-rs")

    def compile(self, source):
        return self.jsonata.Jsonata(source)

    def evaluate(self, compiled, input, timeout=None, max_depth=None):
        if timeout is not None or max_depth is not None:
            # It takes both or neither, as C longs.
            never = 2**31 - 1
            compiled.set_runtime_bounds(
                never if timeout is None else _ms(timeout), never if max_depth is None else max_depth
            )
        return compiled.evaluate(input)

    def error(self, error):
        if isinstance(error, self.jsonata.JsonataError):
            return f"`{error.code}`"
        return None


IMPLEMENTATIONS = {
    "jlens": Jlens,
    "jsonata-python": JsonataPython,
    "jsonata": JsonataWrapper,
    "jsonata-rs": JsonataRs,
}


class Unsupported(Exception):
    pass


def _ms(seconds):
    return None if seconds is None else int(seconds * 1000)


def _version(distribution):
    from importlib.metadata import version

    return version(distribution)


def _normalize(value):
    """The result as JSON values, with integral floats as ints, so that results compare."""
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, dict):
        return {k: _normalize(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [_normalize(v) for v in value]
    return value


# --- speed -----------------------------------------------------------------------------------


def _cases():
    spec = json.loads((HERE.parent / "cases.json").read_text(encoding="utf-8"))
    input = json.loads((HERE.parent / spec["input"]).read_text(encoding="utf-8"))
    cases = []
    for case in spec["cases"]:
        source = case.get("expr") or (HERE.parent / case["file"]).read_text(encoding="utf-8")
        cases.append((case["name"], source))
    return input, cases


def _measure(run, rounds=7, round_s=0.2):
    """Median time per call in microseconds, over `rounds` rounds of about `round_s` each."""
    start = time.perf_counter()
    while time.perf_counter() - start < 0.3:  # warm up
        run()
    times = []
    for _ in range(rounds):
        n = 0
        start = time.perf_counter()
        while time.perf_counter() - start < round_s:
            run()
            n += 1
        times.append((time.perf_counter() - start) * 1e6 / n)
    return statistics.median(times)


def speed_child(name):
    """Measures one implementation; prints a JSON object on its last line."""
    impl = IMPLEMENTATIONS[name]()
    input, cases = _cases()
    out = {"version": impl.version, "python": sys.version.split()[0], "cases": {}}
    document = impl.jlens.Document(input) if name == "jlens" else None
    for case, source in cases:
        entry = {}
        try:
            compiled = impl.compile(source)
            entry["result"] = _normalize(impl.evaluate(compiled, input))
            entry["evaluate"] = _measure(lambda: impl.evaluate(compiled, input))
            if document is not None:
                entry["document"] = _measure(lambda: compiled.evaluate_document(document))
        except Exception as error:  # noqa: BLE001 - reported in the table
            entry["error"] = f"{type(error).__name__}: {str(error)[:100]}"
        out["cases"][case] = entry
    print(json.dumps(out))


def _fmt(us):
    if us >= 1e6:
        return f"{us / 1e6:.2f} s"
    if us >= 1000:
        return f"{us / 1000:.2f} ms"
    return f"{us:.1f} µs"


def speed(pythons):
    runs = {}
    for name, python in pythons.items():
        print(f"measuring {name} ...", file=sys.stderr)
        proc = subprocess.run(
            [python, "-I", str(Path(__file__).resolve()), "speed-child", name],
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        if proc.returncode != 0:
            print(f"{name}: exit code {proc.returncode}\n{proc.stderr[-2000:]}", file=sys.stderr)
            continue
        runs[name] = json.loads(proc.stdout.strip().splitlines()[-1])
    reference = runs.get("jlens")
    others = [n for n in runs if n != "jlens"]
    print(", ".join(f"{n} {r['version']}" for n, r in runs.items()), f"on Python {next(iter(runs.values()))['python']}\n")
    header = [f"{n} {runs[n]['version']}" for n in others]
    if reference:
        header += ["jlens `evaluate`", "jlens `evaluate_document`"]
    print("| Case | " + " | ".join(header) + " |")
    print("|---|" + "---:|" * len(header))
    _, cases = _cases()
    for case, _ in cases:
        expected = reference["cases"][case].get("result") if reference else None
        cells = []
        for name in others:
            entry = runs[name]["cases"][case]
            if "error" in entry:
                cells.append(f"fails: {entry['error']}")
            elif reference and entry["result"] != expected:
                cells.append(f"differs: {json.dumps(entry['result'])[:60]}")
            else:
                cells.append(_fmt(entry["evaluate"]))
        if reference:
            entry = reference["cases"][case]
            cells += [_fmt(entry["evaluate"]), _fmt(entry["document"])] if "error" not in entry else [entry["error"], ""]
        print(f"| {case} | " + " | ".join(cells) + " |")


# --- limits ----------------------------------------------------------------------------------


def _nested(depth):
    value = 0
    for _ in range(depth):
        value = {"a": value}
    return value


def _cyclic():
    """`{"n": 1, "child": {"parent": <itself>}}`: the cycle is one level down."""
    value = {"n": 1}
    value["child"] = {"parent": value}
    return value


RECURSION = "($f := function($n){ 1 + $f($n + 1) }; $f(0))"
LOOP = "($f := function($n){ $f($n + 1) }; $f(0))"

# name: (description, make the input, expression, options)
LIMITS = {
    "deep-900": ("input nested 900 levels, `$string($)`", lambda: _nested(900), "$string($)", {}),
    "deep-2000": ("input nested 2,000 levels, `$string($)`", lambda: _nested(2000), "$string($)", {}),
    "deep-100000": ("input nested 100,000 levels, `$string($)`", lambda: _nested(100_000), "$string($)", {}),
    "cycle-field": ("cyclic input, `n`", _cyclic, "n", {}),
    "cycle-string": ("cyclic input, `$string($)`", _cyclic, "$string($)", {}),
    "recursion": ("unbounded recursion", lambda: None, RECURSION, {}),
    "recursion-depth": ("unbounded recursion, depth limit 500", lambda: None, RECURSION, {"max_depth": 500}),
    "loop": ("endless tail call", lambda: None, LOOP, {}),
    "loop-timeout": ("endless tail call, 1-second timeout", lambda: None, LOOP, {"timeout": 1.0}),
}


def limit_child(name, case):
    """Runs one limits case; prints a JSON object on its last line, unless the process dies."""
    impl = IMPLEMENTATIONS[name]()
    _, make, source, options = LIMITS[case]
    input = make()
    start = time.perf_counter()
    try:
        compiled = impl.compile(source)
        result = impl.evaluate(compiled, input, **options)
        text = json.dumps(result) if not isinstance(result, str) else result
        outcome = f"result ({len(text)} characters)" if len(text) > 40 else f"`{text}`"
    except Unsupported:
        outcome = "no such option"
    except BaseException as error:  # noqa: BLE001 - every exception is an outcome
        code = impl.error(error)
        message = " ".join(str(error).split())[:70]
        outcome = f"{type(error).__name__} {code}" if code else f"{type(error).__name__}: {message}"
    elapsed = time.perf_counter() - start
    print(json.dumps({"outcome": outcome, "seconds": elapsed}))


def limits(pythons):
    print("| Case | " + " | ".join(pythons) + " |")
    print("|---|" + "---|" * len(pythons))
    for case, (description, *_rest) in LIMITS.items():
        cells = []
        for name, python in pythons.items():
            print(f"{case}: {name} ...", file=sys.stderr)
            start = time.perf_counter()
            try:
                proc = subprocess.run(
                    [python, "-I", str(Path(__file__).resolve()), "limit-child", name, case],
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                    timeout=LIMIT_WALL,
                )
            except subprocess.TimeoutExpired:
                cells.append(f"still running after {LIMIT_WALL} s")
                continue
            wall = time.perf_counter() - start
            lines = proc.stdout.strip().splitlines()
            if proc.returncode == 0 and lines:
                report = json.loads(lines[-1])
                cells.append(f"{report['outcome']}, {report['seconds']:.2f} s")
            else:
                code = proc.returncode
                code = f"{code:#x}" if code > 255 else str(code)  # Windows status codes in hex
                cells.append(f"process ends, exit code {code}, {wall:.2f} s")
        print(f"| {description} | " + " | ".join(cells) + " |")


def main():
    command, *args = sys.argv[1:]
    if command == "speed-child":
        speed_child(*args)
    elif command == "limit-child":
        limit_child(*args)
    else:
        pythons = dict(arg.split("=", 1) for arg in args)
        {"speed": speed, "limits": limits}[command](pythons)


if __name__ == "__main__":
    main()
