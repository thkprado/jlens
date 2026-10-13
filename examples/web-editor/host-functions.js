// Host functions: JavaScript functions that the web editor offers to every expression, as an
// application would through `registerFunction`. They are not part of JSONata. The worker
// registers them on each expression; the page declares them to the language service, for
// completion, hover and signature help.
//
// A function that the expression passes to a host function arrives as a JavaScript function
// that calls it (see "Functions as arguments" in the WebAssembly guide).

// Errors that `$try` lets through: the guardrails (time, depth, limits) must stop the
// evaluation, whatever the expression catches.
const GUARDRAILS = new Set(["D1011", "D1012"]);

export const hostFunctions = [
  {
    name: "try",
    signature: "<fx?:x>",
    params: ["step", "fallback"],
    summary:
      "Editor host function (JavaScript). Calls step(); when it fails, gives fallback, " +
      "or calls fallback({code, message}) when it is a function.",
    implementation(step, fallback) {
      try {
        return step();
      } catch (err) {
        if (!err?.code || err.code.startsWith("U") || GUARDRAILS.has(err.code)) throw err;
        const error = { code: err.code, message: String(err.message) };
        return typeof fallback === "function" ? fallback(error) : fallback;
      }
    },
  },
];

/** The declarations of the host functions, for the language service. */
export const hostDeclarations = hostFunctions.map(({ name, signature, params, summary }) => ({
  name,
  signature,
  params,
  summary,
}));
