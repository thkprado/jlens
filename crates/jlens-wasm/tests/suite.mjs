// Tests of the jlens WebAssembly package, shared by the Node.js runner (node.test.mjs) and the
// browser runner (browser.html). Each case receives the initialized module and the environment:
// `virtualTime` is true under headless Chrome's virtual time (`--virtual-time-budget`).

const order = {
  number: "SO-12345",
  customer: { login: "alice.walker99", name: "Acme Stores" },
  items: [
    { code: "1", color: "blue", type: "apparel", unitPrice: 10000.0, quantity: 2, tax: 1500.0 },
    { code: "2", color: "black", type: "electronic", unitPrice: 25000.0, quantity: 1, tax: 4000.0 },
    { code: "3", color: "white", type: "electronic", unitPrice: 12000.0, quantity: 1, tax: 3000.0 },
  ],
};

const rule = `(
  $total := $sum(items.(unitPrice * quantity));
  /** @label "Authorized customer" @message "The customer must be approved." */
  customer.login in ["alice.walker99"]
  and
  /** @label "Minimum total" */
  $total > $minimum
)`;

function equal(actual, expected, what = "value") {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${what}: expected ${e}, got ${a}`);
}

function ok(condition, what) {
  if (!condition) throw new Error(what);
}

function throws(fn, check) {
  try {
    fn();
  } catch (err) {
    check(err);
    return;
  }
  throw new Error("expected an exception");
}

export const cases = [
  ["version", (j) => ok(/^\d+\.\d+\.\d+/.test(j.version()), "a semantic version")],

  ["evaluate with input and bindings", (j) => {
    const expr = new j.Expression("$sum(items[type = $kind].(unitPrice * quantity))");
    equal(expr.evaluate(order, { kind: "electronic" }), 37000);
    equal(expr.source, "$sum(items[type = $kind].(unitPrice * quantity))", "source");
    expr.free();
  }],

  ["undefined result and no input", (j) => {
    equal(new j.Expression("missing").evaluate(order), undefined);
    equal(new j.Expression("1 + 2").evaluate(), 3);
    equal(new j.Expression("$").evaluate(), undefined);
  }],

  ["objects keep their field order; numbers match JavaScript", (j) => {
    const result = new j.Expression('{"z": 1, "a": [2e21, 0.1 + 0.2, 1/3, $power(2, 64)]}').evaluate();
    equal(Object.keys(result), ["z", "a"], "keys");
    equal(result.a, [2e21, 0.1 + 0.2, 1 / 3, 2 ** 64], "numbers");
  }],

  ["documents", (j) => {
    const doc = new j.Document(order);
    const rules = ["$sum(items.(unitPrice * quantity))", "customer.login", "items[type = $t].code"];
    equal(rules.map((r) => new j.Expression(r).evaluateDocument(doc, { t: "electronic" })), [57000, "alice.walker99", ["2", "3"]]);
    const transform = new j.Expression("$ ~> |items|{'n': 1}, ['tax']|");
    transform.evaluateDocument(doc);
    equal(new j.Expression("items[0]").evaluateDocument(doc), order.items[0], "unchanged");
    equal(new j.Expression("a").evaluateDocument(j.Document.fromJson('{"a": [1, {"b": null}]}')), [1, { b: null }]);
    throws(() => j.Document.fromJson("{oops"), (e) => ok(e instanceof TypeError, "TypeError"));
    throws(() => new j.Document(undefined), (e) => ok(e instanceof TypeError, "undefined"));
    doc.free();
  }],

  ["evaluateJson", (j) => {
    const expr = new j.Expression("items[0].{'code': code, 'total': unitPrice * quantity}");
    equal(expr.evaluateJson(JSON.stringify(order)), '{"code":"1","total":20000}');
    equal(expr.evaluateJson("{}"), undefined);
    throws(() => expr.evaluateJson("{not json"), (e) => ok(e instanceof TypeError, "TypeError"));
  }],

  ["evaluate reads the input like JSON.stringify, one level at a time", (j) => {
    const value = (source, input) => new j.Expression(source).evaluate(input);
    const input = {
      date: new Date(Date.UTC(2024, 0, 2)),
      boxed: [new Number(1), new String("s"), new Boolean(false)],
      gaps: [undefined, () => 1, Symbol("s"), NaN, -Infinity],
      skipped: { u: undefined, f() {}, s: Symbol("s"), kept: 1 },
      custom: { toJSON(key) { return { key }; } },
      text: "a\u00e9\u{1d11e}b",
      nested: { a: [{ b: "x" }, { b: "y" }] },
    };
    equal(value("date", input), "2024-01-02T00:00:00.000Z", "toJSON");
    equal(value("boxed", input), [1, "s", false], "boxed primitives");
    equal(value("gaps", input), [null, null, null, null, null], "gaps in arrays");
    equal(value("$keys(skipped)", input), "kept", "left out of objects");
    equal(value("custom", input), { key: "custom" }, "toJSON with its key");
    equal(value("$length(text) & text", input), "4" + input.text, "strings");
    equal(value("nested.a.b", input), ["x", "y"], "nested levels");
    equal(value("$", input), JSON.parse(JSON.stringify(input)), "the whole input");
    equal(value("$", new Date(0)), "1970-01-01T00:00:00.000Z", "a Date as the input");
    equal(value("$", () => 1), undefined, "a function as the input");
    // A cycle or a BigInt throws JSON.stringify's TypeError, only when its level is read.
    const tricky = { ok: { a: 1 }, deep: {}, big: { n: 10n } };
    tricky.deep.again = tricky.deep;
    equal(value("ok.a", tricky), 1, "levels not read");
    throws(() => value("deep.again", tricky), (e) => ok(e instanceof TypeError && /circular/.test(e.message), "cycle"));
    throws(() => value("big", tricky), (e) => ok(e instanceof TypeError && /BigInt/.test(e.message), "BigInt"));
    // A toJSON that throws: its own exception.
    const boom = new Error("boom");
    throws(() => value("x", { x: { toJSON() { throw boom; } } }), (e) => ok(e === boom, "the same exception"));
  }],

  ["syntax errors are JlensErrors with UTF-16 offsets", (j) => {
    throws(() => new j.Expression("'𝄞é' & items[0;]"), (e) => {
      ok(e instanceof Error, "an Error");
      equal(e.name, "JlensError", "name");
      equal(e.code, "S0202", "code");
      // '𝄞é' is 4 UTF-16 code units long (6 with the quotes): `;` is at 15.
      equal([e.start, e.end, e.position], [15, 16, 16], "start, end, position");
      equal(e.token, ";", "token");
      ok(e.message.startsWith('Expected "]"'), "message without the code");
    });
  }],

  ["evaluation errors", (j) => {
    throws(() => new j.Expression("customer.name + 1").evaluate(order), (e) => {
      equal([e.name, e.code], ["JlensError", "T2001"]);
      equal(e.value, "Acme Stores", "value");
    });
  }],

  ["guardrails", (j, env) => {
    // Virtual time does not always advance during a synchronous loop, so there a wall-clock
    // timeout might never fire and the page would hang. Node.js runs this check.
    if (!env.virtualTime) {
      const loop = new j.Expression("($f := function($n){ $f($n + 1) }; $f(0))");
      throws(() => loop.evaluate(undefined, null, { timeout: 20 }), (e) => equal(e.code, "D1012"));
      // A catastrophic backtrack stops inside the match (ADR 0009); unbounded, it runs for minutes.
      const backtrack = new j.Expression("$contains($, /^(a+)+$/)");
      const started = Date.now();
      throws(() => backtrack.evaluate("a".repeat(30) + "!", null, { timeout: 50 }), (e) => equal(e.code, "D1012"));
      ok(Date.now() - started < 2000, "the match stops near the timeout");
    }
    const deep = new j.Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(100))");
    throws(() => deep.evaluate(undefined, null, { maxDepth: 100 }), (e) => equal(e.code, "D1011"));
    equal(deep.evaluate(), 100);
  }],

  ["size limits", (j) => {
    const long = new j.Expression("$pad('', 100, 'x')");
    throws(() => long.evaluate(null, null, { maxStringLength: 50 }), (e) => equal(e.code, "U1001"));
    equal(long.evaluate(null, null, { maxStringLength: 100 }).length, 100);
    const wide = new j.Expression("[1..100]");
    throws(() => wide.evaluate(null, null, { maxArrayLength: 50 }), (e) => equal(e.code, "U1001"));
    // The same object twice: each copy counts toward the result's size.
    const shared = new j.Expression("($x := {'s': $pad('', 40, 'x')}; {'l': $x, 'r': $x})");
    throws(() => shared.evaluate(null, null, { maxResultSize: 80 }), (e) => equal(e.code, "U1001"));
    throws(() => shared.evaluateJson(null, null, { maxResultSize: 80 }), (e) => equal(e.code, "U1001"));
    equal(shared.evaluate(null, null, { maxResultSize: 200 }).r.s.length, 40);
    equal(shared.evaluateWithAudit(null, null, { maxResultSize: 80 }).error.code, "U1001", "audited");
  }],

  ["clock and seed", (j) => {
    const clock = new j.Expression("$now()");
    const now = Date.UTC(2024, 0, 2, 3, 4, 5);
    const iso = "2024-01-02T03:04:05.000Z";
    equal(clock.evaluate(null, null, { now }), iso, "milliseconds");
    equal(clock.evaluate(null, null, { now: new Date(now) }), iso, "Date");
    equal(clock.evaluate(null, null, { now: iso }), iso, "ISO string");
    const random = new j.Expression("[1..5].$random()");
    equal(random.evaluate(null, null, { seed: 7 }), random.evaluate(null, null, { seed: 7 }), "same seed");
    ok(
      JSON.stringify(random.evaluate(null, null, { seed: 7 })) !==
        JSON.stringify(random.evaluate(null, null, { seed: 8 })),
      "different seeds",
    );
  }],

  ["deep recursion and nesting fail cleanly", (j) => {
    // The default depth limit stops the recursion before the engine's stack overflows (an
    // overflow would leave the module unusable).
    const deep = new j.Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(1000000))");
    throws(() => deep.evaluate(), (e) => equal(e.code, "D1011"));
    equal(deep.evaluateWithAudit(undefined, null, { level: "full" }).error.code, "D1011", "audited");
    // After a few runs V8 recompiles the hot functions with larger frames (TurboFan). The default
    // depth must hold then too, for the shape with the largest frames (the guide's table); 800
    // overflows by the third run.
    const orderBy = new j.Expression("($f := function($n){ [$n, $n]^($f($n - 1)) }; $f(0))");
    for (let i = 0; i < 20; i++) {
      equal(orderBy.evaluateWithAudit(undefined, null, { level: "full" }).error.code, "D1011", `run ${i}`);
    }
    // A tail call leaves only the native function's calls back on the stack.
    const callback = new j.Expression("($f := function($n){ $map([$n], function($v){ $f($v) }) }; $f(0))");
    throws(() => callback.evaluate(), (e) => equal(e.code, "D1011"));
    const nested = "(".repeat(1001) + "1" + ")".repeat(1001);
    throws(() => new j.Expression(nested), (e) => equal(e.code, "U1001"));
    const ok999 = "(".repeat(999) + "1" + ")".repeat(999);
    // Each level of nesting is a level of evaluation too, beyond the default depth of 500.
    equal(new j.Expression(ok999).evaluate(undefined, null, { maxDepth: 1000 }), 1, "999 levels");
    equal(j.parseRecover(nested).diagnostics[0].code, "U1001", "recovering parse");
    // Values nested more than 1,000 levels deep, in the input or built by a tail-recursive
    // function, fail with U1001 where they are converted, compared or serialized.
    let input = 1;
    for (let i = 0; i < 100000; i++) input = [input];
    for (const source of ["$", "$string($)", "$count(**)", "[$, $] ~> $distinct"]) {
      throws(() => new j.Expression(source).evaluate(input), (e) => equal(e.code, "U1001", source));
    }
    equal(new j.Expression("$count($)").evaluate(input), 1, "a level of the input");
    equal(new j.Expression("$ = $").evaluate(input), true, "the same input");
    const build = "$build := function($n, $acc) { $n = 0 ? $acc : $build($n - 1, {'a': $acc}) }";
    throws(() => new j.Expression(`(${build}; $build(100000, 1))`).evaluate(), (e) => equal(e.code, "U1001"));
    equal(new j.Expression(`(${build}; $count([$build(100000, 1)]))`).evaluate(), 1, "dropped");
    equal(new j.Expression(`(${build}; $build(1000, 1) = $build(1000, 1))`).evaluate(), true, "1,000 levels");
    equal(new j.Expression("1 + 1").evaluate(), 2, "still usable");
  }],

  ["host functions in the bindings", (j) => {
    const bindings = {
      add: (a, b) => a + b,
      args: function (a) { return [...arguments].map((x) => (x === undefined ? null : x)); },
      types: function () { return [...arguments].map((x) => typeof x); },
      two: (v, i) => [v, i],
      pair: () => [{ a: 1 }, { a: 2 }],
      nothing: () => undefined,
      self: function () { return this === undefined; },
      rate: 0.5,
    };
    const ev = (src, input) => new j.Expression(src).evaluate(input, bindings);
    equal(ev("$add(1, 2)"), 3);
    equal(ev("$add(price, $rate)", { price: 2 }), 2.5, "with a variable");
    equal(ev("$args(1, 'a', missing, {'k': [1]})", {}), [1, "a", null, { k: [1] }], "arguments as JSON");
    equal(ev("$types($add, function($x) { $x }, 1)"), ["function", "function", "number"], "function arguments");
    equal(ev("$pair().a"), [1, 2], "results are values");
    equal(ev("$nothing()"), undefined, "undefined result");
    equal(ev("$self()"), true, "this");
    // `length` is the arity that higher-order functions use.
    equal(ev("$map([5, 6], $args)"), [[5], [6]], "map, one parameter");
    equal(ev("$map([5, 6], $two)"), [[5, 0], [6, 1]], "map, two parameters");
    equal(ev("$add(?, 1)(41)"), 42, "partial application");
    equal(ev("$type($add)"), "function", "type");
    equal(ev("$sum([1])", {}), 1, "built-ins still work");
    equal(new j.Expression("$sum([1])").evaluate(null, { sum: () => "mine" }), "mine", "shadowing");
    equal(new j.Expression("$f()").evaluateJson("{}", { f: () => ({ a: 1 }) }), '{"a":1}', "evaluateJson");
    equal(new j.Expression("$f(a)").evaluateDocument(new j.Document({ a: 2 }), { f: (x) => x * 2 }), 4, "documents");
  }],

  ["registerFunction", (j) => {
    const expr = new j.Expression("$tax(100) + price.$tax()");
    expr.registerFunction("tax", (amount, rate) => amount * (rate ?? 0.25), "<n-n?:n>");
    equal(expr.evaluate({ price: 8 }), 27, "with a signature");
    throws(() => new j.Expression("$tax('x')").evaluate(), (e) => equal(e.code, "T1006"));
    const typed = new j.Expression("$tax('x')");
    typed.registerFunction("$tax", (a) => a, "<n:n>");
    throws(() => typed.evaluate(), (e) => {
      equal([e.name, e.code, e.position, e.token], ["JlensError", "T0410", 5, "tax"]);
    });
    // The bindings take precedence; registering again replaces the function.
    equal(expr.evaluate({ price: 8 }, { tax: () => 1 }), 2, "bindings first");
    expr.registerFunction("tax", () => 3);
    equal(expr.evaluate({ price: 8 }), 6, "replaced");
    throws(() => expr.registerFunction("f", () => 1, "<n<s>:n>"), (e) => {
      equal(e.name, "JlensError");
      ok(e.code.startsWith("S04"), e.code);
    });
  }],

  ["exceptions of host functions are rethrown", (j) => {
    class RateError extends Error {}
    const failing = new RateError("The rate service is down");
    failing.code = "ACME01";
    const expr = new j.Expression("1 + $rate()");
    throws(() => expr.evaluate(null, { rate: () => { throw failing; } }), (e) => {
      ok(e === failing, "the same exception");
      equal([e.position, e.token], [10, "rate"], "position and token");
    });
    throws(() => expr.evaluate(null, { rate: () => { throw "plain"; } }), (e) => equal(e, "plain"));
    // An exception wrapped by $eval is the error of $eval.
    throws(() => new j.Expression("$eval('$rate()')").evaluate(null, { rate: () => { throw failing; } }), (e) => {
      equal([e.name, e.code], ["JlensError", "D3121"]);
    });
    const audited = expr.evaluateWithAudit(null, { rate: () => { throw failing; } });
    equal([audited.error.code, audited.error.message], ["ACME01", "The rate service is down"], "audit");
    const typeError = expr.evaluateWithAudit(null, { rate: () => null.x });
    equal(typeError.error.code, "TypeError", "the name as the code");
    // A result that cannot be converted fails like an exception.
    const cyclic = {};
    cyclic.self = cyclic;
    throws(() => expr.evaluate(null, { rate: () => cyclic }), (e) => ok(e instanceof TypeError, "TypeError"));
    // Expressions evaluated by a host function have their own functions and exceptions.
    const inner = new j.Expression("$g()");
    const nested = { f: () => inner.evaluate(null, { g: () => 20 }) + 1 };
    equal(new j.Expression("$f() * 2").evaluate(null, nested), 42, "nested evaluations");
    throws(() => new j.Expression("$f()").evaluate(null, { f: () => inner.evaluate(null, { g: () => { throw failing; } }) }), (e) => {
      ok(e === failing, "rethrown through both evaluations");
    });
    equal(new j.Expression("$add(1, 2)").evaluate(null, { add: (a, b) => a + b }), 3, "still usable");
  }],

  ["asynchronous host functions fail with U1002", (j) => {
    throws(() => new j.Expression("$fetch()").evaluate(null, { fetch: async () => 1 }), (e) => {
      equal([e.name, e.code, e.token], ["JlensError", "U1002", "fetch"]);
      ok(e.message.includes("$fetch"), e.message);
    });
  }],

  ["host functions call the functions they receive", (j) => {
    const bindings = {
      apply: (f, ...args) => f(...args),
      shape: (f) => [typeof f, f.length, f.arity],
      safe: (f) => { try { return f(); } catch (e) { return e.code; } },
      give: (f) => f(() => 1, [2]),
      add: (a, b) => a + b,
    };
    const ev = (src, more, options) => new j.Expression(src).evaluate(null, { ...bindings, ...more }, options);
    equal(ev("$apply(function($x, $y) { $x + $y }, 1, 2)"), 3, "a lambda");
    equal(ev("$apply($uppercase, 'a')"), "A", "a built-in function");
    equal(ev("$apply($substring(?, 1), 'abc')"), "bc", "a partial application");
    equal(ev("$apply($uppercase ~> $trim, ' a ')"), "A", "a chain");
    equal(ev("$apply($add, 1, 2)"), 3, "a host function");
    equal(ev("($n := 5; $apply(function() { $n + 1 }))"), 6, "a closure");
    equal(ev("$shape(function($a, $b) { 0 })"), ["function", 0, 2], "length 0 and arity");
    equal(ev("$apply($string)"), "null", "the context is null");
    equal(ev("$give(function($f, $a) { {'f': $type($f), 'a': $a} })"), { a: [2] }, "a JavaScript function becomes undefined");
    equal(ev("$apply(function() { $uppercase })"), undefined, "a function result is undefined");
    const twice = new j.Expression("$twice(function($n) { $n * 3 }, 2)");
    twice.registerFunction("twice", (f, x) => f(f(x)), "<fx:x>");
    equal(twice.evaluate(), 18, "with a signature");
    // Errors are thrown with their codes; the host function may catch them or let them through.
    equal(ev("$safe(function() { $error('no') })"), "D3137", "caught");
    throws(() => ev("$apply(function() { $error('no') })"), (e) => {
      equal([e.name, e.code, e.message], ["JlensError", "D3137", "no"]);
    });
    const audited = new j.Expression("$apply(function() { 1 + 'a' })").evaluateWithAudit(null, bindings);
    equal([audited.error.code, audited.error.position], ["T2002", 23], "audited, at the error's position");
    const failing = new Error("down");
    throws(() => ev("$apply($rate)", { rate: () => { throw failing; } }), (e) => ok(e === failing, "a host exception"));
    const deep = "($f := function($n) { $apply($f, $n + 1) }; $f(0))";
    throws(() => ev(deep, {}, { maxDepth: 50 }), (e) => equal(e.code, "D1011"));
    // Called after the host function returned, it throws U1005.
    let kept;
    equal(ev("$keep(function($x) { $x + 1 })", { keep: (f) => { kept = f; return f(1); } }), 2, "kept");
    throws(() => kept(1), (e) => equal([e.name, e.code, e.token], ["JlensError", "U1005", "keep"]));
    // Called again while it runs, through another host function: an error, not a crash.
    let stash;
    const reentry = { outer: (f) => { stash = f; return f(); }, inner: () => stash() };
    throws(() => ev("$outer(function() { $inner() })", reentry), (e) => equal(e.code, "U1001"));
    equal(ev("$apply($add, 2, 3)"), 5, "still usable");
  }],

  ["invalid arguments are TypeErrors", (j) => {
    const expr = new j.Expression("$x");
    throws(() => expr.evaluate(null, [1]), (e) => ok(e instanceof TypeError, "bindings array"));
    throws(() => expr.evaluate(null, () => 1), (e) => ok(e instanceof TypeError, "bindings function"));
    throws(() => expr.evaluate(null, null, { timeout: "soon" }), (e) => ok(e instanceof TypeError, "timeout"));
    throws(() => expr.evaluate(null, null, { maxResultSize: -1 }), (e) => ok(e instanceof TypeError, "size"));
    throws(() => expr.evaluate(null, null, { now: "yesterday" }), (e) => ok(e instanceof TypeError, "now"));
    throws(() => expr.evaluate(null, null, { seed: 1.5 }), (e) => ok(e instanceof TypeError, "seed"));
    throws(
      () => expr.evaluateWithAudit(null, null, { level: "everything" }),
      (e) => ok(e instanceof TypeError, "level"),
    );
  }],

  ["evaluateWithAudit", (j) => {
    const expr = new j.Expression(rule);
    const audit = expr.evaluateWithAudit(order, { minimum: 60000 });
    equal(audit.result, false, "result");
    equal(audit.error, undefined, "error");
    equal(audit.audit.header.level, "decisions", "level");
    equal(audit.audit.header.result, false, "header result");
    ok(audit.audit.root, "a root node");
    const failed = audit.explanation.lines.filter((l) => l.passed === false).map((l) => l.text);
    ok(failed.some((t) => t.startsWith("Minimum total")), `failed checks: ${failed}`);
    ok(audit.explanationText.startsWith("result: false"), audit.explanationText);
    ok(audit.explanationText.includes("✘ Minimum total"), audit.explanationText);

    const full = expr.evaluateWithAudit(order, { minimum: 1 }, { level: "full", limits: { maxNodes: 5 } });
    equal(full.result, true, "full result");
    equal([full.audit.nodeCount, full.audit.truncated], [5, true], "limits");

    const failing = new j.Expression("$x.y + 1").evaluateWithAudit(undefined, { x: { y: "a" } });
    equal(failing.error.code, "T2001", "reported error");
    equal(failing.result, undefined, "no result");
  }],

  ["audit spans are UTF-16 offsets", (j) => {
    const audit = new j.Expression("'𝄞' = 'x' or /** @label \"B\" */ $b").evaluateWithAudit(undefined, { b: true });
    const spans = [];
    const walk = (n) => { spans.push([n.source, n.span.start, n.span.end]); (n.children || []).forEach(walk); };
    walk(audit.audit.root);
    const eq = spans.find(([s]) => s === "'𝄞' = 'x'");
    equal(eq.slice(1), [0, 10], "span of the comparison");
    // A captured value's own `span` and `range` fields are data, left as they are.
    const value = { span: { start: 4, end: 9 }, range: { start: 9, end: 9 } };
    const full = new j.Expression("($e := 'é'; $x)").evaluateWithAudit(undefined, { x: value }, { level: "full" });
    const outputs = [];
    const collect = (n) => { if (n.source === "$x") outputs.push(n.output); (n.children || []).forEach(collect); };
    collect(full.audit.root);
    equal(outputs, [value], "captured value");
  }],

  ["ast and parseRecover", (j) => {
    const ast = new j.Expression("a[b > 1]").ast();
    equal([ast.type, ast.steps[0].name], ["path", "a"]);
    const parsed = j.parseRecover("é.(a + ");
    equal(parsed.diagnostics.map((d) => [d.severity, d.code]), [["error", "S0203"], ["error", "S0207"]]);
    equal(parsed.diagnostics[0].span.start, 7, "UTF-16 span");
    ok(parsed.ast, "a partial tree");
  }],

  ["reads and unionReads", (j) => {
    const expr = new j.Expression("$sum(items[type = $kind].(unitPrice * quantity))");
    equal(expr.reads(), {
      input: { items: { quantity: true, type: true, unitPrice: true } },
      bindings: { kind: true },
    });
    // A function the analysis does not know may read the context of its call: the whole input.
    const priced = new j.Expression("price * $rate(country)");
    equal(priced.reads().input, true, "unknown function");
    // A JavaScript function cannot: given the bindings, the call reads only its arguments.
    equal(priced.reads({ rate: () => 0.1 }).input, { country: true, price: true }, "binding");
    // Even when a binding the analysis does not know is used as a value.
    equal(new j.Expression("$rate(country) + $kind").reads({ rate: () => 0.1 }), {
      input: { country: true },
      bindings: { kind: true, rate: true },
    }, "escaped binding");
    priced.registerFunction("rate", () => 0.1);
    equal(priced.reads().input, { country: true, price: true }, "registered");
    equal(new j.Expression("$eval(rule)").reads().everyBinding, true, "everyBinding");

    const union = j.unionReads([
      new j.Expression("a.b + $x.y").reads(),
      new j.Expression("$count(c) + $exists(a)").reads(),
    ]);
    equal(union, { input: { a: { b: true }, c: {} }, bindings: { x: { y: true } } });
    equal(j.unionReads([{ input: { a: { b: true } }, bindings: {} }, { input: true }]).input, true);
    equal(j.unionReads([]), { bindings: {} }, "empty union");
    for (const bad of [{}, [{ input: false }], [{ inputs: true }]]) {
      throws(() => j.unionReads(bad), (e) => ok(e instanceof TypeError, "a TypeError"));
    }
  }],

  ["inferSchema with overrides", (j) => {
    const schema = j.inferSchema([order], {
      "items[].type": { description: "Product category", enum: ["apparel", "electronic", "toy"] },
    });
    equal(schema.type, "object", "type");
    const type = schema.properties.items.items.properties.type;
    equal([type.description, type.enum], ["Product category", ["apparel", "electronic", "toy"]]);
    // Paths missing from the samples are declared.
    const declared = j.inferSchema([order], { "customer.country": { type: ["string"] } });
    equal(declared.properties.customer.properties.country.type, "string", "declared field");
    throws(() => j.inferSchema([order], { "items[].type": { colour: "x" } }), (e) => {
      ok(e instanceof TypeError, "TypeError");
      ok(e.message.includes("colour"), e.message);
    });
    throws(() => j.inferSchema(order), (e) => ok(e instanceof TypeError, "not an array"));
  }],

  ["completion", (j) => {
    const service = new j.LanguageService(j.inferSchema([order]), { minimum: { type: "number" } });
    let text = "items[type = 'electronic'].";
    let c = service.complete(text, text.length);
    equal(
      c.items.filter((i) => i.kind === "field").map((i) => i.label),
      ["code", "color", "type", "unitPrice", "quantity", "tax"],
      "fields",
    );
    // UTF-16 offsets on both sides.
    text = "'𝄞é' & customer.lo";
    c = service.complete(text, text.length);
    equal(c.items.map((i) => i.label), ["login"], "labels");
    equal(c.range, { start: text.length - 2, end: text.length }, "range");
    text = "$mi";
    equal(service.complete(text, 3).items[0].label, "$minimum", "variables");
  }],

  ["diagnostics, hover and signature help", (j) => {
    const service = new j.LanguageService(j.inferSchema([order]));
    const diagnostics = service.diagnostics("'é' & customer.Login");
    equal(diagnostics.map((d) => d.code), ["W0201"], "codes");
    ok(diagnostics[0].message.includes("did you mean `login`"), diagnostics[0].message);
    equal(diagnostics[0].span, { start: 15, end: 20 }, "span");
    const hover = service.hover("customer.login", 11);
    ok(hover.contents.includes("string"), hover.contents);
    equal(hover.range, { start: 9, end: 14 }, "hover range");
    equal(service.hover("   ", 1), undefined, "nothing to hover");
    const help = service.signatureHelp("$substring('abc', ", 18);
    equal([help.name, help.activeParam], ["$substring", 1]);
    equal(new j.LanguageService().diagnostics("a +").map((d) => d.code), ["S0207"]);
  }],

  ["declared host functions in the language service", (j) => {
    const service = new j.LanguageService(null, null, [
      { name: "$shipping", signature: "<ns?:n>", params: ["weight", "region"], summary: "Shipping cost" },
      { name: "sum" },
    ]);
    const items = service.complete("$s", 2).items.filter((i) => i.kind === "function");
    equal(items.slice(0, 2).map((i) => [i.label, i.detail ?? null]), [["$shipping", "<ns?:n>"], ["$sum", null]]);
    const help = service.signatureHelp("$shipping(3, ", 13);
    equal([help.params, help.activeParam, help.summary], [["$weight", "$region"], 1, "Shipping cost"]);
    throws(() => new j.LanguageService(null, null, [{ name: "x", sig: "<n>" }]), (e) => ok(e instanceof TypeError, "unknown field"));
    throws(() => new j.LanguageService(null, null, {}), (e) => ok(e instanceof TypeError, "not an array"));
  }],

  ["function catalog", (j) => {
    const sum = j.functions().find((f) => f.name === "sum");
    equal([sum.category, sum.signature, sum.params], ["aggregation", "<a<n>:n>", ["array"]]);
  }],
];
