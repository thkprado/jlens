/**
 * Variables and functions an expression is evaluated with: `{limit: 10}` defines `$limit`, and
 * `{rate: (region) => 0.2}` defines `$rate(region)`. Values are converted to JSON; functions are
 * called as host functions (see `HostFunction`).
 */
export type Bindings = Record<string, unknown>;

/**
 * A JavaScript function called from an expression. Arguments and the result are converted
 * through JSON: `undefined` stands for undefined, and a function argument arrives as a
 * `Callable`. `this` is `undefined`; use a signature with `-` to receive the context value.
 * Without a signature, `length` tells higher-order functions how many arguments to pass. An
 * exception it throws is rethrown by the evaluation, with the `position` and `token` of the call
 * when it has none. Returning a promise fails with `U1002`.
 */
export type HostFunction = (...args: any[]) => unknown;

/**
 * A JSONata function passed to a `HostFunction`, such as the lambda in `$retry(function() {...},
 * 3)`. Calling it applies the function, synchronously, with the context value `null`: arguments
 * and the result are converted through JSON, and a JavaScript function among the arguments
 * becomes `undefined`. Its errors are thrown as a `JlensError` with their codes, or as the
 * exception a host function threw. `length` is 0 and `arity` is the number of parameters, as in
 * jsonata-js. Valid until the host function returns: calling it later throws `U1005`.
 */
export type Callable = ((...args: any[]) => unknown) & { readonly arity: number };

/**
 * Evaluation guardrails, and the clock and seed that make an evaluation reproducible. Each
 * option is read once, as a property (getters and inherited properties included, `toJSON` not
 * called); `undefined` and `null` mean the default. A number must be finite: `NaN` and
 * `Infinity` throw a `TypeError`, as other invalid values do.
 */
export interface EvaluateOptions {
  /** Maximum duration of one evaluation, in milliseconds (error `D1012`). */
  timeout?: number;
  /** Maximum evaluation depth (default 500; error `D1011`). */
  maxDepth?: number;
  /**
   * Maximum number of evaluation steps (error `U1006`): each node evaluated, each function
   * called back and each 1,024 steps of a regular expression's backtracking. The same
   * expression, input and options take the same steps on every machine and natively.
   */
  maxSteps?: number;
  /** Maximum length, in bytes of UTF-8, of a string the evaluation builds (default 256 MiB; error `U1001`). */
  maxStringLength?: number;
  /** Maximum number of items of an array the evaluation builds (default 16,777,216; error `U1001`). */
  maxArrayLength?: number;
  /**
   * Maximum size of the result as compact JSON text, in bytes (default 256 MiB; error `U1001`).
   * It also limits the arguments of a host function. A value that appears several times counts
   * each time.
   */
  maxResultSize?: number;
  /**
   * The evaluation's timestamp for `$now()` and `$millis()`: an integer number of milliseconds
   * since the epoch, a valid `Date` or an ISO 8601 timestamp. Default: the system clock.
   */
  now?: number | Date | string;
  /** Seed for `$random()` and `$shuffle()`: the same seed gives the same results. */
  seed?: number;
}

/** A range of the expression source, in UTF-16 code units (`end` is exclusive). */
export interface Span {
  start: number;
  end: number;
}

/**
 * The error thrown by `new Expression()`, `evaluate()` and `evaluateJson()`. Check it with
 * `err.name === "JlensError"`.
 */
export interface JlensError extends Error {
  name: "JlensError";
  /** The JSONata error code, such as `S0202` or `T2001`. */
  code: string;
  /** Where the error was detected, like jsonata-js: usually just after the offending token. */
  position?: number;
  /** The offending source range, when known. */
  start?: number;
  end?: number;
  /** The token involved, when there is one. */
  token?: string;
  /** The value involved, when there is one. */
  value?: unknown;
}

/** The fields of a `JlensError`, as reported in an `AuditResult`. */
export interface ErrorInfo {
  code: string;
  message: string;
  position?: number;
  start?: number;
  end?: number;
  token?: string;
  value?: unknown;
}

// ---------------------------------------------------------------------------------------------
// Auditing

/** What an audit records. */
export type AuditLevel = "off" | "decisions" | "full";

/** Bounds on what an audit captures. */
export interface CaptureLimits {
  /** Items kept from an array (and fields from an object) in captured values. Default 10. */
  maxItems: number;
  /** Characters kept from a string in captured values. Default 200. */
  maxString: number;
  /** Nesting depth kept in captured values. Default 4. */
  maxDepth: number;
  /** Rejected items kept as samples per filter. Default 5. */
  maxRejected: number;
  /** Nodes recorded in the tree. Default 10000. */
  maxNodes: number;
  /** Characters of source text kept per node. Default 200. */
  maxSource: number;
}

/** Options of `evaluateWithAudit`. */
export interface AuditOptions extends EvaluateOptions {
  /** Default `"decisions"`. */
  level?: AuditLevel;
  limits?: Partial<CaptureLimits>;
}

/** The result of `evaluateWithAudit`. */
export interface AuditResult {
  /** The result; absent when undefined or when the evaluation failed. */
  result?: unknown;
  /** The error, when the evaluation failed. */
  error?: ErrorInfo;
  /** The audit tree. */
  audit: AuditTree;
  /** Why the result was reached. */
  explanation: Explanation;
  /** `explanation` rendered as text, as printed by `jlens eval --explain`. */
  explanationText: string;
}

export interface AuditTree {
  header: AuditHeader;
  /** Absent with level `"off"`. */
  root?: AuditNode;
  nodeCount: number;
  /** Whether nodes were dropped because of `maxNodes`. */
  truncated: boolean;
}

export interface AuditHeader {
  /** `jlens-core` and its version. */
  engine: string;
  /** FNV-1a 64-bit hash of the expression source, in hexadecimal. */
  expressionHash: string;
  /** FNV-1a 64-bit hash of the input document's JSON text, in hexadecimal. */
  inputHash: string;
  /** The evaluation timestamp (`$now()`), in ISO 8601. */
  timestamp: string;
  durationMs: number;
  /** The steps the evaluation took, which an evaluation without an audit takes too. */
  steps: number;
  level: AuditLevel;
  result?: unknown;
  error?: { code: string; message: string };
}

export type OutcomeKind = "true" | "false" | "value" | "undefined" | "error";

/** One evaluation of one node. */
export interface AuditNode {
  /** The syntax-tree node id. */
  node: number;
  /** The node type (`binary`, `path`, `function`, ...). */
  kind: string;
  /** The operator, function or variable name. */
  op?: string;
  span: Span;
  /** The node's source text (possibly shortened). */
  source: string;
  /** Annotation `@label`, `@message` and `@id`. */
  label?: string;
  message?: string;
  checkId?: string;
  /** The context value (level `"full"` only). */
  input?: unknown;
  output?: unknown;
  outcome: OutcomeKind;
  error?: { code: string; message: string };
  /** The compared values, for comparisons (`null` stands for undefined). */
  operands?: [unknown, unknown];
  /** For `and`/`or`: the right side was not evaluated. */
  shortCircuited?: boolean;
  /** For conditionals: the branch taken. */
  branch?: "then" | "else" | "none";
  /** For `$name := value`: the variable bound. */
  binding?: string;
  filters?: FilterAudit[];
  children?: AuditNode[];
}

/** Statistics of a predicate filter. */
export interface FilterAudit {
  node: number;
  source: string;
  label?: string;
  tested: number;
  passed: number;
  /** A sample of the items that did not pass. */
  rejected?: unknown[];
}

export interface Explanation {
  /** The result, or the error, as text. */
  result: string;
  lines: ExplanationLine[];
}

export interface ExplanationLine {
  depth: number;
  /** `true` for a check that held, `false` for one that failed, `null` otherwise. */
  passed: boolean | null;
  text: string;
  /** The annotation `@message`, shown for failed checks. */
  message?: string;
  /** The syntax-tree node the line is about. */
  node: number;
}

// ---------------------------------------------------------------------------------------------
// The fields an expression reads

/**
 * What an expression reads of a value: `true` for the whole value; an object for only the fields
 * it lists, each with its own `Read` (of an array, its items, each read the same way; of a
 * scalar, the scalar). An empty object needs the value but none of its fields, as `$count` does.
 */
export type Read = true | { [field: string]: Read };

/** What an expression reads of its input and of its bindings, given by `Expression.reads()`. */
export interface Reads {
  /** Absent when the expression does not read its input. */
  input?: Read;
  /** The bindings the expression reads, by name (without the `$`). */
  bindings: Record<string, Read>;
  /** Set when the expression may read any binding whole, as `$eval` can: keep all bindings. */
  everyBinding?: true;
}

// ---------------------------------------------------------------------------------------------
// Parsing

/** A node of the processed syntax tree. The `type` field tells the kind of node. */
export interface AstNode {
  id: number;
  type: string;
  span: Span;
  annotation?: Annotation;
  [field: string]: unknown;
}

/** An audit annotation, read from a `/** ... *\/` comment. */
export interface Annotation {
  label?: string;
  message?: string;
  id?: string;
  description?: string;
  span: Span;
}

export interface Diagnostic {
  severity: "error" | "warning";
  /** A JSONata error code (`S0xxx`) or a jlens warning code (`W0xxx`). */
  code: string;
  message: string;
  span: Span;
}

/** The result of `parseRecover`. */
export interface ParseResult {
  ast: AstNode;
  diagnostics: Diagnostic[];
}

// ---------------------------------------------------------------------------------------------
// Schemas and the language service

/** A JSON Schema document (2020-12). */
export type JsonSchema = boolean | { [keyword: string]: unknown };

export type JsonType = "null" | "boolean" | "integer" | "number" | "string" | "array" | "object";

/** Settings applied to the value at one path of an inferred schema. */
export interface SchemaOverride {
  description?: string;
  label?: string;
  /** Hide from completion. */
  hidden?: boolean;
  enum?: unknown[];
  format?: string;
  type?: JsonType[];
  examples?: unknown[];
}

export type CompletionKind = "field" | "variable" | "function" | "keyword" | "operator";

export interface CompletionItem {
  label: string;
  kind: CompletionKind;
  /** The type of a field or variable, the signature of a function. */
  detail?: string;
  /** Markdown. */
  documentation?: string;
  /** The text to insert in place of `Completions.range`. */
  insertText: string;
}

export interface Completions {
  /** The range the chosen item replaces (the word under the cursor). */
  range: Span;
  /** Best first. */
  items: CompletionItem[];
}

export interface Hover {
  range: Span;
  /** Markdown. */
  contents: string;
}

export interface SignatureHelp {
  /** The function name, with `$`. */
  name: string;
  signature: string;
  params: string[];
  /** The parameter being edited (0-based). */
  activeParam: number;
  summary: string;
}

export type FunctionCategory =
  | "string"
  | "numeric"
  | "aggregation"
  | "boolean"
  | "array"
  | "object"
  | "higher-order"
  | "date-time"
  | "encoding"
  | "misc";

/** A host function declared to the language service, completed like a built-in function. */
export interface FunctionDeclaration {
  /** The name, with or without `$`. */
  name: string;
  /** The signature, such as `<n-n?:n>`. */
  signature?: string;
  /** Parameter names, without `$`, shown by signature help. */
  params?: string[];
  summary?: string;
}

export interface FunctionInfo {
  /** The name, without `$`. */
  name: string;
  signature: string;
  params: string[];
  category: FunctionCategory;
  summary: string;
}

// ---------------------------------------------------------------------------------------------
// Classes and functions with optional arguments (declared here so that the arguments are optional)

/**
 * A compiled JSONata expression. Compile once, then evaluate against any number of documents.
 * Call `free()` when it is no longer needed, or rely on the garbage collector.
 */
export class Expression {
  /** Compiles a JSONata expression. Throws a `JlensError` on syntax errors. */
  constructor(source: string);
  /** The source text of the expression. */
  readonly source: string;
  /**
   * Evaluates the expression against `input`, a JSON-compatible value (`undefined` means no
   * input document). Returns `undefined` when the result is undefined; throws a `JlensError`
   * when the evaluation fails.
   */
  evaluate(input?: unknown, bindings?: Bindings | null, options?: EvaluateOptions): unknown;
  /**
   * Like `evaluate`, with the input document and the result as JSON text. This avoids
   * converting large documents to and from JavaScript objects. The fields of the input that the
   * expression does not read (`reads()`) are skipped while parsing, checked as JSON syntax only.
   */
  evaluateJson(
    input?: string | null,
    bindings?: Bindings | null,
    options?: EvaluateOptions,
  ): string | undefined;
  /**
   * Evaluates the expression against a `Document` converted beforehand, which avoids converting
   * the input on every call. Use it to evaluate several expressions against one document.
   */
  evaluateDocument(document: Document, bindings?: Bindings | null, options?: EvaluateOptions): unknown;
  /**
   * Evaluates the expression and records how the result was reached. Evaluation errors are
   * reported in the returned `error`, not thrown; an exception of a host function is reported
   * with its `code` (or its `name`) and its `message`.
   */
  evaluateWithAudit(input?: unknown, bindings?: Bindings | null, options?: AuditOptions): AuditResult;
  /**
   * Makes `implementation` available as `$name` in every evaluation, as jsonata-js does. With a
   * `signature` (such as `<n-n?:n>`), arguments are checked before each call, as for built-in
   * functions. A function of the same name in the bindings takes precedence. Throws a
   * `JlensError` when the signature is invalid.
   */
  registerFunction(name: string, implementation: HostFunction, signature?: string): void;
  /**
   * The fields of the input and of the bindings the expression reads: an input cut down to them
   * gives the same result. The functions in `bindings`, and those registered on the expression,
   * read only their arguments; a call to a function it does not know reads its context whole.
   */
  reads(bindings?: Bindings | null): Reads;
  /** The processed syntax tree, as plain objects. */
  ast(): AstNode;
  /** Releases the compiled expression. */
  free(): void;
  [Symbol.dispose](): void;
}

/**
 * A JSON document converted once into jlens's internal representation. Evaluating against a
 * `Document` (`expr.evaluateDocument(doc)`) skips converting the input on every call.
 */
export class Document {
  /** Converts a JSON-compatible value. */
  constructor(input: unknown);
  /**
   * Parses JSON text. Throws a `TypeError` when it is not valid JSON. With `read` (the `input`
   * of `reads()` or of `unionReads()`), the fields it leaves out are skipped, checked as JSON
   * syntax only, and the document serves the expressions it comes from as the whole one would.
   */
  static fromJson(text: string, read?: Read | null): Document;
  free(): void;
  [Symbol.dispose](): void;
}

/**
 * Completion, diagnostics, hover and signature help for expressions over documents described by
 * a JSON Schema.
 */
export class LanguageService {
  /**
   * `schema` describes the input documents; without it, functions and variables are still
   * completed. `variables` declares the bindings the expressions are evaluated with (names with
   * or without `$`), optionally with a schema for their values. `functions` declares the host
   * functions they are evaluated with.
   */
  constructor(
    schema?: JsonSchema | null,
    variables?: Record<string, JsonSchema | null> | null,
    functions?: FunctionDeclaration[] | null,
  );
  /** Suggestions for the word at `offset`, and the range they replace. */
  complete(text: string, offset: number): Completions;
  /** Syntax errors, and references to fields the schema does not have. */
  diagnostics(text: string): Diagnostic[];
  /** The type and description of what is under `offset`, if anything. */
  hover(text: string, offset: number): Hover | undefined;
  /** The parameters of the function call around `offset`, if any. */
  signatureHelp(text: string, offset: number): SignatureHelp | undefined;
  free(): void;
  [Symbol.dispose](): void;
}

/**
 * What any of several expressions reads: an input cut down to the union gives each of them its
 * result. Throws a `TypeError` when an item is not in the form `reads()` returns.
 */
export function unionReads(reads: Reads[]): Reads;

/**
 * Infers a JSON Schema (2020-12, with `x-jlens-*` annotations) from sample documents.
 * `overrides` adds descriptions, labels, allowed values and hidden fields by path, such as
 * `"items[].type"`.
 * Paths missing from the samples are declared; a malformed path or override throws a `TypeError`.
 */
export function inferSchema(
  samples: unknown[],
  overrides?: Record<string, SchemaOverride>,
): JsonSchema;
