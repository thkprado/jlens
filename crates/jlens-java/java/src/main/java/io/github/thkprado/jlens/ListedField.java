package io.github.thkprado.jlens;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A field of the schema of a service's input, or another value it describes, as {@link
 * LanguageService#fields()} lists it: what completion and hover tell of it.
 *
 * <p>A value of an enumeration or of an extension is a JSON value: a {@code String}, a {@link
 * java.math.BigDecimal}, a {@code Boolean}, {@code null}, or a {@code List} or a {@code Map} of
 * such values.
 *
 * @param path the path, written as {@link Uses} writes it: {@code $} for the value itself,
 *     {@code items[]} for the items of {@code items}, a name that is not a plain one between
 *     backquotes
 * @param typeSummary the types as completion and hover give them ({@code array<object>},
 *     {@code string | null}); {@code unknown} when the schema gives none
 * @param types the types the schema gives the value, as JSON Schema names them; empty when it
 *     gives none
 * @param required whether the object that holds the field requires it; {@code null} for a path
 *     that is not a field: the value itself, the items of an array
 * @param label its short display name ({@code title}), or {@code null}
 * @param description what it means, or {@code null}
 * @param enumeration the values the schema lists for it, {@code null} when it lists none
 * @param closed the enumeration is closed, as {@link Use#closed()} tells
 * @param domain its open domain ({@code x-jlens-domain}), or {@code null}
 * @param format its string format, or {@code null}
 * @param extensions its own {@code x-} keywords, as written, which jlens does not read: those
 *     of its place first, then those of the definitions it refers to
 */
public record ListedField(
        String path,
        String typeSummary,
        List<String> types,
        Boolean required,
        String label,
        String description,
        List<Object> enumeration,
        boolean closed,
        String domain,
        String format,
        Map<String, Object> extensions) {
    /** Copies {@code types}, {@code enumeration} and {@code extensions}. */
    public ListedField {
        types = List.copyOf(types);
        // An enumeration may list `null`, and an extension be `null`, which `copyOf` refuses.
        if (enumeration != null) {
            enumeration = Collections.unmodifiableList(new ArrayList<>(enumeration));
        }
        extensions = Collections.unmodifiableMap(new LinkedHashMap<>(extensions));
    }

    /** The fields of the JSON array the C API replies with. */
    static List<ListedField> list(String json) {
        if (!(Json.parse(json) instanceof List<?> items)) {
            throw new IllegalStateException("jlens: unexpected fields from the C API: " + json);
        }
        List<ListedField> fields = new ArrayList<>(items.size());
        for (Object item : items) fields.add(of(item));
        return List.copyOf(fields);
    }

    /** The field of a parsed JSON object. */
    static ListedField of(Object value) {
        String what = "a listed field";
        Map<String, Object> members = Json.object(value, what);
        Json.only(members, what, "path", "typeSummary", "types", "required", "label",
                "description", "enum", "closed", "domain", "format", "extensions");
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
        Map<String, Object> extensions = new LinkedHashMap<>();
        Map<?, ?> written = Json.member(members, "extensions", Map.class);
        if (written != null) {
            written.forEach((keyword, extension) -> extensions.put((String) keyword, extension));
        }
        return new ListedField(
                Json.member(members, "path", String.class),
                Json.member(members, "typeSummary", String.class),
                types,
                Json.member(members, "required", Boolean.class),
                Json.member(members, "label", String.class),
                Json.member(members, "description", String.class),
                enumeration == null ? null : new ArrayList<>(enumeration),
                Json.flag(members, "closed"),
                Json.member(members, "domain", String.class),
                Json.member(members, "format", String.class),
                extensions);
    }

    /** The JSON form as a value: the members that are set, in the order every package writes. */
    Map<String, Object> tree() {
        Map<String, Object> members = new LinkedHashMap<>();
        members.put("path", path);
        members.put("typeSummary", typeSummary);
        if (!types.isEmpty()) members.put("types", types);
        if (required != null) members.put("required", required);
        if (label != null) members.put("label", label);
        if (description != null) members.put("description", description);
        if (enumeration != null) members.put("enum", enumeration);
        if (closed) members.put("closed", true);
        if (domain != null) members.put("domain", domain);
        if (format != null) members.put("format", format);
        if (!extensions.isEmpty()) members.put("extensions", extensions);
        return members;
    }
}
