package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

class DocumentTest {
    private static final String TEXT =
            "{\"items\":[{\"name\":\"a\",\"price\":2},{\"name\":\"b\",\"price\":3}],\"other\":[1,2,3]}";

    @Test
    void evaluatesSeveralExpressions() {
        try (Document document = Document.fromJson(TEXT);
                Expression names = Expression.compile("items.name");
                Expression total = Expression.compile("$sum(items.price)")) {
            assertEquals("[\"a\",\"b\"]", names.evaluateJson(document));
            assertEquals("5", total.evaluateJson(document));
            assertEquals("5", total.evaluateJson(document, Options.builder().maxSteps(100).build()));
        }
    }

    @Test
    void keepsOnlyTheFieldsRead() {
        try (Expression names = Expression.compile("items.name");
                Expression other = Expression.compile("$count(other)")) {
            Read read = names.reads().input();
            assertEquals(
                    new Read.Fields(Map.of("items", new Read.Fields(Map.of("name", Read.WHOLE)))),
                    read);
            try (Document document = Document.fromJson(TEXT, read)) {
                assertEquals("[\"a\",\"b\"]", names.evaluateJson(document));
                assertEquals("0", other.evaluateJson(document), "other was not kept");
            }
            Read both = Reads.union(List.of(names.reads(), other.reads())).input();
            try (Document document = Document.fromJson(TEXT.getBytes(StandardCharsets.UTF_8), both)) {
                assertEquals("3", other.evaluateJson(document));
            }
        }
    }

    @Test
    void textThatIsNotJsonFails() {
        assertEquals("U2001", assertThrows(JlensException.class, () -> Document.fromJson("{")).code());
        assertEquals("U2001", assertThrows(JlensException.class,
                () -> Document.fromJson(new byte[] {(byte) 0xc3})).code());
    }

    @Test
    void closedDocumentsThrow() {
        Document document = Document.fromJson("1");
        document.close();
        document.close();
        try (Expression expression = Expression.compile("$")) {
            assertThrows(IllegalStateException.class, () -> expression.evaluateJson(document));
        }
    }

    @Test
    void readsAndTheirJson() {
        try (Expression expression = Expression.compile("$x.a + $count($y)")) {
            Reads reads = expression.reads();
            assertNull(reads.input());
            assertEquals("{\"bindings\":{\"x\":{\"a\":true},\"y\":{}}}", reads.toJson());
            assertEquals(reads, Reads.fromJson(reads.toJson()));
        }
        try (Expression expression = Expression.compile("$eval('a')")) {
            assertEquals(true, expression.reads().everyBinding());
        }
        // A host function receives its arguments and nothing else: with the bindings, a call
        // to one reads them alone, and the context a `-` of its signature may take.
        try (Bindings bindings = Bindings.builder()
                        .function("tax", "<n-n?:n>", args -> 0)
                        .function("rate", 1, args -> 0)
                        .value("limit", 1)
                        .build();
                Expression expression = Expression.compile("$rate(country) + $tax(price) + $limit");
                Expression context = Expression.compile("order.$tax()");
                Expression unknown = Expression.compile("$other(price)")) {
            assertEquals(Read.WHOLE, expression.reads().input());
            assertEquals(expression.reads(), expression.reads(Bindings.NONE));
            Reads reads = expression.reads(bindings);
            assertEquals("{\"input\":{\"country\":true,\"price\":true},"
                    + "\"bindings\":{\"limit\":true,\"rate\":true,\"tax\":true}}", reads.toJson());
            assertEquals("{\"order\":{}}", context.reads(bindings).input().toJson());
            assertEquals(Read.WHOLE, unknown.reads(bindings).input());
        }
        Bindings closed = Bindings.builder().function("rate", 1, args -> 0).build();
        closed.close();
        try (Expression expression = Expression.compile("$rate(1)")) {
            assertThrows(IllegalStateException.class, () -> expression.reads(closed));
        }
        assertEquals(Read.WHOLE, Read.fromJson("true"));
        assertEquals("{\"a\\\"b\":true}", Read.fromJson("{\"a\\\"b\": true}").toJson());
        assertThrows(IllegalArgumentException.class, () -> Read.fromJson("false"));
        assertThrows(IllegalArgumentException.class, () -> Read.fromJson("{"));
        assertThrows(IllegalArgumentException.class, () -> Reads.fromJson("[]"));
    }
}
