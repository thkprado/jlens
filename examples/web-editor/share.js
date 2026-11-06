// Shared addresses: the expression, the input document, the bindings and the schema overrides, in
// the address after `#s=`, so that no server stores them. The state is JSON text, compressed with `deflate-raw`
// (CompressionStream) and written in base64url. Nothing else depends on the browser, so the
// round trip is also tested in Node.js (share.test.mjs).

/** The prefix of a shared address's fragment. */
export const PREFIX = "#s=";

/**
 * The longest address the Share button gives. Some tools (chat applications, mail clients,
 * proxies) cut addresses beyond about 8 KB.
 */
export const MAX_ADDRESS = 8000;

/** The largest state a shared address may expand to, so that a crafted address cannot. */
const MAX_STATE = 8 << 20;

/**
 * Encodes `{ expression, input, bindings, overrides }` as the fragment of a shared address, with
 * `#s=`. Empty overrides are left out, so a state without them encodes as it did before they
 * existed.
 */
export async function encodeShare({ expression, input, bindings, overrides }) {
  const state = { expression, input, bindings };
  if (overrides && Object.keys(overrides).length > 0) state.overrides = overrides;
  const json = JSON.stringify(state);
  const stream = new Blob([json]).stream().pipeThrough(new CompressionStream("deflate-raw"));
  const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return PREFIX + btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * Decodes the fragment of a shared address (with `#s=`) into
 * `{ expression, input, bindings, overrides }`; an address made before the overrides existed
 * opens with none. Returns `null` for a fragment that is not a shared one; throws for a damaged
 * one.
 */
export async function decodeShare(fragment) {
  if (!fragment.startsWith(PREFIX)) return null;
  const binary = atob(fragment.slice(PREFIX.length).replace(/-/g, "+").replace(/_/g, "/"));
  const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
  const reader = new Blob([bytes])
    .stream()
    .pipeThrough(new DecompressionStream("deflate-raw"))
    .getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > MAX_STATE) {
      await reader.cancel();
      throw new Error("The shared address expands beyond 8 MB.");
    }
    chunks.push(value);
  }
  const state = JSON.parse(new TextDecoder().decode(await new Blob(chunks).arrayBuffer()));
  if (typeof state?.expression !== "string") {
    throw new Error("The shared address holds no expression.");
  }
  const bindings = state.bindings ?? {};
  if (!isObject(bindings)) {
    throw new Error("The bindings of the shared address are not a JSON object.");
  }
  const overrides = state.overrides ?? {};
  if (!isObject(overrides)) {
    throw new Error("The schema overrides of the shared address are not a JSON object.");
  }
  return { expression: state.expression, input: state.input ?? null, bindings, overrides };
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
