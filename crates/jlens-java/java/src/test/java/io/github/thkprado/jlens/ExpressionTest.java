package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.OptionalInt;
import org.junit.jupiter.api.Test;

class ExpressionTest {
    private static String evaluate(String expression, String input) {
        try (Expression compiled = Expression.compile(expression)) {
            return compiled.evaluateJson(input);
        }
    }

    private static JlensException failure(String expression, String input) {
        return assertThrows(JlensException.class, () -> evaluate(expression, input));
    }

    @Test
    void evaluatesJsonText() {
        String input = "{\"name\":\"café ☕\",\"items\":[{\"price\":2},{\"price\":3.5}]}";
        assertEquals("5.5", evaluate("$sum(items.price)", input));
        assertEquals("\"CAFÉ ☕\"", evaluate("$uppercase(name)", input));
        assertEquals("[1,2]", evaluate("[1, 2]", null));
        try (Expression expression = Expression.compile("$string(\"é😀\")")) {
            assertEquals("\"é😀\"", expression.evaluateJson());
            assertEquals("$string(\"é😀\")", expression.source());
        }
    }

    @Test
    void undefinedIsNullAndNullIsJson() {
        assertNull(evaluate("missing", "{}"));
        assertEquals("null", evaluate("a", "{\"a\":null}"));
    }

    @Test
    void readsUtf8Bytes() {
        try (Expression expression = Expression.compile("a & '!'")) {
            byte[] input = "{\"a\":\"ü\"}".getBytes(StandardCharsets.UTF_8);
            assertEquals("\"ü!\"", expression.evaluateJson(input));
            JlensException error = assertThrows(JlensException.class,
                    () -> expression.evaluateJson(new byte[] {'"', (byte) 0xff, '"'}));
            assertEquals("U2001", error.code());
        }
    }

    @Test
    void compileErrorsCarryUtf16Offsets() {
        JlensException error = assertThrows(JlensException.class, () -> Expression.compile("\"😀\" = )"));
        assertEquals("S0211", error.code());
        assertEquals(OptionalInt.of(8), error.position());
        assertEquals(OptionalInt.of(7), error.start());
        assertEquals(OptionalInt.of(8), error.end());
        assertEquals(")", error.token().orElseThrow());
        assertTrue(error.toString().contains("S0211: "), error.toString());
        assertTrue(!error.getMessage().startsWith("S0211"), error.getMessage());
    }

    @Test
    void evaluationErrorsCarryTheirValue() {
        JlensException error = failure("\"a😀\" + 1", null);
        assertEquals("T2001", error.code());
        assertEquals("\"a😀\"", error.valueJson().orElseThrow());
        assertEquals(OptionalInt.of(0), error.start());
        assertEquals(OptionalInt.of(9), error.end(), "UTF-16, not the 11 bytes of UTF-8");
    }

    @Test
    void inputThatIsNotJsonFails() {
        assertEquals("U2001", failure("a", "{a:1}").code());
    }

    @Test
    void closedExpressionsThrowAndCloseTwice() {
        Expression expression = Expression.compile("1");
        expression.close();
        expression.close();
        assertThrows(IllegalStateException.class, expression::evaluateJson);
        assertThrows(IllegalStateException.class, expression::reads);
    }

    @Test
    void manyExpressionsLeftToTheCleaner() {
        // Not closed: the Cleaner frees them, and the table of handles keeps working.
        for (int i = 0; i < 10_000; i++) {
            assertEquals(Integer.toString(i), Expression.compile(Integer.toString(i)).evaluateJson());
        }
    }

    /**
     * Replies around the 4,096 bytes of the scratch buffer the C API replies into, which a larger
     * reply does not use (roadmap 0.26 #10), from every input.
     */
    @Test
    void repliesOfEverySize() {
        try (Expression pad = Expression.compile("$pad('', n, 'x')");
                Document small = Document.fromJson("{\"n\": 1}")) {
            assertEquals("x", pad.evaluate(small));
            for (int n = 4_080; n <= 4_100; n++) {
                String expected = "x".repeat(n);
                String text = "{\"n\": " + n + "}";
                try (Document document = Document.fromJson(text)) {
                    assertEquals(expected, pad.evaluate(document));
                    assertEquals('"' + expected + '"', pad.evaluateJson(document));
                }
                assertEquals(expected, pad.evaluateFromJson(text));
                assertEquals('"' + expected + '"', pad.evaluateJson(text));
                assertEquals(expected, pad.evaluate(Map.of("n", n)));
            }
        }
    }
}
