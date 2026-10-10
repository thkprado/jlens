// Tests of the jlens WebAssembly package, shared by the Node.js runner (node.test.mjs) and the
// browser runner (browser.html). Each case receives the initialized module.

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

  ["guardrails", (j) => {
    const loop = new j.Expression("($f := function($n){ $f($n + 1) }; $f(0))");
    throws(() => loop.evaluate(undefined, null, { timeout: 20 }), (e) => equal(e.code, "D1012"));
    const deep = new j.Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(100))");
    throws(() => deep.evaluate(undefined, null, { maxDepth: 100 }), (e) => equal(e.code, "D1011"));
    equal(deep.evaluate(), 100);
  }],

  ["deep recursion and nesting fail cleanly", (j) => {
    // The default depth limit stops the recursion before the engine's stack overflows (an
    // overflow would leave the module unusable).
    const deep = new j.Expression("($f := function($n){ $n = 0 ? 0 : 1 + $f($n - 1) }; $f(1000000))");
    throws(() => deep.evaluate(), (e) => equal(e.code, "D1011"));
    equal(deep.evaluateWithAudit(undefined, null, { level: "full" }).error.code, "D1011", "audited");
    const nested = "(".repeat(1001) + "1" + ")".repeat(1001);
    throws(() => new j.Expression(nested), (e) => equal(e.code, "U1001"));
    const ok999 = "(".repeat(999) + "1" + ")".repeat(999);
    equal(new j.Expression(ok999).evaluate(), 1, "999 levels");
    equal(j.parseRecover(nested).diagnostics[0].code, "U1001", "recovering parse");
    equal(new j.Expression("1 + 1").evaluate(), 2, "still usable");
  }],

  ["invalid arguments are TypeErrors", (j) => {
    const expr = new j.Expression("$x");
    throws(() => expr.evaluate(null, [1]), (e) => ok(e instanceof TypeError, "bindings array"));
    throws(() => expr.evaluate(null, null, { timeout: "soon" }), (e) => ok(e instanceof TypeError, "timeout"));
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
  }],

  ["ast and parseRecover", (j) => {
    const ast = new j.Expression("a[b > 1]").ast();
    equal([ast.type, ast.steps[0].name], ["path", "a"]);
    const parsed = j.parseRecover("é.(a + ");
    equal(parsed.diagnostics.map((d) => [d.severity, d.code]), [["error", "S0203"], ["error", "S0207"]]);
    equal(parsed.diagnostics[0].span.start, 7, "UTF-16 span");
    ok(parsed.ast, "a partial tree");
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

  ["function catalog", (j) => {
    const sum = j.functions().find((f) => f.name === "sum");
    equal([sum.category, sum.signature, sum.params], ["aggregation", "<a<n>:n>", ["array"]]);
  }],
];
