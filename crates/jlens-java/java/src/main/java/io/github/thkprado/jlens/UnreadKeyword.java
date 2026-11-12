package io.github.thkprado.jlens;

import java.util.Map;
import java.util.Objects;

/**
 * A keyword that the load of a {@link Schema} did not read: the place of the schema that has
 * it is unknown, so that nothing is warned under it.
 *
 * @param place where the schema that has the keyword is: the URI of its document, as it was
 *     given, and a JSON Pointer as its fragment ({@code #/properties/code} in a schema given
 *     with no URI)
 * @param keyword the keyword: {@code $dynamicRef} or {@code $recursiveRef}, which the scope of a
 *     validation resolves, or {@code type} with a name that is not a JSON type
 */
public record UnreadKeyword(String place, String keyword) {
    /** Checks that neither is {@code null}. */
    public UnreadKeyword {
        Objects.requireNonNull(place, "place");
        Objects.requireNonNull(keyword, "keyword");
    }

    /** The keyword of a parsed JSON object, as the C API replies with it. */
    static UnreadKeyword of(Object value) {
        Map<String, Object> members = Json.object(value, "an unread keyword");
        return new UnreadKeyword(
                Json.member(members, "place", String.class),
                Json.member(members, "keyword", String.class));
    }
}
