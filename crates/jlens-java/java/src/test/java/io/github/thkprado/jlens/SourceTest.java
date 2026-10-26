package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Date;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

/** Java objects as input, read one level at a time through the C API's reader (0.25 #6). */
class SourceTest {
    private static Object evaluate(String expression, Object input) {
        try (Expression compiled = Expression.compile(expression)) {
            return compiled.evaluate(input);
        }
    }

    private static String code(String expression, Object input) {
        return assertThrows(JlensException.class, () -> evaluate(expression, input)).code();
    }

    private static Map<String, Object> items() {
        Map<String, Object> pen = new LinkedHashMap<>();
        pen.put("name", "pen");
        pen.put("price", 1.5);
        pen.put("tags", List.of("a", "b"));
        Map<String, Object> ink = new HashMap<>();
        ink.put("name", "ink");
        ink.put("price", 4);
        ink.put("stock", null);
        return Map.of("items", List.of(pen, ink));
    }

    @Test
    void pathsIndexesAndFunctions() {
        Map<String, Object> input = items();
        assertEquals("pen", evaluate("items[0].name", input));
        assertEquals("ink", evaluate("($i := 1; items[$i].name)", input));
        assertEquals(5.5, evaluate("$sum(items.price)", input));
        assertEquals(List.of("ink"), evaluate("[items[price > 2].name]", input));
        assertEquals(2L, evaluate("$count(items)", input));
        assertEquals(List.of("a", "b"), evaluate("items.tags", input));
        assertNull(evaluate("items[1].stock", input), "null");
        assertNull(evaluate("items[1].missing", input), "undefined");
        assertEquals("{\"name\":\"pen\",\"price\":1.5,\"tags\":[\"a\",\"b\"]}",
                evaluate("$string(items[0])", input), "the map's order");
    }

    @Test
    void eachTypeIsReadAsItsJsonValue() {
        Map<String, Object> input = new LinkedHashMap<>();
        input.put("int", 1);
        input.put("long", 1L << 40);
        input.put("short", (short) -2);
        input.put("byte", (byte) 3);
        input.put("double", 0.25);
        input.put("float", 0.5f);
        input.put("big", new BigInteger("100000000000000000000"));
        input.put("decimal", new BigDecimal("1.75"));
        input.put("nan", Double.NaN);
        input.put("infinite", Float.NEGATIVE_INFINITY);
        input.put("string", "é😀");
        input.put("true", true);
        input.put("null", null);
        input.put("linked", new LinkedList<>(List.of(1, 2, 3)));
        String expected = "{\"int\":1,\"long\":1099511627776,\"short\":-2,\"byte\":3,\"double\":0.25,"
                + "\"float\":0.5,\"big\":100000000000000000000,\"decimal\":1.75,\"nan\":null,"
                + "\"infinite\":null,\"string\":\"é😀\",\"true\":true,\"null\":null,\"linked\":[1,2,3]}";
        assertEquals(expected, evaluate("$string($)", input));
        assertEquals(3L, evaluate("linked[2]", input), "a list that is not random access");
        assertEquals("x", evaluate("$", "x"), "a string is a JSON string");
        assertEquals(2L, evaluate("$ + 1", 1));
        assertNull(evaluate("$", null));
        assertEquals(Boolean.TRUE, evaluate("$not($)", false));
    }

    @Test
    void otherObjectsFailWithU1004() {
        assertEquals("U1004", code("$", new Date()));
        assertEquals("U1004", code("a", Map.of("a", new Object())));
        assertEquals("U1004", code("$string($)", Map.of(1, "one")), "a key that is not a string");
        assertEquals("U1004", code("$string($)", List.of(new int[] {1})));
        JlensException error = assertThrows(JlensException.class,
                () -> evaluate("a", Map.of("a", new StringBuilder("x"))));
        assertTrue(error.getMessage().contains("java.lang.StringBuilder"), error.getMessage());
        // Only what the evaluation reads is checked.
        assertEquals(1L, evaluate("a", Map.of("a", 1, "b", new Object())));
    }

    @Test
    void cyclesFailWithU1004() {
        Map<String, Object> cycle = new HashMap<>();
        cycle.put("self", cycle);
        List<Object> list = new ArrayList<>();
        list.add(list);
        assertEquals("U1004", code("$string($)", cycle));
        assertEquals("U1004", code("self.self", cycle));
        assertEquals("U1004", code("$[0][0]", list));
        assertEquals("U1004", code("$string($)", List.of(Map.of("l", list))));
        // A value met twice, not inside itself, is no cycle.
        List<Integer> shared = List.of(1);
        assertEquals("[[1],[1]]", evaluate("$string($)", List.of(shared, shared)));
        assertSame(cycle, evaluate("$", cycle), "returned as is, never read");
    }

    @Test
    void theInputUnchangedIsReturnedAsItself() {
        Map<String, Object> input = items();
        assertSame(input, evaluate("$", input));
        List<?> items = (List<?>) input.get("items");
        assertSame(items, evaluate("items", input));
        assertSame(items.get(0), evaluate("items[0]", input));
        assertSame(((Map<?, ?>) items.get(0)).get("tags"), evaluate("items[0].tags", input));
        List<?> filtered = (List<?>) evaluate("items[price > 0]", input);
        assertSame(items.get(0), filtered.get(0), "an array built, its items the input's");
        assertSame(items.get(1), filtered.get(1));
        Map<?, ?> copy = (Map<?, ?>) evaluate("items[0] ~> |$|{'name': 'x'}|", input);
        assertEquals("x", copy.get("name"));
        assertEquals("pen", ((Map<?, ?>) items.get(0)).get("name"), "the input is not modified");
        Map<String, Object> merged = new LinkedHashMap<>();
        merged.put("a", 1L);
        assertEquals(merged, evaluate("$merge([$])", Map.of("a", 1)), "a new map");
    }

    @Test
    void deepInputStopsWithAnErrorOnSmallStacks() throws Exception {
        String[] expressions = {"$string($)", "$count(**)", "$", "$.$[0][0][0]"};
        List<String> failures = new ArrayList<>();
        for (long stack : new long[] {128 << 10, 256 << 10, 1 << 20}) {
            for (int depth : new int[] {100, 2_000, 100_000}) {
                Object deep = 1;
                for (int i = 0; i < depth; i++) deep = List.of(deep);
                Object input = deep;
                for (String expression : expressions) {
                    AtomicReference<String> outcome = new AtomicReference<>();
                    AtomicReference<String> message = new AtomicReference<>("");
                    Thread thread = new Thread(null, () -> {
                        try {
                            Object result = evaluate(expression, input);
                            outcome.set(result == input ? "the input" : "a result");
                        } catch (JlensException e) {
                            outcome.set(e.code());
                            message.set(": " + e.getMessage());
                        } catch (Throwable e) {
                            outcome.set(e.toString());
                        }
                    }, "small stack", stack);
                    thread.start();
                    thread.join();
                    String got = outcome.get();
                    boolean expected;
                    if (expression.equals("$")) {
                        expected = got.equals("the input");
                    } else if (depth > 1_000 && expression.equals("$string($)")) {
                        expected = got.equals("U1001");
                    } else {
                        expected = got.equals("a result") || got.matches("[A-Z]\\d{4}");
                    }
                    if (!expected) {
                        failures.add(expression + ", stack " + stack + ", depth " + depth + ": "
                                + got + message.get());
                    }
                }
            }
        }
        // Every case is run, so that one failure does not hide the others.
        assertEquals(List.of(), failures);
    }

    @Test
    void virtualThreadsEachReadTheirOwnInput() throws Exception {
        try (Expression expression = Expression.compile("$sum(items.price) + items[0].n");
                ExecutorService executor = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Boolean>> results = new ArrayList<>();
            for (int t = 0; t < 200; t++) {
                int n = t;
                results.add(executor.submit(() -> {
                    Map<String, Object> input = Map.of("items",
                            List.of(Map.of("price", n, "n", n), Map.of("price", 1, "n", 0)));
                    for (int i = 0; i < 50; i++) {
                        if (!Long.valueOf(2L * n + 1).equals(expression.evaluate(input))) return false;
                    }
                    return true;
                }));
            }
            for (Future<Boolean> result : results) assertTrue(result.get());
        }
    }

    @Test
    void optionsAndADocumentGivenAsAnObject() {
        try (Expression expression = Expression.compile("[1..$]");
                Document document = Document.fromJson("3")) {
            Options small = Options.builder().maxArrayLength(2).build();
            assertEquals("U1001", assertThrows(JlensException.class,
                    () -> expression.evaluate((Object) 3, small)).code());
            assertEquals(List.of(1L, 2L, 3L), expression.evaluate((Object) document));
        }
    }

    // ---- Property: Java objects give what their JSON text gives -------------------------------

    private static final String[] EXPRESSIONS = {
        "$", "a", "a.b", "*", "**", "$.*[0]", "a[0]", "a[-1].b", "($i := 1; a[$i])", "$count(a)",
        "$string($)", "$keys($)", "[$]", "{'v': a}", "$ ~> | $ | {'x': 1} |", "a[b = 1]",
        "$sort(a)", "a[b > 0].b", "$type(a)",
    };

    @Test
    void javaObjectsGiveWhatTheirJsonTextGives() {
        Random random = new Random(6);
        for (int round = 0; round < 2_000; round++) {
            Object input = value(random, 4);
            String text = json(input);
            for (String source : EXPRESSIONS) {
                try (Expression expression = Expression.compile(source)) {
                    String expected;
                    try {
                        expected = json(expression.evaluateFromJson(text));
                    } catch (JlensException e) {
                        expected = e.code();
                    }
                    String got;
                    try {
                        got = json(expression.evaluate(input));
                    } catch (JlensException e) {
                        got = e.code();
                    }
                    assertEquals(expected, got, source + " on " + text);
                }
            }
        }
    }

    private static final String[] KEYS = {"a", "b", "x", "é", "a b", ""};

    /** A random value of each kind the reader reads, nested up to {@code depth} levels. */
    private static Object value(Random random, int depth) {
        int kind = random.nextInt(depth > 0 ? 9 : 6);
        return switch (kind) {
            case 0 -> null;
            case 1 -> random.nextBoolean();
            case 2 -> random.nextInt(5) - 1;
            case 3 -> switch (random.nextInt(4)) {
                case 0 -> random.nextLong();
                case 1 -> 0.5 * random.nextInt(9);
                case 2 -> new BigDecimal("1e30");
                default -> random.nextDouble() * 1e6;
            };
            case 4 -> List.of("", "s", "é😀", "a\"b").get(random.nextInt(4));
            case 5 -> (float) random.nextInt(4);
            case 6, 7 -> {
                Map<String, Object> map = random.nextBoolean() ? new LinkedHashMap<>() : new HashMap<>();
                int n = random.nextInt(4);
                for (int i = 0; i < n; i++) map.put(KEYS[random.nextInt(KEYS.length)], value(random, depth - 1));
                yield map;
            }
            default -> {
                int n = random.nextInt(4);
                Object[] items = new Object[n];
                for (int i = 0; i < n; i++) items[i] = value(random, depth - 1);
                yield random.nextBoolean() ? Arrays.asList(items) : new ArrayList<>(Arrays.asList(items));
            }
        };
    }

    /** The JSON text of a value, numbers as jlens writes them, so that 2 and 2.0 are equal. */
    private static String json(Object value) {
        StringBuilder out = new StringBuilder();
        json(value, out);
        return out.toString();
    }

    private static void json(Object value, StringBuilder out) {
        switch (value) {
            case null -> out.append("null");
            case String s -> out.append(Json.quote(s));
            case Number n -> {
                double d = n.doubleValue();
                if (!Double.isFinite(d)) out.append("null");
                else if (d == Math.rint(d) && Math.abs(d) < 1e18) out.append((long) d);
                else out.append(d);
            }
            case Map<?, ?> map -> {
                out.append('{');
                String comma = "";
                for (Map.Entry<?, ?> field : map.entrySet()) {
                    out.append(comma).append(Json.quote((String) field.getKey())).append(':');
                    json(field.getValue(), out);
                    comma = ",";
                }
                out.append('}');
            }
            case List<?> list -> {
                out.append('[');
                String comma = "";
                for (Object item : list) {
                    out.append(comma);
                    json(item, out);
                    comma = ",";
                }
                out.append(']');
            }
            default -> out.append(value);
        }
    }
}
