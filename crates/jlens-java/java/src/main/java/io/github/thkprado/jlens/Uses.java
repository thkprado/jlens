package io.github.thkprado.jlens;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.TreeMap;

/**
 * What an expression uses of the schema of its input and of each variable, given by {@link
 * LanguageService#uses(Expression)}: an application that keeps the answer beside an expression
 * can tell, when a new version of the schema arrives, whether anything the expression uses
 * changed ({@link #changes(Uses)}).
 *
 * <p>Each value read is a map from paths to what the schema says there ({@link Use}). In a
 * path, {@code $} is the value itself, {@code items[].type} the field {@code type} of the
 * items of {@code items}, and a name that is not a plain one is written between backquotes.
 * Every value on the way to a field read has its own path, and so do the items of an array.
 *
 * {@snippet :
 * try (LanguageService service = LanguageService.of(schema);
 *      Expression expression = Expression.compile("$sum(items[type = \"book\"].price)")) {
 *     Uses uses = service.uses(expression);
 *     uses.input().get("items[].price").types();   // [number]
 *     String kept = uses.toJson();                 // stored beside the expression
 * }
 * }
 *
 * @param input what the expression uses of its input, by path; {@code null} when it does not
 *     read it
 * @param bindings what it uses of each variable it reads, by name (without the {@code $}) and
 *     then by path; the host functions declared to the service are left out
 * @param everyBinding whether it may read any variable whole, as {@code $eval} can: every
 *     variable declared with a schema is then in {@code bindings}, whole
 */
public record Uses(
        Map<String, Use> input, Map<String, Map<String, Use>> bindings, boolean everyBinding) {
    /** Copies the maps, sorted by path and by name. */
    public Uses {
        if (input != null) input = Collections.unmodifiableSortedMap(new TreeMap<>(input));
        TreeMap<String, Map<String, Use>> copy = new TreeMap<>();
        bindings.forEach((name, paths) ->
                copy.put(name, Collections.unmodifiableSortedMap(new TreeMap<>(paths))));
        bindings = Collections.unmodifiableSortedMap(copy);
    }

    /**
     * What changed between this answer, that of the schema an expression was written against,
     * and the answer the same expression has with a new version of the schema: the paths added,
     * removed and changed, and what changed in each. It needs no schema and no service: this
     * answer may be one read from the text kept beside the expression ({@link
     * #fromJson(String)}).
     *
     * <p>Types and enumerations are compared as sets: two answers that differ only in their
     * order have no change. A path is compared with the same path: a field that was renamed is
     * one path that became unknown, or was removed, and another that was added.
     *
     * @param current the answer with the new schema
     * @return the changes, empty when nothing the expression uses changed
     * @throws JlensException with {@code U2005} when an answer made by hand is not one the
     *     schema can give, such as a type that JSON Schema does not have
     */
    public UsesChanges changes(Uses current) {
        long old = Native.buffer(toJson());
        try {
            long other = Native.buffer(current.toJson());
            try {
                return UsesChanges.fromJson(Native.json(Native.usesChanges(old, other)));
            } finally {
                Native.free(other);
            }
        } finally {
            Native.free(old);
        }
    }

    /**
     * The answer of its JSON form.
     *
     * @param json {@code {"input": {path: Use}, "bindings": {name: {path: Use}},
     *     "everyBinding": true}}
     * @return the answer
     * @throws IllegalArgumentException when {@code json} is not in that form
     */
    public static Uses fromJson(String json) {
        Map<String, Object> members = Json.object(Json.given(json), "an answer");
        Json.only(members, "an answer", "input", "bindings", "everyBinding");
        Map<String, Map<String, Use>> bindings = new TreeMap<>();
        if (members.containsKey("bindings")) {
            Json.object(members.get("bindings"), "bindings")
                    .forEach((name, paths) -> bindings.put(name, Use.paths(paths)));
        }
        return new Uses(
                members.containsKey("input") ? Use.paths(members.get("input")) : null,
                bindings,
                Json.flag(members, "everyBinding"));
    }

    /**
     * The JSON form, the one every jlens package gives: to keep beside the expression. Compare
     * two answers with {@link #changes(Uses)} or {@code equals}, not by their text.
     *
     * @return {@code {"input": {path: Use}, "bindings": {name: {path: Use}}, "everyBinding":
     *     true}}, without {@code input} when the expression does not read it and without {@code
     *     everyBinding} when it is false; a {@link Use} has the members that are set, under the
     *     names of its components, the enumeration under {@code enum}
     */
    public String toJson() {
        Map<String, Object> members = new LinkedHashMap<>();
        if (input != null) members.put("input", Use.tree(input));
        Map<String, Object> byName = new LinkedHashMap<>();
        bindings.forEach((name, paths) -> byName.put(name, Use.tree(paths)));
        members.put("bindings", byName);
        if (everyBinding) members.put("everyBinding", true);
        return Json.write(members);
    }
}
