package io.github.thkprado.jlens;

import java.util.Map;
import java.util.Objects;

/**
 * A tuple of a {@link Schema} that says nothing of the items after it: {@code prefixItems}, or
 * {@code items} as an array until draft-07, with no schema for the rest ({@code items}, {@code
 * additionalItems}, {@code unevaluatedItems}) and no {@code maxItems} that keeps the array to
 * the tuple. JSON Schema then allows any item after the tuple, so the items of the array are
 * checked only at a literal index inside it. The tuple is read: this tells where a schema may
 * say less than its author meant.
 *
 * @param place where the schema that has the tuple is: the URI of its document, as it was
 *     given, and a JSON Pointer as its fragment ({@code #/properties/point} in a schema given
 *     with no URI)
 */
public record OpenTuple(String place) {
    /** Checks that the place is not {@code null}. */
    public OpenTuple {
        Objects.requireNonNull(place, "place");
    }

    /** The tuple of a parsed JSON object, as the C API replies with it. */
    static OpenTuple of(Object value) {
        Map<String, Object> members = Json.object(value, "an open tuple");
        return new OpenTuple(Json.member(members, "place", String.class));
    }
}
