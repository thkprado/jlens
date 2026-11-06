// Open domains: values only the application can fetch, such as the customers of a store. A
// field declares one with `x-jlens-domain` in its JSON Schema, or `domain` in the schema
// overrides; completion then returns a value request, and the editor asks `domainValues` for the
// values that match what is typed. This stand-in answers from a fixed list after a short delay,
// as a server would.

const DELAY_MS = 150;
const LIMIT = 20;

const sources = {
  customers: [
    "alice.walker99",
    "bob.harrison42",
    "carla.mendes",
    "chen.wei",
    "dmitri.volkov",
    "fatima.zahra",
    "joao.silva",
    "kenji.tanaka",
    "lucia.romero",
    "olivia.brown7",
  ],
};

/** The values of `request.domain` that start with `request.prefix`, ignoring case. */
export async function domainValues(request) {
  const values = sources[request.domain] ?? [];
  await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
  const prefix = request.prefix.toLowerCase();
  return values.filter((value) => value.toLowerCase().startsWith(prefix)).slice(0, LIMIT);
}
