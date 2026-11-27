package io.github.thkprado.jlens;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The JSON the package reads and writes itself: error envelopes, reads, options, diagnostics
 * and what an expression uses of its schema. Results are
 * the caller's JSON text, which this class never parses, or values ({@link Values}). The text comes from the C API, so a
 * malformed one is a bug, reported as an {@link IllegalStateException}. Values are read and
 * written with a stack of their own, never by recursion, as they nest as deep as the C API's
 * (1,000 levels) on a thread with a small stack (roadmap 0.37 #4).
 */
final class Json {
    private final String text;
    private int at;

    private Json(String text) {
        this.text = text;
    }

    /**
     * Parses {@code text}: objects as {@link LinkedHashMap}s, arrays as {@link List}s, numbers as
     * {@link BigDecimal}s, and {@code null} as {@code null}.
     */
    static Object parse(String text) {
        Json json = new Json(text);
        Object value = json.value();
        json.space();
        if (json.at != text.length()) throw json.malformed();
        return value;
    }

    /** The members of the object {@code text}, each as its JSON text. */
    static Map<String, String> members(String text) {
        Json json = new Json(text);
        Map<String, String> members = new LinkedHashMap<>();
        json.expect('{');
        json.space();
        if (json.peek() == '}') {
            json.at++;
            return members;
        }
        do {
            json.space();
            String key = json.string();
            json.space();
            json.expect(':');
            json.space();
            int start = json.at;
            json.value();
            members.put(key, text.substring(start, json.at));
            json.space();
        } while (json.next() == ',');
        json.at--;
        json.expect('}');
        return members;
    }

    /**
     * Parses {@code text}, which the application gives: text that is not JSON is its mistake, an
     * {@link IllegalArgumentException}.
     */
    static Object given(String text) {
        try {
            return parse(text);
        } catch (IllegalStateException e) {
            throw new IllegalArgumentException("not JSON: " + text, e);
        }
    }

    /** {@code value}, which must be an object, as a map; {@code what} names it in the failure. */
    @SuppressWarnings("unchecked")
    static Map<String, Object> object(Object value, String what) {
        if (value instanceof Map<?, ?> map) return (Map<String, Object>) map;
        throw new IllegalArgumentException(what + " must be an object, not " + value);
    }

    /** Fails when {@code object} has a member that is not one of {@code names}. */
    static void only(Map<String, Object> object, String what, String... names) {
        for (String name : object.keySet()) {
            if (!List.of(names).contains(name)) {
                throw new IllegalArgumentException(what + " has no member " + name);
            }
        }
    }

    /** Member {@code name} of {@code object}, a boolean; false when it is absent. */
    static boolean flag(Map<String, Object> object, String name) {
        Boolean flag = member(object, name, Boolean.class);
        return flag != null && flag;
    }

    /**
     * Member {@code name} of {@code object} as a {@code type}, {@code null} when it is absent.
     */
    static <T> T member(Map<String, Object> object, String name, Class<T> type) {
        if (!object.containsKey(name)) return null;
        Object value = object.get(name);
        if (!type.isInstance(value)) {
            throw new IllegalArgumentException(name + " has the wrong type: " + value);
        }
        return type.cast(value);
    }

    /**
     * A value as {@link #parse(String)} gives it, as compact JSON text; a map's members in its
     * order.
     */
    static String write(Object value) {
        StringBuilder out = new StringBuilder();
        // What is left to write, the next last: values, and the text between them.
        List<Object> left = new ArrayList<>();
        left.add(value);
        while (!left.isEmpty()) {
            switch (left.removeLast()) {
                case null -> out.append("null");
                case Text(String text) -> out.append(text);
                case String text -> out.append(quote(text));
                case Boolean flag -> out.append(flag);
                case Number number -> out.append(number);
                case Map<?, ?> map -> {
                    out.append('{');
                    left.add(new Text("}"));
                    List<Map.Entry<?, ?>> members = new ArrayList<>(map.entrySet());
                    for (int i = members.size() - 1; i >= 0; i--) {
                        left.add(members.get(i).getValue());
                        left.add(new Text((i > 0 ? "," : "")
                                + quote(String.valueOf(members.get(i).getKey())) + ":"));
                    }
                }
                case List<?> list -> {
                    out.append('[');
                    left.add(new Text("]"));
                    for (int i = list.size() - 1; i >= 0; i--) {
                        left.add(list.get(i));
                        if (i > 0) left.add(new Text(","));
                    }
                }
                case Object other -> throw new IllegalArgumentException("not a JSON value: " + other);
            }
        }
        return out.toString();
    }

    /** Text {@link #write(Object)} writes as it is. */
    private record Text(String text) {}

    /** {@code value} as a JSON string. */
    static String quote(String value) {
        StringBuilder out = new StringBuilder(value.length() + 2).append('"');
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
                    else out.append(c);
                }
            }
        }
        return out.append('"').toString();
    }

    /** The value at {@code at}, its arrays and objects read with a stack of their own. */
    @SuppressWarnings("unchecked")
    private Object value() {
        // The arrays and objects open, the innermost last, and for an object the key of its
        // next member (null for an array).
        List<Object> open = new ArrayList<>();
        List<String> keys = new ArrayList<>();
        while (true) {
            space();
            char first = peek();
            Object value;
            if (first == '{' || first == '[') {
                at++;
                space();
                boolean object = first == '{';
                Object made = object ? new LinkedHashMap<String, Object>() : new ArrayList<Object>();
                if (peek() != (object ? '}' : ']')) {
                    open.add(made);
                    keys.add(object ? key() : null);
                    continue;
                }
                at++;
                value = made;
            } else {
                value = scalar(first);
            }
            // Adds the value to the innermost array or object, closing those that end with it.
            while (true) {
                if (open.isEmpty()) return value;
                Object container = open.getLast();
                boolean object = container instanceof Map;
                if (object) ((Map<String, Object>) container).put(keys.getLast(), value);
                else ((List<Object>) container).add(value);
                space();
                char next = next();
                if (next == ',') {
                    if (object) keys.set(keys.size() - 1, key());
                    break;
                }
                if (next != (object ? '}' : ']')) throw malformed();
                open.removeLast();
                keys.removeLast();
                value = container;
            }
        }
    }

    /** A member's key and its colon. */
    private String key() {
        space();
        String key = string();
        space();
        expect(':');
        return key;
    }

    private Object scalar(char first) {
        return switch (first) {
            case '"' -> string();
            case 't' -> word("true", Boolean.TRUE);
            case 'f' -> word("false", Boolean.FALSE);
            case 'n' -> word("null", null);
            default -> number();
        };
    }

    private String string() {
        expect('"');
        StringBuilder out = new StringBuilder();
        while (true) {
            char c = next();
            if (c == '"') return out.toString();
            if (c != '\\') {
                out.append(c);
                continue;
            }
            switch (next()) {
                case '"' -> out.append('"');
                case '\\' -> out.append('\\');
                case '/' -> out.append('/');
                case 'b' -> out.append('\b');
                case 'f' -> out.append('\f');
                case 'n' -> out.append('\n');
                case 'r' -> out.append('\r');
                case 't' -> out.append('\t');
                case 'u' -> {
                    if (at + 4 > text.length()) throw malformed();
                    try {
                        out.append((char) Integer.parseInt(text, at, at + 4, 16));
                    } catch (NumberFormatException e) {
                        throw malformed();
                    }
                    at += 4;
                }
                default -> throw malformed();
            }
        }
    }

    private BigDecimal number() {
        int start = at;
        while (at < text.length() && "+-0123456789.eE".indexOf(text.charAt(at)) >= 0) at++;
        try {
            return new BigDecimal(text.substring(start, at));
        } catch (NumberFormatException e) {
            throw malformed();
        }
    }

    private Object word(String word, Object value) {
        if (!text.startsWith(word, at)) throw malformed();
        at += word.length();
        return value;
    }

    private void space() {
        while (at < text.length() && " \t\n\r".indexOf(text.charAt(at)) >= 0) at++;
    }

    private char peek() {
        if (at >= text.length()) throw malformed();
        return text.charAt(at);
    }

    private char next() {
        char c = peek();
        at++;
        return c;
    }

    private void expect(char c) {
        if (next() != c) throw malformed();
    }

    private IllegalStateException malformed() {
        return new IllegalStateException("malformed JSON from the C API at " + at + ": " + text);
    }
}
