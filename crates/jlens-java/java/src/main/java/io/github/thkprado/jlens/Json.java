package io.github.thkprado.jlens;

import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The JSON the package reads and writes itself: error envelopes, reads and options. Results are
 * the caller's JSON text, which this class never parses, or values ({@link Values}). The text comes from the C API, so a
 * malformed one is a bug, reported as an {@link IllegalStateException}.
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

    private Object value() {
        space();
        return switch (peek()) {
            case '{' -> object();
            case '[' -> array();
            case '"' -> string();
            case 't' -> word("true", Boolean.TRUE);
            case 'f' -> word("false", Boolean.FALSE);
            case 'n' -> word("null", null);
            default -> number();
        };
    }

    private Map<String, Object> object() {
        Map<String, Object> object = new LinkedHashMap<>();
        expect('{');
        space();
        if (peek() == '}') {
            at++;
            return object;
        }
        do {
            space();
            String key = string();
            space();
            expect(':');
            object.put(key, value());
            space();
        } while (next() == ',');
        at--;
        expect('}');
        return object;
    }

    private List<Object> array() {
        List<Object> array = new ArrayList<>();
        expect('[');
        space();
        if (peek() == ']') {
            at++;
            return array;
        }
        do {
            array.add(value());
            space();
        } while (next() == ',');
        at--;
        expect(']');
        return array;
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
