package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Date;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import org.junit.jupiter.api.Test;

/**
 * The code of the Java chapter of the book ({@code docs/src/guides/java.md}), which includes it
 * between the {@code ANCHOR} comments. Keeping it here means every snippet of the chapter
 * compiles and runs.
 */
class GuideTest {
    @Test
    void quickStart() {
        // ANCHOR: quick_start
        try (Expression expression = Expression.compile("$sum(items[type = $kind].(price * qty))");
                Bindings books = Bindings.of(Map.of("kind", "book"))) {
            Map<String, Object> order = Map.of("items", List.of(
                    Map.of("type", "book", "price", 12.5, "qty", 2),
                    Map.of("type", "pen", "price", 1.5, "qty", 10)));
            assertEquals(25L, expression.evaluate(order, books));
            assertEquals("25", expression.evaluateJson(
                    "{\"items\": [{\"type\": \"book\", \"price\": 12.5, \"qty\": 2}]}", books));
        }
        // ANCHOR_END: quick_start
    }

    @Test
    void undefined() {
        // ANCHOR: undefined
        try (Expression missing = Expression.compile("missing");
                Expression none = Expression.compile("null")) {
            assertNull(missing.evaluateFromJson("{}"));          // undefined
            assertNull(none.evaluateFromJson("{}"));             // null
            assertNull(missing.evaluateJson("{}"));              // undefined
            assertEquals("null", none.evaluateJson("{}"));       // null
        }
        // ANCHOR_END: undefined
    }

    @Test
    void options() {
        // ANCHOR: options
        try (Expression clock = Expression.compile("[$now(), $random()]")) {
            Options fixed = Options.builder()
                    .now(Instant.parse("2024-01-02T03:04:05Z"))
                    .seed(7)
                    .build();
            List<?> first = (List<?>) clock.evaluate(Map.of(), fixed);
            assertEquals("2024-01-02T03:04:05.000Z", first.get(0));
            assertEquals(first, clock.evaluate(Map.of(), fixed));
        }

        try (Expression loop = Expression.compile("($f := function($n) { $f($n + 1) }; $f(0))")) {
            Options briefly = Options.builder().timeout(Duration.ofMillis(50)).build();
            JlensException error = assertThrows(JlensException.class, () -> loop.evaluate(Map.of(), briefly));
            assertEquals("D1012", error.code());
        }

        try (Expression sum = Expression.compile("$sum($map([1..1000], function($i) { $i }))")) {
            Options budget = Options.builder().maxSteps(1000).build();  // the same on every machine
            JlensException error = assertThrows(JlensException.class, () -> sum.evaluate(Map.of(), budget));
            assertEquals("U1006", error.code());
            assertEquals("1000", error.valueJson().orElseThrow());
        }
        // ANCHOR_END: options
    }

    @Test
    void input() {
        // ANCHOR: input
        List<Object> items = List.of(Map.of("sku", "A1"), Map.of("sku", "B2"));
        Map<String, Object> order = Map.of(
                "customer", Map.of("name", "Ada"),
                "items", items,
                "placed", new Date());                           // no JSON form
        try (Expression name = Expression.compile("customer.name");
                Expression all = Expression.compile("items");
                Expression text = Expression.compile("$string($)")) {
            assertEquals("Ada", name.evaluate(order));           // reads two fields of two maps
            assertSame(items, all.evaluate(order));              // the list itself, not a copy
            JlensException error = assertThrows(JlensException.class, () -> text.evaluate(order));
            assertEquals("U1004", error.code());                 // reading "placed" failed
        }
        // ANCHOR_END: input
    }

    @Test
    void jsonText() {
        // ANCHOR: json_text
        String body = """
                {"customer": {"name": "Ada", "tier": "gold"}, "log": [{"at": 1}, {"at": 2}]}""";
        try (Expression tier = Expression.compile("customer.tier")) {
            assertEquals("\"gold\"", tier.evaluateJson(body));  // "log" is only checked as JSON
            assertEquals("gold", tier.evaluateFromJson(body));
            JlensException error = assertThrows(JlensException.class, () -> tier.evaluateJson("{"));
            assertEquals("U2001", error.code());
        }
        // ANCHOR_END: json_text
    }

    @Test
    void document() {
        // ANCHOR: document
        try (Document order = Document.fromJson("""
                        {"total": 120, "customer": {"tier": "gold"}}""");
                Expression big = Expression.compile("total > 100");
                Expression gold = Expression.compile("customer.tier = 'gold'")) {
            assertEquals(true, big.evaluate(order));
            assertEquals(true, gold.evaluate(order));
        }
        // ANCHOR_END: document
    }

    @Test
    void threads() throws Exception {
        // ANCHOR: threads
        try (Expression total = Expression.compile("$sum(items.price)");
                Document order = Document.fromJson("""
                        {"items": [{"price": 2}, {"price": 3}]}""");
                ExecutorService pool = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Object>> results = new ArrayList<>();
            for (int i = 0; i < 1000; i++) {
                results.add(pool.submit(() -> total.evaluate(order)));
            }
            for (Future<Object> result : results) {
                assertEquals(5L, result.get());
            }
        }
        // ANCHOR_END: threads
    }

    @Test
    void hostFunctions() {
        Map<String, Object> order = Map.of(
                "region", "south",
                "items", List.of(Map.of("weight", 2), Map.of("weight", 3)));
        // ANCHOR: host_functions
        Map<String, Double> rates = Map.of("north", 1.5, "south", 2.0);
        try (Bindings bindings = Bindings.builder()
                        .function("shipping", "<ns:n>", args ->
                                ((Number) args.get(0)).doubleValue() * rates.get((String) args.get(1)))
                        .build();
                Expression expression = Expression.compile("$shipping($sum(items.weight), region)")) {
            assertEquals(10L, expression.evaluate(order, bindings));
        }
        // ANCHOR_END: host_functions
    }

    @Test
    void hostExceptions() {
        // ANCHOR: host_exceptions
        IllegalStateException down = new IllegalStateException("the rate service is down");
        try (Bindings bindings = Bindings.builder()
                        .function("rate", 1, args -> {
                            throw down;
                        })
                        .build();
                Expression expression = Expression.compile("$rate(region)")) {
            Map<String, Object> order = Map.of("region", "south");
            IllegalStateException thrown =
                    assertThrows(IllegalStateException.class, () -> expression.evaluate(order, bindings));
            assertSame(down, thrown);
        }
        // ANCHOR_END: host_exceptions
    }

    @Test
    void functionsAsArguments() {
        Map<List<Object>, Long> stock = new java.util.HashMap<>();
        stock.put(List.of("A1", 1L), null);
        stock.put(List.of("A1", 2L), 7L);
        HostFunction lookup = args -> {
            Long found = stock.get(List.of(args.get(0), args.get(1)));
            if (found == null) throw new IllegalStateException("timed out");
            return found;
        };
        // ANCHOR: functions_as_arguments
        HostFunction retry = args -> {
            JsonataFunction step = (JsonataFunction) args.get(0);
            long times = (Long) args.get(1);
            for (long attempt = 1; ; attempt++) {
                try {
                    return step.call(attempt);
                } catch (RuntimeException e) {
                    if (attempt == times) throw e;
                }
            }
        };
        try (Bindings bindings = Bindings.builder()
                        .function("retry", 2, retry)
                        .function("lookup", 2, lookup)
                        .build();
                Expression expression = Expression.compile("$retry(function($i) { $lookup(sku, $i) }, 3)")) {
            assertEquals(7L, expression.evaluate(Map.of("sku", "A1"), bindings));
        }
        // ANCHOR_END: functions_as_arguments
    }

    @Test
    void errors() {
        // ANCHOR: errors
        JlensException error = assertThrows(JlensException.class,
                () -> Expression.compile("Account.Order[0;]"));
        assertEquals("S0202", error.code());
        assertEquals("Expected \"]\", got \";\"", error.getMessage());
        assertEquals(16, error.position().getAsInt());           // where it was detected
        assertEquals(15, error.start().getAsInt());              // the offending part of the source
        assertEquals(16, error.end().getAsInt());
        assertEquals(";", error.token().orElseThrow());
        // ANCHOR_END: errors

        // ANCHOR: error_value
        try (Expression expression = Expression.compile("customer.name + 1")) {
            JlensException notANumber = assertThrows(JlensException.class,
                    () -> expression.evaluate(Map.of("customer", Map.of("name", "Acme"))));
            assertEquals("T2001", notANumber.code());
            assertEquals("\"Acme\"", notANumber.valueJson().orElseThrow());
        }
        // ANCHOR_END: error_value
    }

    @Test
    void reads() {
        // ANCHOR: reads
        try (Expression expression = Expression.compile("$sum(items[type = $kind].(price * quantity))")) {
            Reads reads = expression.reads();
            assertEquals(new Read.Fields(Map.of("items", new Read.Fields(Map.of(
                            "price", Read.WHOLE, "quantity", Read.WHOLE, "type", Read.WHOLE)))),
                    reads.input());
            assertEquals(Map.of("kind", Read.WHOLE), reads.bindings());
            assertEquals("{\"input\":{\"items\":{\"price\":true,\"quantity\":true,\"type\":true}},"
                    + "\"bindings\":{\"kind\":true}}", reads.toJson());
        }
        // ANCHOR_END: reads

        String text = """
                {"total": 120, "customer": {"tier": "gold", "notes": "..."}, "log": []}""";
        // ANCHOR: document_read
        try (Expression big = Expression.compile("total > 100");
                Expression gold = Expression.compile("customer.tier = 'gold'")) {
            Read read = Reads.union(List.of(big.reads(), gold.reads())).input();
            try (Document document = Document.fromJson(text, read)) {   // "notes" and "log" skipped
                assertEquals(true, big.evaluate(document));
                assertEquals(true, gold.evaluate(document));
            }
        }
        // ANCHOR_END: document_read
    }

    @Test
    void audit() {
        // ANCHOR: audit
        try (Expression expression = Expression.compile(
                        "/** @label \"Big order\" */ $sum(items.(price * qty)) > $limit");
                Bindings limit = Bindings.of(Map.of("limit", 20))) {
            Map<String, Object> order = Map.of("items", List.of(
                    Map.of("price", 2.5, "qty", 4),
                    Map.of("price", 1, "qty", 3)));
            AuditOptions options = AuditOptions.of(AuditLevel.DECISIONS)
                    .withLimits(CaptureLimits.DEFAULTS.withMaxNodes(1000));
            AuditResult audited = expression.evaluateWithAudit(order, limit, options);
            assertEquals(false, audited.result());
            assertNull(audited.error());
            assertEquals("result: false\n✘ Big order — $sum(items.(price * qty)) > $limit → false (13 > 20)\n",
                    audited.explanation().text());
            AuditNode root = audited.audit().root();
            assertEquals("Big order", root.label());
            assertEquals(AuditNode.Outcome.FALSE, root.outcome());
            assertEquals(new AuditNode.Comparison(13L, 20L), root.decision());
        }
        // ANCHOR_END: audit
    }
}
