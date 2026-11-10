package io.github.thkprado.jlens;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.function.Function;

/**
 * What changed in what an expression uses between two schemas, given by {@link
 * Uses#changes(Uses)}.
 *
 * {@snippet :
 * Uses kept = Uses.fromJson(stored);                    // the answer with the old schema
 * UsesChanges changes = kept.changes(service.uses(expression));
 * if (!changes.isEmpty()) {
 *     changes.input().changed().forEach((path, change) -> {
 *         // "active": its types changed from [boolean] to [string]
 *     });
 * }
 * }
 *
 * @param input what changed in what the expression uses of its input; empty when nothing did
 * @param bindings what changed in what it uses of each variable, by name; only the variables
 *     with a change. A variable that only one answer has comes with all its paths added or
 *     removed
 * @param everyBinding {@link Uses#everyBinding()}, when the two answers differ in it, which
 *     those of one expression do not; {@code null} otherwise
 */
public record UsesChanges(
        PathChanges input, Map<String, PathChanges> bindings, Change<Boolean> everyBinding) {
    /** Copies {@code bindings}, sorted by name. */
    public UsesChanges {
        bindings = Collections.unmodifiableSortedMap(new TreeMap<>(bindings));
    }

    /**
     * What changed in the paths an expression reads of one value.
     *
     * @param added the paths that only the new answer has, with what the new schema says
     *     there: a field added under a value the expression reads whole
     * @param removed the paths that only the old answer has, with what the old schema said
     *     there
     * @param changed the paths that both answers have, and of which the schemas do not say the
     *     same
     */
    public record PathChanges(
            Map<String, Use> added, Map<String, Use> removed, Map<String, UseChanges> changed) {
        /** Copies the maps, sorted by path. */
        public PathChanges {
            added = Collections.unmodifiableSortedMap(new TreeMap<>(added));
            removed = Collections.unmodifiableSortedMap(new TreeMap<>(removed));
            changed = Collections.unmodifiableSortedMap(new TreeMap<>(changed));
        }

        /** {@return whether nothing changed} */
        public boolean isEmpty() {
            return added.isEmpty() && removed.isEmpty() && changed.isEmpty();
        }

        private static PathChanges of(Object value) {
            if (value == null) return new PathChanges(Map.of(), Map.of(), Map.of());
            Map<String, Object> members = Json.object(value, "the changes of a value");
            Json.only(members, "the changes of a value", "added", "removed", "changed");
            Map<String, UseChanges> changed = new TreeMap<>();
            if (members.containsKey("changed")) {
                Json.object(members.get("changed"), "changed")
                        .forEach((path, change) -> changed.put(path, UseChanges.of(change)));
            }
            return new PathChanges(
                    members.containsKey("added") ? Use.paths(members.get("added")) : Map.of(),
                    members.containsKey("removed") ? Use.paths(members.get("removed")) : Map.of(),
                    changed);
        }

        private Map<String, Object> tree() {
            Map<String, Object> members = new LinkedHashMap<>();
            if (!added.isEmpty()) members.put("added", Use.tree(added));
            if (!removed.isEmpty()) members.put("removed", Use.tree(removed));
            if (!changed.isEmpty()) {
                Map<String, Object> byPath = new LinkedHashMap<>();
                changed.forEach((path, change) -> byPath.put(path, change.tree()));
                members.put("changed", byPath);
            }
            return members;
        }
    }

    /**
     * What changed in what the schemas say of one path: a component for each one of {@link
     * Use} that changed, {@code null} for one that did not. A field that the new schema no
     * longer declares has {@code unknown} changed, with every component the old schema gave
     * it.
     *
     * @param unknown {@link Use#unknown()}: the schema started or stopped declaring the field
     * @param whole {@link Use#whole()}, which the two answers of one expression do not differ
     *     in
     * @param types {@link Use#types()}, compared as sets; a side that gives no type is {@code
     *     null}
     * @param enumeration {@link Use#enumeration()}, compared as a set
     * @param closed {@link Use#closed()}
     * @param domain {@link Use#domain()}
     * @param format {@link Use#format()}
     * @param required {@link Use#required()}
     */
    public record UseChanges(
            Change<Boolean> unknown,
            Change<Boolean> whole,
            Change<List<String>> types,
            EnumChange enumeration,
            Change<Boolean> closed,
            Change<String> domain,
            Change<String> format,
            Change<Boolean> required) {
        private static UseChanges of(Object value) {
            Map<String, Object> members = Json.object(value, "the changes of a path");
            Json.only(members, "the changes of a path", "unknown", "whole", "types", "enum",
                    "closed", "domain", "format", "required");
            Function<Object, Boolean> flag = side -> cast(side, Boolean.class);
            Function<Object, String> text = side -> cast(side, String.class);
            return new UseChanges(
                    Change.of(members.get("unknown"), flag),
                    Change.of(members.get("whole"), flag),
                    Change.of(members.get("types"), side -> {
                        List<String> types = new ArrayList<>();
                        for (Object type : cast(side, List.class)) {
                            types.add(cast(type, String.class));
                        }
                        return List.copyOf(types);
                    }),
                    EnumChange.of(members.get("enum")),
                    Change.of(members.get("closed"), flag),
                    Change.of(members.get("domain"), text),
                    Change.of(members.get("format"), text),
                    Change.of(members.get("required"), flag));
        }

        private Map<String, Object> tree() {
            Map<String, Object> members = new LinkedHashMap<>();
            if (unknown != null) members.put("unknown", unknown.tree());
            if (whole != null) members.put("whole", whole.tree());
            if (types != null) members.put("types", types.tree());
            if (enumeration != null) members.put("enum", enumeration.tree());
            if (closed != null) members.put("closed", closed.tree());
            if (domain != null) members.put("domain", domain.tree());
            if (format != null) members.put("format", format.tree());
            if (required != null) members.put("required", required.tree());
            return members;
        }
    }

    /**
     * A component that changed: what the old schema said and what the new one says. In the JSON
     * form they are {@code old} and {@code new}.
     *
     * @param before what the old schema said, {@code null} when it said nothing
     * @param after what the new schema says, {@code null} when it says nothing
     * @param <T> the component's type
     */
    public record Change<T>(T before, T after) {
        private static <T> Change<T> of(Object value, Function<Object, T> side) {
            if (value == null) return null;
            Map<String, Object> members = Json.object(value, "a change");
            Json.only(members, "a change", "old", "new");
            return new Change<>(
                    members.containsKey("old") ? side.apply(members.get("old")) : null,
                    members.containsKey("new") ? side.apply(members.get("new")) : null);
        }

        private Map<String, Object> tree() {
            Map<String, Object> members = new LinkedHashMap<>();
            if (before != null) members.put("old", before);
            if (after != null) members.put("new", after);
            return members;
        }
    }

    /**
     * An enumeration that changed. The two are compared as sets: the order of the values and a
     * value listed twice change nothing. A value is a JSON value, as in {@link
     * Use#enumeration()}.
     *
     * @param before the values the old schema listed, {@code null} when it had no enumeration
     * @param after the values the new schema lists, {@code null} when it has no enumeration
     * @param added the values that only the new schema lists, in its order
     * @param removed the values that only the old schema listed, in its order
     */
    public record EnumChange(
            List<Object> before, List<Object> after, List<Object> added, List<Object> removed) {
        /** Copies the lists. */
        public EnumChange {
            // An enumeration may list `null`, which `List.copyOf` refuses.
            if (before != null) before = Collections.unmodifiableList(new ArrayList<>(before));
            if (after != null) after = Collections.unmodifiableList(new ArrayList<>(after));
            added = Collections.unmodifiableList(new ArrayList<>(added));
            removed = Collections.unmodifiableList(new ArrayList<>(removed));
        }

        private static EnumChange of(Object value) {
            if (value == null) return null;
            Map<String, Object> members = Json.object(value, "an enumeration's change");
            Json.only(members, "an enumeration's change", "old", "new", "added", "removed");
            List<?> before = Json.member(members, "old", List.class);
            List<?> after = Json.member(members, "new", List.class);
            List<?> added = Json.member(members, "added", List.class);
            List<?> removed = Json.member(members, "removed", List.class);
            return new EnumChange(
                    before == null ? null : new ArrayList<>(before),
                    after == null ? null : new ArrayList<>(after),
                    added == null ? List.of() : new ArrayList<>(added),
                    removed == null ? List.of() : new ArrayList<>(removed));
        }

        private Map<String, Object> tree() {
            Map<String, Object> members = new LinkedHashMap<>();
            if (before != null) members.put("old", before);
            if (after != null) members.put("new", after);
            if (!added.isEmpty()) members.put("added", added);
            if (!removed.isEmpty()) members.put("removed", removed);
            return members;
        }
    }

    /** {@return whether nothing the expression uses changed} */
    public boolean isEmpty() {
        return input.isEmpty() && bindings.isEmpty() && everyBinding == null;
    }

    /**
     * The changes of their JSON form.
     *
     * @param json {@code {"input": PathChanges, "bindings": {name: PathChanges},
     *     "everyBinding": Change}}
     * @return the changes
     * @throws IllegalArgumentException when {@code json} is not in that form
     */
    public static UsesChanges fromJson(String json) {
        Map<String, Object> members = Json.object(Json.given(json), "the changes");
        Json.only(members, "the changes", "input", "bindings", "everyBinding");
        Map<String, PathChanges> bindings = new TreeMap<>();
        if (members.containsKey("bindings")) {
            Json.object(members.get("bindings"), "bindings")
                    .forEach((name, changes) -> bindings.put(name, PathChanges.of(changes)));
        }
        return new UsesChanges(
                PathChanges.of(members.get("input")),
                bindings,
                Change.of(members.get("everyBinding"), side -> cast(side, Boolean.class)));
    }

    /**
     * The JSON form, the one every jlens package gives: to show to who reviews the expression.
     *
     * @return {@code {"input": PathChanges, "bindings": {name: PathChanges}, "everyBinding":
     *     Change}}, each member left out when nothing changed there, so that <code>{}</code>
     *     says that nothing did. A {@code PathChanges} has {@code added} and {@code removed},
     *     from paths to a {@link Use}, and {@code changed}, from paths to the components that
     *     changed, each as {@code {"old": …, "new": …}}; an enumeration also has the values
     *     {@code added} and {@code removed}
     */
    public String toJson() {
        Map<String, Object> members = new LinkedHashMap<>();
        if (!input.isEmpty()) members.put("input", input.tree());
        if (!bindings.isEmpty()) {
            Map<String, Object> byName = new LinkedHashMap<>();
            bindings.forEach((name, changes) -> byName.put(name, changes.tree()));
            members.put("bindings", byName);
        }
        if (everyBinding != null) members.put("everyBinding", everyBinding.tree());
        return Json.write(members);
    }

    private static <T> T cast(Object value, Class<T> type) {
        if (!type.isInstance(value)) {
            throw new IllegalArgumentException("unexpected value in the changes: " + value);
        }
        return type.cast(value);
    }
}
