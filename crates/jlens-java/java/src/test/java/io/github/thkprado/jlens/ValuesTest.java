package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

/** Results as Java objects (0.25 #5). */
class ValuesTest {
    private static Object evaluate(String expression) {
        try (Expression compiled = Expression.compile(expression)) {
            return compiled.evaluate();
        }
    }

    @Test
    void scalars() {
        assertNull(evaluate("null"));
        assertNull(evaluate("$nothing"), "undefined is null too");
        assertSame(Boolean.TRUE, evaluate("true"));
        assertSame(Boolean.FALSE, evaluate("1 > 2"));
        assertEquals("é😀 ☕", evaluate("\"é😀 ☕\""));
        assertEquals("", evaluate("\"\""));
    }

    @Test
    void integralNumbersBelow2To63AreLongs() {
        assertEquals(5L, evaluate("2 + 3"));
        assertEquals(-3L, evaluate("-3"));
        assertEquals(0L, evaluate("-0"));
        assertEquals(9007199254740992L, evaluate("9007199254740992"));
        assertEquals(-9223372036854775808L, evaluate("-9223372036854775808"));
        assertEquals(1.5, evaluate("3 / 2"));
        assertEquals(Math.pow(2, 63), evaluate("9223372036854775808"), "2^63 is a Double");
        assertEquals(1e20, evaluate("1e20"));
        assertEquals(1e300, evaluate("1e300"));
        assertInstanceOf(Double.class, evaluate("$sum([0.5, 0.5]) / 2"));
        assertInstanceOf(Long.class, evaluate("$sum([0.5, 0.5])"));
    }

    @Test
    void arraysAndObjects() {
        Object result = evaluate(
                "{\"b\": [1, \"x\", null, [], {}], \"a\": {\"deep\": [true]}, \"skip\": $nothing}");
        Map<?, ?> map = assertInstanceOf(LinkedHashMap.class, result);
        assertEquals(List.of("b", "a"), new ArrayList<>(map.keySet()), "in order, undefined left out");
        List<?> b = assertInstanceOf(ArrayList.class, map.get("b"));
        List<Object> expected = new ArrayList<>(List.of(1L, "x"));
        expected.add(null);
        expected.add(List.of());
        expected.add(Map.of());
        assertEquals(expected, b);
        assertEquals(Map.of("deep", List.of(true)), map.get("a"));
        assertEquals(List.of(), evaluate("[]"));
        assertEquals(Map.of(), evaluate("{}"));
    }

    @Test
    void resultsCanBeModified() {
        @SuppressWarnings("unchecked")
        Map<String, Object> map = (Map<String, Object>) evaluate("{\"a\": [1]}");
        map.put("b", 2L);
        @SuppressWarnings("unchecked")
        List<Object> list = (List<Object>) map.get("a");
        list.add(3L);
        assertEquals(Map.of("a", List.of(1L, 3L), "b", 2L), map);
    }

    @Test
    void keysRepeatedAcrossObjects() {
        try (Expression expression = Expression.compile("items.{\"name\": name, \"twice\": price * 2}");
                Document document = Document.fromJson(
                        "{\"items\": [{\"name\": \"a\", \"price\": 1.25}, {\"name\": \"ü\", \"price\": 2}]}")) {
            Object result = expression.evaluate(document);
            assertEquals(List.of(Map.of("name", "a", "twice", 2.5), Map.of("name", "ü", "twice", 4L)), result);
            List<?> rows = (List<?>) result;
            Object first = ((Map<?, ?>) rows.get(0)).keySet().iterator().next();
            Object second = ((Map<?, ?>) rows.get(1)).keySet().iterator().next();
            assertSame(first, second, "one String per key");
        }
    }

    @Test
    void fromJsonText() {
        try (Expression expression = Expression.compile("items[price > 1].name")) {
            String input = "{\"items\": [{\"name\": \"a\", \"price\": 1}, {\"name\": \"b\", \"price\": 2}]}";
            assertEquals("b", expression.evaluateFromJson(input));
            assertEquals("b", expression.evaluateFromJson(input.getBytes(StandardCharsets.UTF_8)));
            assertNull(expression.evaluateFromJson("{}"));
            assertNull(expression.evaluateFromJson((String) null));
            assertEquals("U2001", assertThrows(JlensException.class,
                    () -> expression.evaluateFromJson("{items")).code());
        }
    }

    @Test
    void theLimitsApply() {
        try (Expression expression = Expression.compile("[1..100]")) {
            Options small = Options.builder().maxResultSize(10).build();
            assertEquals("U1001", assertThrows(JlensException.class,
                    () -> expression.evaluateFromJson((String) null, small)).code());
            assertEquals(100, ((List<?>) expression.evaluate()).size());
        }
        assertEquals("T2001", assertThrows(JlensException.class, () -> evaluate("\"a\" + 1")).code());
    }

    @Test
    void deepResultsOnASmallStack() throws Exception {
        // Near the deepest result the core writes (1,000 levels), decoded with a stack of its own.
        String deep = "($f := function($n) { $n = 0 ? 0 : {\"a\": $f($n - 1)} }; $f(990))";
        AtomicReference<Object> outcome = new AtomicReference<>();
        Thread thread = new Thread(null, () -> {
            try (Expression expression = Expression.compile(deep)) {
                outcome.set(expression.evaluate());
            } catch (Throwable e) {
                outcome.set(e);
            }
        }, "small stack", 128 << 10);
        thread.start();
        thread.join();
        Object value = outcome.get();
        int depth = 0;
        while (value instanceof Map<?, ?> map) {
            value = map.get("a");
            depth++;
        }
        assertEquals(0L, value);
        assertEquals(990, depth);
    }
}
