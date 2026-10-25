package io.github.thkprado.jlens;

import java.util.Collection;
import java.util.Collections;
import java.util.Map;
import java.util.StringJoiner;
import java.util.TreeMap;

/**
 * What an expression reads of its input and of its bindings, given by {@link
 * Expression#reads()} (ADR 0016): an input cut down to {@link #input()} gives the same result.
 *
 * @param input what the expression reads of its input, {@code null} when it does not read it
 * @param bindings what it reads of each binding, by name (without the {@code $})
 * @param everyBinding whether it may read any binding whole, as {@code $eval} can
 */
public record Reads(Read input, Map<String, Read> bindings, boolean everyBinding) {
    /** Copies {@code bindings}, sorted by name. */
    public Reads {
        bindings = Collections.unmodifiableSortedMap(new TreeMap<>(bindings));
    }

    /**
     * What any of several expressions reads: the read to keep for a {@link Document} that all
     * of them evaluate.
     *
     * @param reads the reads of each expression
     * @return their union
     */
    public static Reads union(Collection<Reads> reads) {
        StringJoiner json = new StringJoiner(",", "[", "]");
        reads.forEach(read -> json.add(read.toJson()));
        long buffer = Native.buffer(json.toString());
        try {
            return fromJson(Native.json(Native.unionReads(buffer)));
        } finally {
            Native.free(buffer);
        }
    }

    /**
     * The reads of their JSON form.
     *
     * @param json {@code {"input": Read, "bindings": {name: Read}, "everyBinding": true}}
     * @return the reads
     * @throws IllegalArgumentException when {@code json} is not in that form
     */
    public static Reads fromJson(String json) {
        Map<String, String> members;
        try {
            members = Json.members(json);
        } catch (IllegalStateException e) {
            throw new IllegalArgumentException("not a JSON object: " + json, e);
        }
        Read input = members.containsKey("input") ? Read.fromJson(members.get("input")) : null;
        Map<String, Read> bindings = new TreeMap<>();
        if (members.containsKey("bindings")) {
            if (!(Read.fromJson(members.get("bindings")) instanceof Read.Fields(var fields))) {
                throw new IllegalArgumentException("bindings must be an object");
            }
            bindings.putAll(fields);
        }
        boolean every = members.containsKey("everyBinding")
                && Boolean.TRUE.equals(Json.parse(members.get("everyBinding")));
        return new Reads(input, bindings, every);
    }

    /**
     * The JSON form.
     *
     * @return {@code {"input": Read, "bindings": {name: Read}, "everyBinding": true}}, without
     *     {@code input} when the expression does not read it and without {@code everyBinding}
     *     when it is false
     */
    public String toJson() {
        StringJoiner json = new StringJoiner(",", "{", "}");
        if (input != null) json.add("\"input\":" + input.toJson());
        json.add("\"bindings\":" + new Read.Fields(bindings).toJson());
        if (everyBinding) json.add("\"everyBinding\":true");
        return json.toString();
    }
}
