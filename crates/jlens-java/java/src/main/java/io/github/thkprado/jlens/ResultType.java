package io.github.thkprado.jlens;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * What the typing knows of the result of an expression, from {@link
 * LanguageService#resultType(String)}: the types it may have, and whether it may be undefined or
 * several values, with the operand that makes it so.
 *
 * @param types the JSON types the result may have when it is defined, as JSON Schema names
 *     them; {@code null} when they are not known (a function the expression defines, an object
 *     constructor, no schema, a syntax error). An integer is also a number. Several values
 *     gathered in an array, as a path over several items gives them, count as an array
 * @param undefined the operand that may make the result undefined (a field that may be absent,
 *     a filter, an array that may be empty, a condition without {@code else}...); {@code null}
 *     when it is always defined, as far as the typing knows: with {@code types} unknown, it may
 *     be undefined too
 * @param sequence the operand that may make the result several values gathered in an array,
 *     and one value alone (a step over an array, a filter); {@code null} when it cannot
 * @param check how the result compares with the types the service expects ({@link
 *     LanguageService.Builder#result(String...)}); {@code null} when it expects none
 */
public record ResultType(List<String> types, Span undefined, Span sequence, Check check) {
    /** Copies {@code types}, when there are some. */
    public ResultType {
        types = types == null ? null : List.copyOf(types);
    }

    /** How the result of an expression compares with the types expected of it. */
    public enum Check {
        /** Always one of the expected types, and never undefined. */
        ALWAYS,
        /** Never one of them, when it is defined. */
        NEVER,
        /** It may be another type, or undefined. */
        MAYBE,
        /** The typing does not know. */
        UNKNOWN,
    }

    /** The result type of the JSON object the C API replies with. */
    static ResultType of(String json) {
        try {
            Map<String, Object> members = Json.object(Json.parse(json), "a result type");
            Json.only(members, "a result type", "types", "undefined", "sequence", "check");
            List<String> types = null;
            if (members.get("types") instanceof List<?> listed) {
                types = new ArrayList<>(listed.size());
                for (Object name : listed) types.add((String) name);
            }
            Check check = members.get("check") instanceof String name
                    ? Check.valueOf(name.toUpperCase(java.util.Locale.ROOT))
                    : null;
            return new ResultType(types, span(members.get("undefined")),
                    span(members.get("sequence")), check);
        } catch (RuntimeException e) {
            throw new IllegalStateException(
                    "jlens: unexpected result type from the C API: " + json, e);
        }
    }

    private static Span span(Object value) {
        if (value == null) return null;
        Map<?, ?> span = (Map<?, ?>) value;
        return new Span(((BigDecimal) span.get("start")).intValueExact(),
                ((BigDecimal) span.get("end")).intValueExact());
    }
}
