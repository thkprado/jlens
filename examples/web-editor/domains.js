// Open domains: values only the application can fetch, such as the customers of a store. A
// field declares one with `x-jlens-domain` in its JSON Schema, or `domain` in the schema
// overrides; completion then returns a value request, and the editor asks `domainValues` for the
// values that match what is typed. This stand-in answers from a fixed list after a short delay,
// as a server would.
//
// A domain may depend on another value: the subgroups of a group. The request carries its
// `conditions`, the fields compared with literals beside the value (by `=` or `in`, joined to it
// by `and` in the same predicate), and `subgroups` answers with those of the groups they name.

const DELAY_MS = 150;
const LIMIT = 20;

const subgroups = {
  Electronics: ["Cables", "Headphones", "Phones", "Speakers", "TVs"],
  Home: ["Furniture", "Kitchen", "Lighting", "Living room"],
  Sports: ["Camping", "Cycling", "Fitness"],
};

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
  countries: ["AR", "BR", "CA", "CL", "DE", "ES", "FR", "GB", "IT", "JP", "MX", "PT", "US", "UY"],
  products: [
    "CABLE-USB-C",
    "HDMI-2M",
    "KEYBOARD-PT",
    "LAPTOP-14",
    "MONITOR-27",
    "MOUSE-WIRELESS",
    "PHONE-CASE",
    "TV-43-LED",
    "TV-55-OLED",
    "TV-65-OLED",
  ],
  groups: Object.keys(subgroups),
};

/** The values of `request.domain`; those of `subgroups` depend on the groups beside them. */
function candidates(request) {
  if (request.domain !== "subgroups") return sources[request.domain] ?? [];
  const groups = request.conditions
    .filter((condition) => condition.path === "items[].categories[].group")
    .flatMap((condition) => condition.values);
  // No group beside the value: every subgroup.
  const named = groups.length ? groups : Object.keys(subgroups);
  return [...new Set(named.flatMap((group) => subgroups[group] ?? []))].sort();
}

/** The values of `request.domain` that start with `request.prefix`, ignoring case. */
export async function domainValues(request) {
  const values = candidates(request);
  await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
  const prefix = request.prefix.toLowerCase();
  return values.filter((value) => value.toLowerCase().startsWith(prefix)).slice(0, LIMIT);
}
