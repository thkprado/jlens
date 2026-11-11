package io.github.thkprado.jlens;

import java.util.Map;
import java.util.Objects;

/**
 * A {@code $ref} that the load of a {@link Schema} did not follow (ADR 0025): the document it
 * leads to was not given, or has no such place. Its place in the schema is unknown: nothing is
 * warned under it.
 *
 * @param place where the {@code $ref} is written: the URI of its document, as it was given,
 *     and a JSON Pointer as its fragment ({@code #/properties/carrier} in a schema given with
 *     no URI)
 * @param reference the {@code $ref} as written
 * @param uri the {@code $ref} resolved against its base: the place it names
 * @param document the URI of the document that was not given, to give to {@link
 *     Schema.Loader#document(String, String)}; {@code null} when the document is there and
 *     has no such place
 */
public record UnresolvedRef(String place, String reference, String uri, String document) {
    /** Checks that only {@code document} may be {@code null}. */
    public UnresolvedRef {
        Objects.requireNonNull(place, "place");
        Objects.requireNonNull(reference, "reference");
        Objects.requireNonNull(uri, "uri");
    }

    /** The reference of a parsed JSON object, as the C API replies with it. */
    static UnresolvedRef of(Object value) {
        Map<String, Object> members = Json.object(value, "an unresolved $ref");
        return new UnresolvedRef(
                Json.member(members, "place", String.class),
                Json.member(members, "reference", String.class),
                Json.member(members, "uri", String.class),
                Json.member(members, "document", String.class));
    }
}
