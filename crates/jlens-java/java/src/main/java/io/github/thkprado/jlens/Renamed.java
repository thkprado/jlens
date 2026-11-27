package io.github.thkprado.jlens;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * An expression with fields of its input renamed, as {@link LanguageService#rename(Expression,
 * Map)} gives it.
 *
 * @param text the rewritten text: the expression's own, with the edits made
 * @param edits each name replaced, in the order of the text
 * @param unsure the places the rename cannot be sure of, in the order of the text, where the
 *     text is left as it is: a field renamed there may be read where no step names it, or
 *     named as data
 */
public record Renamed(String text, List<Edit> edits, List<Unsure> unsure) {
    /** Copies the lists. */
    public Renamed {
        edits = List.copyOf(edits);
        unsure = List.copyOf(unsure);
    }

    /**
     * A name replaced.
     *
     * @param span the name in the expression's source, with its backquotes
     * @param text what replaces it: the new name, between backquotes when it is not a plain one
     */
    public record Edit(Span span, String text) {}

    /**
     * A place the rename cannot be sure of.
     *
     * @param span the operand in the expression's source; the whole text for {@link
     *     Reason#LOST}
     * @param path the path concerned, as {@link Uses} writes it: the value used whole, or the
     *     renamed field; {@code $} for {@link Reason#LOST}
     * @param reason why
     */
    public record Unsure(Span span, String path, Reason reason) {}

    /** Why a rename cannot be sure of a place. */
    public enum Reason {
        /**
         * A value that holds a renamed field is used whole: given to a function, which may read
         * the field or its name ({@code $keys}, {@code $sift}, {@code $string}, a function of the
         * expression or the application), iterated by {@code *} or {@code **}, an operand.
         */
        WHOLE,
        /** A renamed field is named by a string, given to {@code $lookup}. */
        STRING,
        /** The step reads a renamed field and another one: no single name fits both. */
        MIXED,
        /** The analysis gave up on part of the expression ({@code $eval}...). */
        LOST
    }

    /** The answer of the JSON object the C API replies with. */
    static Renamed fromJson(String json) {
        String what = "a rename";
        Map<String, Object> members = Json.object(Json.parse(json), what);
        Json.only(members, what, "text", "edits", "unsure");
        List<Edit> edits = new ArrayList<>();
        for (Object item : list(members, "edits")) {
            Map<String, Object> edit = Json.object(item, "an edit");
            edits.add(new Edit(span(edit), Json.member(edit, "text", String.class)));
        }
        List<Unsure> unsure = new ArrayList<>();
        for (Object item : list(members, "unsure")) {
            Map<String, Object> place = Json.object(item, "an unsure place");
            Reason reason = switch (Json.member(place, "reason", String.class)) {
                case "whole" -> Reason.WHOLE;
                case "string" -> Reason.STRING;
                case "mixed" -> Reason.MIXED;
                case "lost" -> Reason.LOST;
                case null, default -> throw new IllegalStateException(
                        "jlens: unexpected rename from the C API: " + json);
            };
            unsure.add(new Unsure(span(place), Json.member(place, "path", String.class), reason));
        }
        return new Renamed(Json.member(members, "text", String.class), edits, unsure);
    }

    private static List<?> list(Map<String, Object> members, String name) {
        List<?> list = Json.member(members, name, List.class);
        return list == null ? List.of() : list;
    }

    private static Span span(Map<String, Object> item) {
        Map<String, Object> span = Json.object(item.get("span"), "a span");
        return new Span(
                Json.member(span, "start", BigDecimal.class).intValueExact(),
                Json.member(span, "end", BigDecimal.class).intValueExact());
    }
}
