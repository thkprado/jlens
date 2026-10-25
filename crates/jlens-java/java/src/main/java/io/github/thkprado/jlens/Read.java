package io.github.thkprado.jlens;

import java.util.Map;
import java.util.StringJoiner;
import java.util.TreeMap;

/**
 * What an expression reads of a value (ADR 0016): the whole value, or only some of its fields.
 * Its JSON form writes {@link Whole} as {@code true} and {@link Fields} as an object.
 */
public sealed interface Read permits Read.Whole, Read.Fields {
    /** The value as it is. */
    Read WHOLE = new Whole();

    /** The value as it is. */
    record Whole() implements Read {}

    /**
     * Of an object, only these fields, each cut by its own {@code Read}; of an array, its items,
     * each read the same way, at any depth; of a scalar, the scalar. No fields means the value
     * but none of its fields, as {@code $count} reads it.
     *
     * @param fields the fields read, by name
     */
    record Fields(Map<String, Read> fields) implements Read {
        /** Copies {@code fields}, sorted by name. */
        public Fields {
            fields = java.util.Collections.unmodifiableSortedMap(new TreeMap<>(fields));
        }
    }

    /**
     * The read of its JSON form.
     *
     * @param json {@code true}, or an object of reads
     * @return the read
     * @throws IllegalArgumentException when {@code json} is not a read
     */
    static Read fromJson(String json) {
        Object value;
        try {
            value = Json.parse(json);
        } catch (IllegalStateException e) {
            throw new IllegalArgumentException("not JSON: " + json, e);
        }
        return of(value);
    }

    /** The read of a parsed JSON value. */
    @SuppressWarnings("unchecked")
    private static Read of(Object value) {
        if (Boolean.TRUE.equals(value)) return WHOLE;
        if (value instanceof Map<?, ?> map) {
            Map<String, Read> fields = new TreeMap<>();
            ((Map<String, Object>) map).forEach((name, read) -> fields.put(name, of(read)));
            return new Fields(fields);
        }
        throw new IllegalArgumentException("a read is true or an object of reads, not " + value);
    }

    /**
     * The JSON form.
     *
     * @return {@code true}, or an object of reads
     */
    default String toJson() {
        return switch (this) {
            case Whole _ -> "true";
            case Fields(Map<String, Read> fields) -> {
                StringJoiner json = new StringJoiner(",", "{", "}");
                fields.forEach((name, read) -> json.add(Json.quote(name) + ":" + read.toJson()));
                yield json.toString();
            }
        };
    }
}
