package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.time.Instant;
import java.util.ArrayList;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import org.junit.jupiter.api.Test;

/**
 * Audited evaluations (0.25 #8), the cases of the Python suite's and the WebAssembly suite's
 * audits.
 */
class AuditTest {
    private static final Map<String, Object> ORDER = Map.of(
            "number", "SO-12345",
            "customer", Map.of("login", "alice.walker99", "name", "Acme Stores"),
            "items", List.of(
                    Map.of("code", "1", "type", "apparel", "unitPrice", 10000.0, "quantity", 2),
                    Map.of("code", "2", "type", "electronic", "unitPrice", 25000.0, "quantity", 1),
                    Map.of("code", "3", "type", "electronic", "unitPrice", 12000.0, "quantity", 1)));

    private static final String RULE = """
            (
              $total := $sum(items.(unitPrice * quantity));
              /** @label "Authorized customer" @message "The customer must be approved." */
              customer.login in ["alice.walker99"]
              and
              /** @label "Minimum total" */
              $total > $minimum
            )""";

    /** Every node of the tree under {@code root}, depth first. */
    private static List<AuditNode> nodes(AuditNode root) {
        List<AuditNode> nodes = new ArrayList<>();
        Deque<AuditNode> pending = new ArrayDeque<>();
        if (root != null) pending.push(root);
        while (!pending.isEmpty()) {
            AuditNode node = pending.pop();
            nodes.add(node);
            node.children().reversed().forEach(pending::push);
        }
        return nodes;
    }

    private static AuditResult audited(String source, Object input, Bindings bindings, AuditOptions audit) {
        try (Expression expression = Expression.compile(source)) {
            return expression.evaluateWithAudit(input, bindings, audit);
        }
    }

    @Test
    void evaluateWithAudit() {
        try (Expression expression = Expression.compile(RULE);
                Bindings minimum = Bindings.of(Map.of("minimum", 60000));
                Bindings low = Bindings.of(Map.of("minimum", 1))) {
            AuditResult audit = expression.evaluateWithAudit(ORDER, minimum);
            assertEquals(false, audit.result());
            assertNull(audit.error());
            assertEquals(AuditLevel.DECISIONS, audit.audit().header().level());
            assertEquals(false, audit.audit().header().result());
            assertNotNull(audit.audit().root());
            List<String> failed = audit.explanation().lines().stream()
                    .filter(line -> Boolean.FALSE.equals(line.passed()))
                    .map(Explanation.Line::text)
                    .toList();
            assertTrue(failed.stream().anyMatch(text -> text.startsWith("Minimum total")), failed.toString());
            String text = audit.explanation().text();
            assertTrue(text.startsWith("result: false"), text);
            assertTrue(text.contains("✘ Minimum total"), text);
            assertEquals(text, audit.explanation().toString());
            assertEquals("false", audit.explanation().result());

            AuditOptions full = AuditOptions.of(AuditLevel.FULL)
                    .withLimits(CaptureLimits.DEFAULTS.withMaxNodes(5));
            AuditResult limited = expression.evaluateWithAudit(ORDER, low, full);
            assertEquals(true, limited.result());
            assertEquals(5, limited.audit().nodeCount());
            assertTrue(limited.audit().truncated());
            assertTrue(nodes(limited.audit().root()).size() <= 5);
            AuditResult off = expression.evaluateWithAudit(ORDER, low, AuditOptions.of(AuditLevel.OFF));
            assertNull(off.audit().root());
            assertEquals(true, off.result());
        }

        AuditResult failing = audited("$x.y + 1", null, Bindings.of(Map.of("x", Map.of("y", "a"))),
                AuditOptions.DEFAULTS);
        assertEquals("T2001", failing.error().code());
        assertEquals("\"a\"", failing.error().valueJson().orElseThrow());
        assertNull(failing.result());
        assertEquals(new AuditError("T2001", failing.error().getMessage()), failing.audit().header().error());
        try (Expression range = Expression.compile("[1..100]")) {
            AuditResult tooLong = range.evaluateWithAudit(null, Bindings.NONE,
                    Options.builder().maxArrayLength(50).build(), AuditOptions.DEFAULTS);
            assertEquals("U1001", tooLong.error().code());
        }

        assertThrows(NullPointerException.class, () -> AuditOptions.of(null));
        assertThrows(IllegalArgumentException.class, () -> CaptureLimits.DEFAULTS.withMaxNodes(-1));
        try (Expression expression = Expression.compile(RULE)) {
            List<Object> cyclic = new ArrayList<>();
            cyclic.add(cyclic);
            IllegalArgumentException cycle =
                    assertThrows(IllegalArgumentException.class, () -> expression.evaluateWithAudit(cyclic));
            assertTrue(cycle.getMessage().contains("Circular reference detected"), cycle.getMessage());
            try (Document document = Document.fromJson("{}")) {
                assertThrows(IllegalArgumentException.class, () -> expression.evaluateWithAudit(document));
            }
        }
    }

    @Test
    void theHeaderAndWhatNodesDecided() {
        Instant now = Instant.parse("2026-10-25T12:00:00Z");
        String source = "($n := 3; $n > 2 ? ($n > 1 or $n < 5) : (false and $n))";
        try (Expression expression = Expression.compile(source)) {
            AuditResult audit = expression.evaluateWithAudit(null, Bindings.NONE,
                    Options.builder().now(now).build(), AuditOptions.DEFAULTS);
            assertEquals(true, audit.result());
            AuditTree.Header header = audit.audit().header();
            assertEquals(now, header.timestamp());
            assertTrue(header.engine().startsWith("jlens-core"), header.engine());
            assertEquals(16, header.expressionHash().length());
            assertTrue(header.steps() > 0);
            assertTrue(!header.duration().isNegative());
            List<AuditNode> nodes = nodes(audit.audit().root());
            assertTrue(nodes.stream().anyMatch(n -> n.decision() instanceof AuditNode.Binding b
                    && b.variable().equals("$n")), nodes.toString());
            assertTrue(nodes.stream().anyMatch(n -> n.decision() instanceof AuditNode.Conditional c
                    && c.branch() == AuditNode.Branch.THEN), nodes.toString());
            AuditNode comparison = nodes.stream().filter(n -> n.source().equals("$n > 2")).findFirst().orElseThrow();
            assertEquals(new AuditNode.Comparison(3L, 2L), comparison.decision());
            assertEquals(AuditNode.Outcome.TRUE, comparison.outcome());
            assertEquals(Boolean.TRUE, comparison.passed());
            assertEquals(">", comparison.op());
            AuditNode or = nodes.stream().filter(n -> "or".equals(n.op())).findFirst().orElseThrow();
            assertInstanceOf(AuditNode.ShortCircuit.class, or.decision());
        }
        // A filter's statistics, its rejected items as Java objects.
        try (Expression expression = Expression.compile("items[unitPrice > 11000].code")) {
            AuditResult audit = expression.evaluateWithAudit(ORDER);
            assertEquals(List.of("2", "3"), audit.result());
            AuditNode.Filter filter = nodes(audit.audit().root()).stream()
                    .flatMap(n -> n.filters().stream()).findFirst().orElseThrow();
            assertEquals("unitPrice > 11000", filter.source());
            assertEquals(3, filter.tested());
            assertEquals(2, filter.passed());
            @SuppressWarnings("unchecked")
            Map<String, Object> rejected = (Map<String, Object>) filter.rejected().getFirst();
            assertEquals("1", rejected.get("code"));
        }
        // An undefined operand is null.
        try (Expression expression = Expression.compile("missing = 1")) {
            AuditNode root = expression.evaluateWithAudit(Map.of()).audit().root();
            assertEquals(new AuditNode.Comparison(null, 1L), root.decision());
        }
    }

    @Test
    void auditSpansAreUtf16Offsets() {
        String source = "'𝄞' = 'x' or /** @label \"B\" */ $b";
        AuditResult audit = audited(source, null, Bindings.of(Map.of("b", true)), AuditOptions.DEFAULTS);
        AuditNode comparison = nodes(audit.audit().root()).stream()
                .filter(n -> n.source().equals("'𝄞' = 'x'")).findFirst().orElseThrow();
        assertEquals(new Span(0, 10), comparison.span());
        AuditNode labelled = nodes(audit.audit().root()).stream()
                .filter(n -> "B".equals(n.label())).findFirst().orElseThrow();
        assertEquals("$b", labelled.source());
        assertEquals(new Span(32, 34), labelled.span());
        assertEquals("$b", source.substring(32, 34));

        // A captured value's own `span` field is data, left as it is.
        Map<String, Object> value = Map.of("span", Map.of("start", 4L, "end", 9L));
        AuditResult full = audited("($e := 'é'; $x)", null, Bindings.of(Map.of("x", value)),
                AuditOptions.of(AuditLevel.FULL));
        List<Object> outputs = nodes(full.audit().root()).stream()
                .filter(n -> n.source().equals("$x")).map(AuditNode::output).toList();
        assertEquals(List.of(value), outputs);
    }

    @Test
    void hostFunctionsInAudits() {
        HostFunctionTest.RateException failing = new HostFunctionTest.RateException("The rate service is down");
        try (Expression expression = Expression.compile("1 + $rate()");
                Bindings throwing = Bindings.builder().function("rate", 0, args -> { throw failing; }).build();
                Bindings fine = Bindings.builder().function("rate", 0, args -> 2).build()) {
            AuditResult audited = expression.evaluateWithAudit(null, throwing);
            assertEquals("RateException", audited.error().code());
            assertEquals("The rate service is down", audited.error().getMessage());
            assertSame(failing, audited.error().getCause(), "the exception as the cause");
            assertEquals(3L, expression.evaluateWithAudit(null, fine).result());
        }
        // An error of a function a host function called keeps its code and position.
        try (Bindings apply = Bindings.builder()
                .function("apply", "<f:x>", args -> ((JsonataFunction) args.getFirst()).call())
                .build()) {
            AuditResult audited = audited("$apply(function() { 1 + 'a' })", null, apply, AuditOptions.DEFAULTS);
            assertEquals("T2002", audited.error().code());
            assertEquals(23, audited.error().position().orElseThrow());
            assertNull(audited.error().getCause());
        }
    }

    /**
     * A full audit of a deep recursion stops with {@code D1011} or succeeds, never crashes the
     * JVM, on threads with small stacks too.
     */
    @Test
    void deepAudits() throws Exception {
        String loop = "($f := function($n) { $n = 0 ? 0 : 1 + $f($n - 1) }; $f(%d))";
        for (long stack : new long[] {128 << 10, 1 << 20}) {
            for (int depth : new int[] {10_000, 1_000_000}) {
                AtomicReference<String> outcome = new AtomicReference<>();
                Consumer<AuditResult> record = audit -> outcome.set(audit.error() != null
                        ? audit.error().code()
                        : audit.result() + " " + audit.audit().truncated());
                Thread thread = new Thread(null, () -> {
                    try (Expression expression = Expression.compile(loop.formatted(depth))) {
                        record.accept(expression.evaluateWithAudit(null, AuditOptions.of(AuditLevel.FULL)));
                    } catch (Throwable e) {
                        outcome.set(e.toString());
                    }
                }, "small stack", stack);
                thread.start();
                thread.join();
                String expected = depth > 50_000 ? "D1011" : depth + " true";
                assertEquals(expected, outcome.get(), "stack " + stack + ", depth " + depth);
            }
        }
    }

    @Test
    void closedObjects() {
        Expression expression = Expression.compile("1");
        expression.close();
        assertThrows(IllegalStateException.class, () -> expression.evaluateWithAudit(null));
        Bindings bindings = Bindings.of(Map.of("x", 1));
        bindings.close();
        try (Expression open = Expression.compile("$x")) {
            assertThrows(IllegalStateException.class, () -> open.evaluateWithAudit(null, bindings));
        }
    }
}
