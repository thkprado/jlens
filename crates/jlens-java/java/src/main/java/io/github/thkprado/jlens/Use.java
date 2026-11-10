package io.github.thkprado.jlens;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * What a schema says of one value an expression reads: an entry of {@link Uses}, by its path.
 *
 * <p>A value of an enumeration is a JSON value: a {@code String}, a {@link
 * java.math.BigDecimal}, a {@code Boolean}, {@code null}, or a {@code List} or a {@code Map} of
 * such values.
 *
 * @param unknown the schema says nothing of this path: there is no schema for the value, or the
 *     field is not one of those it declares; nothing else is set then, but {@code whole}
 * @param whole the expression reads the value whole ({@code *}, {@code **}, {@code $eval}, an
 *     operand, the result), not only some of its fields: every path the schema has under this
 *     one is in the answer too
 * @param types the types the schema gives the value, as JSON Schema names them ({@code
 *     string}, {@code number}, {@code integer}, {@code boolean}, {@code null}, {@code object},
 *     {@code array}); empty when it gives none
 * @param enumeration the values the schema lists for it, {@code null} when it lists none
 * @param closed the enumeration is closed: declared, not inferred from samples, and with no
 *     open domain, so that a value outside it is warned about ({@code W0206})
 * @param domain the open domain of the value ({@code x-jlens-domain}), or {@code null}
 * @param format the string format of the value, or {@code null}
 * @param required whether the object that holds the field requires it; {@code null} for a path
 *     that is not a field: the value itself, the items of an array. A field is always there
 *     only when the fields on its way, which have their own paths, are required too
 */
public record Use(
        boolean unknown,
        boolean whole,
        List<String> types,
        List<Object> enumeration,
        boolean closed,
        String domain,
        String format,
        Boolean required) {
    /** Copies {@code types} and {@code enumeration}. */
    public Use {
        types = List.copyOf(types);
        // An enumeration may list `null`, which `List.copyOf` refuses.
        if (enumeration != null) {
            enumeration = Collections.unmodifiableList(new ArrayList<>(enumeration));
        }
    }

    /** The use of a parsed JSON object. */
    static Use of(Object value) {
        Map<String, Object> members = Json.object(value, "what the schema says of a path");
        Json.only(members, "what the schema says of a path", "unknown", "whole", "types", "enum",
                "closed", "domain", "format", "required");
        List<String> types = new ArrayList<>();
        List<?> listed = Json.member(members, "types", List.class);
        if (listed != null) {
            for (Object type : listed) {
                if (!(type instanceof String name)) {
                    throw new IllegalArgumentException("a type must be a string, not " + type);
                }
                types.add(name);
            }
        }
        List<?> enumeration = Json.member(members, "enum", List.class);
        return new Use(
                Json.flag(members, "unknown"),
                Json.flag(members, "whole"),
                types,
                enumeration == null ? null : new ArrayList<>(enumeration),
                Json.flag(members, "closed"),
                Json.member(members, "domain", String.class),
                Json.member(members, "format", String.class),
                Json.member(members, "required", Boolean.class));
    }

    /** The JSON form as a value: the members that are set, in the order every package writes. */
    Map<String, Object> tree() {
        Map<String, Object> members = new LinkedHashMap<>();
        if (unknown) members.put("unknown", true);
        if (whole) members.put("whole", true);
        if (!types.isEmpty()) members.put("types", types);
        if (enumeration != null) members.put("enum", enumeration);
        if (closed) members.put("closed", true);
        if (domain != null) members.put("domain", domain);
        if (format != null) members.put("format", format);
        if (required != null) members.put("required", required);
        return members;
    }

    /** The uses of a parsed JSON object from paths to uses, sorted by path. */
    static Map<String, Use> paths(Object value) {
        Map<String, Use> paths = new java.util.TreeMap<>();
        Json.object(value, "the paths").forEach((path, use) -> paths.put(path, of(use)));
        return paths;
    }

    /** The JSON form of a map from paths to uses, as a value. */
    static Map<String, Object> tree(Map<String, Use> paths) {
        Map<String, Object> members = new LinkedHashMap<>();
        paths.forEach((path, use) -> members.put(path, use.tree()));
        return members;
    }
}
