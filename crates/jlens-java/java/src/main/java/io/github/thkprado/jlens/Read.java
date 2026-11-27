package io.github.thkprado.jlens;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * What an expression reads of a value (ADR 0016): the whole value, or only some of its fields.
 * Its JSON form writes {@link Whole} as {@code true} and {@link Fields} as an object. Reads are
 * made, written, compared and printed without recursion, as they nest as deep as the paths of
 * an expression (roadmap 0.37 #4).
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

        /** Equal to {@code other} when it reads the same fields, at every depth. */
        @Override
        public boolean equals(Object other) {
            return other instanceof Fields fields && toJson().equals(fields.toJson());
        }

        @Override
        public int hashCode() {
            return toJson().hashCode();
        }

        /** {@return the read's JSON form, after the record's name} */
        @Override
        public String toString() {
            return "Fields[" + toJson() + "]";
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

    /** The read of a parsed JSON value, its objects read with a stack of their own. */
    @SuppressWarnings("unchecked")
    private static Read of(Object value) {
        List<OpenRead> open = new ArrayList<>();
        Object at = value;
        while (true) {
            Read read = null;
            if (Boolean.TRUE.equals(at)) {
                read = WHOLE;
            } else if (at instanceof Map<?, ?> map) {
                open.add(new OpenRead((Map<String, Object>) map));
            } else {
                throw new IllegalArgumentException("a read is true or an object of reads, not " + at);
            }
            // Puts the read in the innermost object, closing those that end with it.
            while (true) {
                if (read != null) {
                    if (open.isEmpty()) return read;
                    open.getLast().fields.put(open.getLast().name, read);
                }
                OpenRead innermost = open.getLast();
                if (innermost.members.hasNext()) {
                    Map.Entry<String, Object> member = innermost.members.next();
                    innermost.name = member.getKey();
                    at = member.getValue();
                    break;
                }
                open.removeLast();
                read = new Fields(innermost.fields);
            }
        }
    }

    /**
     * The JSON form.
     *
     * @return {@code true}, or an object of reads
     */
    default String toJson() {
        StringBuilder json = new StringBuilder();
        // What is left to write, the next last: reads, and the text between them.
        List<Object> left = new ArrayList<>();
        left.add(this);
        while (!left.isEmpty()) {
            switch (left.removeLast()) {
                case Whole _ -> json.append("true");
                case Fields(Map<String, Read> fields) -> {
                    json.append('{');
                    left.add("}");
                    List<Map.Entry<String, Read>> members = new ArrayList<>(fields.entrySet());
                    for (int i = members.size() - 1; i >= 0; i--) {
                        left.add(members.get(i).getValue());
                        left.add((i > 0 ? "," : "") + Json.quote(members.get(i).getKey()) + ":");
                    }
                }
                case Object text -> json.append(text);
            }
        }
        return json.toString();
    }
}

/** An object of reads being read by {@code Read.of}: its members left, its fields, the member at hand. */
final class OpenRead {
    final Iterator<Map.Entry<String, Object>> members;
    final Map<String, Read> fields = new TreeMap<>();
    String name;

    OpenRead(Map<String, Object> object) {
        members = object.entrySet().iterator();
    }
}
