/** Variables an expression is evaluated with: `{limit: 10}` defines `$limit`. */
export type Bindings = Record<string, unknown>;

/** Evaluation guardrails. */
export interface EvaluateOptions {
  /** Maximum duration of one evaluation, in milliseconds (error `D1012`). */
  timeout?: number;
  /** Maximum evaluation depth (error `D1011`). */
  maxDepth?: number;
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
   * converting large documents to and from JavaScript objects.
   */
  evaluateJson(
    input?: string | null,
    bindings?: Bindings | null,
    options?: EvaluateOptions,
  ): string | undefined;
  /**
   * Evaluates the expression and records how the result was reached. Evaluation errors are
   * reported in the returned `error`, not thrown.
   */
  evaluateWithAudit(input?: unknown, bindings?: Bindings | null, options?: AuditOptions): AuditResult;
  /** The processed syntax tree, as plain objects. */
  ast(): AstNode;
  /** Releases the compiled expression. */
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
   * or without `$`), optionally with a schema for their values.
   */
  constructor(schema?: JsonSchema | null, variables?: Record<string, JsonSchema | null>);
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
 * Infers a JSON Schema (2020-12, with `x-jlens-*` annotations) from sample documents.
 * `overrides` adds descriptions, labels, allowed values and hidden fields by path, such as
 * `"items[].type"`.
 * Paths missing from the samples are declared; a malformed path or override throws a `TypeError`.
 */
export function inferSchema(
  samples: unknown[],
  overrides?: Record<string, SchemaOverride>,
): JsonSchema;
