package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
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
    void documentsAreVariables() {
        try (Document before = Document.fromJson("{\"total\":10,\"lines\":[1,2]}");
                Document after = Document.fromJson("{\"total\":12,\"lines\":[1,2,3]}");
                Expression grew = Expression.compile("total - $before.total");
                Expression both = Expression.compile("[$count($a.lines), $count($b.lines), $]")) {
            assertEquals(2L, grew.evaluate(after, Map.of("before", before)));
            assertEquals("2", grew.evaluateJson(after, Map.of("$before", before)));
            // One document as the input and as a variable.
            assertEquals("0", grew.evaluateJson(after, Map.of("before", after)));
            // With no input, the expression runs on its variables alone.
            Map<String, Document> two = Map.of("a", before, "b", after);
            assertEquals(List.of(2L, 3L), both.evaluate(null, two));
            assertEquals("[2,3]", both.evaluateJson(null, two));
            assertEquals("[2,3]", both.evaluateJson(null, two, Bindings.NONE));
            // A variable that is not bound is undefined.
            assertNull(grew.evaluate(after, Map.of()));
            assertNull(grew.evaluateJson(after, Map.of()));
            Options few = Options.builder().maxSteps(2).build();
            assertEquals("U1006", assertThrows(JlensException.class,
                    () -> both.evaluate(null, two, Bindings.NONE, few)).code());
            assertEquals("U1006", assertThrows(JlensException.class,
                    () -> both.evaluateJson(null, two, Bindings.NONE, few)).code());
            // More names than the scratch buffer holds.
            Map<String, Document> many = new LinkedHashMap<>();
            for (int i = 0; i < 400; i++) many.put("a document of many " + i, after);
            many.put("before", before);
            assertEquals(2L, grew.evaluate(after, many));
            assertEquals("2", grew.evaluateJson(after, many));
        }
    }

    @Test
    void aDocumentOfAVariableKeepsTheFieldsRead() {
        try (Expression expression = Expression.compile("$count($table.rates[code = $$.code])");
                Document order = Document.fromJson("{\"code\":\"b\"}")) {
            Read read = expression.reads().bindings().get("table");
            assertEquals("{\"rates\":{\"code\":true}}", read.toJson());
            String text = "{\"rates\":[{\"code\":\"a\",\"x\":1},{\"code\":\"b\",\"x\":2}],\"y\":[]}";
            try (Document table = Document.fromJson(text, read);
                    Expression whole = Expression.compile("$table")) {
                assertEquals(1L, expression.evaluate(order, Map.of("table", table)));
                assertEquals("{\"rates\":[{\"code\":\"a\"},{\"code\":\"b\"}]}",
                        whole.evaluateJson(null, Map.of("table", table)));
            }
        }
    }

    @Test
    void aDocumentTakesThePlaceOfABindingOfItsName() {
        try (Bindings bindings = Bindings.builder()
                        .value("rate", 0.5)
                        .function("add", 2, args -> ((Number) args.get(0)).doubleValue()
                                + ((Number) args.get(1)).doubleValue())
                        .function("fail", 0, args -> {
                            throw new IllegalArgumentException("no rate");
                        })
                        .build();
                Document document = Document.fromJson("{\"x\":3}");
                Expression beside = Expression.compile("$add($doc.x, $rate)");
                Expression variable = Expression.compile("$add($rate.x, 1)");
                Expression function = Expression.compile("$add.x + $rate");
                Expression called = Expression.compile("$add(1, 2)");
                Expression failing = Expression.compile("$doc.x + $fail()")) {
            assertEquals(3.5, beside.evaluate(null, Map.of("doc", document), bindings));
            assertEquals(4L, variable.evaluate(null, Map.of("rate", document), bindings));
            assertEquals("3.5", function.evaluateJson(null, Map.of("add", document), bindings));
            assertEquals("T1006", assertThrows(JlensException.class,
                    () -> called.evaluate(null, Map.of("add", document), bindings)).code());
            // The bindings are as they were.
            assertEquals(3L, called.evaluate(null, Map.of(), bindings));
            assertEquals(3L, called.evaluate(document, bindings));
            // A host function's exception is thrown again, unchanged.
            assertEquals("no rate", assertThrows(IllegalArgumentException.class,
                    () -> failing.evaluate(null, Map.of("doc", document), bindings)).getMessage());
        }
    }

    @Test
    void closedDocumentsOfVariablesThrow() {
        Document closed = Document.fromJson("1");
        closed.close();
        Bindings freed = Bindings.of(Map.of("x", 1));
        freed.close();
        try (Document document = Document.fromJson("2");
                Expression expression = Expression.compile("$x")) {
            assertThrows(IllegalStateException.class,
                    () -> expression.evaluate(document, Map.of("x", closed)));
            assertThrows(IllegalStateException.class,
                    () -> expression.evaluateJson(closed, Map.of("x", document)));
            assertThrows(IllegalStateException.class,
                    () -> expression.evaluate(document, Map.of("x", document), freed));
            Map<String, Document> none = new HashMap<>();
            none.put("x", null);
            assertThrows(NullPointerException.class, () -> expression.evaluate(document, none));
            assertEquals(2L, expression.evaluate(null, Map.of("x", document)));
            Expression gone = Expression.compile("$x");
            gone.close();
            assertThrows(IllegalStateException.class,
                    () -> gone.evaluate(document, Map.of("x", document)));
        }
    }

    @Test
    void threadsShareTheDocumentsOfVariables() throws Exception {
        try (Document table = Document.fromJson("{\"rates\":[1,2,3]}");
                Document order = Document.fromJson("{\"index\":2}");
                Expression expression = Expression.compile("$table.rates[$$.index]");
                ExecutorService pool = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Object>> results = new ArrayList<>();
            for (int i = 0; i < 1000; i++) {
                results.add(pool.submit(() -> expression.evaluate(order, Map.of("table", table))));
            }
            for (Future<Object> result : results) assertEquals(3L, result.get());
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
