// Type-checks the declarations of the package (`tsc --noEmit --strict`); it is not run.

import init, {
  type AuditResult,
  type Callable,
  type CompletionKind,
  type Completions,
  type Diagnostic,
  type EvaluateOptions,
  type FunctionDeclaration,
  type FunctionInfo,
  type HostFunction,
  type JlensError,
  type JsonSchema,
  type Read,
  type Reads,
  type ValueRequest,
  Document,
  Expression,
  LanguageService,
  functions,
  inferSchema,
  parseRecover,
  unionReads,
  valueLiteral,
  version,
} from "../pkg/jlens.js";
// The classic script with the .wasm embedded, which declares the global `jlens`.
import "../pkg/jlens.global.js";

export async function check(): Promise<void> {
  await init();
  const v: string = version();

  const expr = new Expression("$sum(items.price)");
  const source: string = expr.source;
  const r1: unknown = expr.evaluate();
  const r2: unknown = expr.evaluate({ items: [] }, { limit: 1 }, { timeout: 100, maxDepth: 50, maxSteps: 1000 });
  const limits: EvaluateOptions = { maxStringLength: 1024, maxArrayLength: 100, maxResultSize: 4096 };
  const clock: EvaluateOptions = { now: new Date(0), seed: 7 };
  expr.evaluate(null, null, { ...limits, ...clock, now: "2024-01-02T03:04:05Z" });
  const r3: string | undefined = expr.evaluateJson('{"items": []}');
  const audit: AuditResult = expr.evaluateWithAudit({}, null, {
    level: "full",
    limits: { maxNodes: 100 },
  });
  const text: string = audit.explanationText;
  const failed = audit.explanation.lines.filter((l) => l.passed === false).map((l) => l.text);
  const root = audit.audit.root;
  const start: number | undefined = root?.span.start;
  const doc = Document.fromJson("{}");
  const cut = Document.fromJson('{"a": 1}', expr.reads().input);
  cut.free();
  const r4: unknown = expr.evaluateDocument(new Document({ items: [] }), { limit: 1 });
  const r5: unknown = expr.evaluateDocument(doc);
  doc.free();
  const add: HostFunction = (a: number, b: number) => a + b;
  const r6: unknown = expr.evaluate({}, { add, rate: 0.5 });
  const apply: HostFunction = (f: Callable, x: unknown) => (f.arity > 0 ? f(x) : f());
  const r7: unknown = expr.evaluate({}, { apply });
  expr.registerFunction("tax", (amount: number) => amount * 0.2, "<n:n>");
  expr.registerFunction("now", () => 0);
  const reads: Reads = expr.reads({ add });
  const input: Read | undefined = reads.input;
  const every: true | undefined = unionReads([reads, expr.reads()]).everyBinding;
  const fields = input === true ? [] : Object.keys(input ?? {});
  const type: string = expr.ast().type;
  expr.free();

  try {
    new Expression("(");
  } catch (e) {
    const err = e as JlensError;
    const code: string = err.code;
    const position: number | undefined = err.position;
    console.log(code, position);
  }

  const { ast, diagnostics } = parseRecover("a +");
  const d: Diagnostic | undefined = diagnostics[0];
  const schema: JsonSchema = inferSchema([{ a: 1 }], { a: { description: "A", enum: [1, 2] } });
  const service = new LanguageService(schema, { limit: { type: "number" }, other: null });
  const plain = new LanguageService();
  const declared: FunctionDeclaration[] = [{ name: "tax", signature: "<n:n>" }, { name: "now" }];
  const withFunctions = new LanguageService(schema, null, declared);
  const completions: Completions = service.complete("a", 1);
  const kinds = completions.items.map((i) => i.kind);
  const value: CompletionKind = "value";
  const request: ValueRequest | undefined = completions.valueRequest;
  const literal: string = valueLiteral("x", request?.quote ?? "'");
  const domain: JsonSchema = inferSchema([{ a: "x" }], { a: { domain: "codes" } });
  const hover = service.hover("a", 0)?.contents;
  const help = plain.signatureHelp("$sum(", 5)?.activeParam;
  const catalog: FunctionInfo[] = functions();
  console.log(v, source, r1, r2, r3, r4, r5, r6, r7, withFunctions, every, fields, text, failed, start, type, ast.id, d, kinds, value, request, literal, domain, hover, help, catalog);
}

// The global set by jlens.global.js: the module's exports, with `init` for its default export.
export async function checkGlobal(): Promise<void> {
  await jlens.init();
  jlens.initSync();
  const expr: Expression = new jlens.Expression("$sum(items.price)");
  const document: Document = new jlens.Document({ items: [] });
  const v: string = jlens.version();
  const r: unknown = expr.evaluateDocument(document);
  // @ts-expect-error: the global has no default export
  jlens.default;
  // @ts-expect-error: the global is read-only
  jlens.version = () => "";
  console.log(v, r);
}
