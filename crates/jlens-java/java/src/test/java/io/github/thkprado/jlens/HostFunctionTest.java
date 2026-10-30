package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.Date;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.OptionalInt;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import org.junit.jupiter.api.Test;

/**
 * Bindings and host functions (0.25 #7), the cases of the Python suite's and the WebAssembly
 * suite's host functions.
 */
class HostFunctionTest {
    private static final Map<String, Object> ORDER = Map.of("items", List.of(1, 2));

    /** The exception of the rate service, with a code of its own. */
    static final class RateException extends RuntimeException {
        @java.io.Serial
        private static final long serialVersionUID = 1L;

        RateException(String message) {
            super(message);
        }
    }

    private static Object evaluate(String source, Bindings bindings) {
        try (Expression expression = Expression.compile(source)) {
            return expression.evaluateFromJson((String) null, bindings);
        }
    }

    private static Object evaluate(String source, Bindings bindings, Options options) {
        try (Expression expression = Expression.compile(source)) {
            return expression.evaluateFromJson((String) null, bindings, options);
        }
    }

    private static double number(Object value) {
        return ((Number) value).doubleValue();
    }

    @Test
    void bindingsHoldVariables() {
        try (Expression expression = Expression.compile("$limit + $count(items)");
                Bindings limit = Bindings.of(Map.of("limit", 10))) {
            assertEquals(12L, expression.evaluate(ORDER, limit));
            assertEquals(12L, expression.evaluateFromJson("{\"items\": [1, 2]}", limit));
            assertEquals("12", expression.evaluateJson("{\"items\": [1, 2]}", limit));
        }
        Map<String, Object> nothing = new HashMap<>();
        nothing.put("x", null);
        assertNull(evaluate("$x", Bindings.of(nothing)));
        assertEquals(List.of(1L, 2L), evaluate("$x.y", Bindings.of(Map.of("x", Map.of("y", List.of(1, 2))))));
        assertEquals("$", evaluate("$d", Bindings.builder().value("$d", "$").build()), "the $ is optional");
        assertThrows(IllegalArgumentException.class, () -> Bindings.of(Map.of("limit", Set.of(1))));
        assertThrows(IllegalArgumentException.class, () -> Bindings.of(Map.of("limit", Map.of(1, 2))));
        List<Object> cyclic = new ArrayList<>();
        cyclic.add(cyclic);
        IllegalArgumentException cycle =
                assertThrows(IllegalArgumentException.class, () -> Bindings.of(Map.of("limit", cyclic)));
        assertTrue(cycle.getMessage().contains("Circular reference detected"), cycle.getMessage());
        List<Object> deep = new ArrayList<>();
        List<Object> at = deep;
        for (int i = 0; i < 2_000; i++) {
            List<Object> inner = new ArrayList<>();
            at.add(inner);
            at = inner;
        }
        assertEquals("U1001", assertThrows(JlensException.class, () -> Bindings.of(Map.of("deep", deep))).code());
        Bindings closed = Bindings.of(Map.of("x", 1));
        closed.close();
        closed.close();
        assertThrows(IllegalStateException.class, () -> evaluate("$x", closed));
        Bindings.NONE.close();
        assertEquals(1L, evaluate("1", Bindings.NONE));
    }

    private static final Bindings FUNCTIONS = Bindings.builder()
            .function("add", 2, args -> number(args.get(0)) + number(args.get(1)))
            .function("args", 1, args -> new ArrayList<>(args))
            .function("types", 0, args -> args.stream()
                    .map(a -> a == null ? "null" : a.getClass().getSimpleName())
                    .toList())
            .function("two", 2, args -> List.of(args.get(0), args.get(1)))
            .function("pair", 0, args -> List.of(Map.of("a", 1), Map.of("a", 2)))
            .function("nothing", 0, args -> null)
            .value("rate", 0.5)
            .build();

    @Test
    void hostFunctionsInTheBindings() {
        assertEquals(3L, evaluate("$add(1, 2)", FUNCTIONS));
        try (Expression expression = Expression.compile("$add(price, $rate)")) {
            assertEquals(2.5, expression.evaluate(Map.of("price", 2), FUNCTIONS));
        }
        List<Object> given = new ArrayList<>();
        given.add(1L);
        given.add("a");
        given.add(null);
        given.add(Map.of("k", List.of(1L)));
        assertEquals(given, evaluate("$args(1, 'a', missing, {'k': [1]})", FUNCTIONS));
        assertEquals(List.of("JsonataFunction", "JsonataFunction", "Long"),
                evaluate("$types($add, function($x) { $x }, 1)", FUNCTIONS));
        assertEquals(List.of(1L, 2L), evaluate("$pair().a", FUNCTIONS));
        assertNull(evaluate("$nothing()", FUNCTIONS));
        assertEquals(false, evaluate("$exists($nothing())", FUNCTIONS));
        // The arity is what higher-order functions use.
        assertEquals(List.of(List.of(5L), List.of(6L)), evaluate("$map([5, 6], $args)", FUNCTIONS));
        assertEquals(List.of(List.of(5L, 0L), List.of(6L, 1L)), evaluate("$map([5, 6], $two)", FUNCTIONS));
        assertEquals(42L, evaluate("$add(?, 1)(41)", FUNCTIONS));
        assertEquals("function", evaluate("$type($add)", FUNCTIONS));
        assertEquals(1L, evaluate("$sum([1])", FUNCTIONS));
        Bindings mine = Bindings.builder().function("sum", 0, args -> "mine").build();
        assertEquals("mine", evaluate("$sum([1])", mine), "bindings take precedence over built-ins");
        try (Expression expression = Expression.compile("$pair()[0]");
                Document document = Document.fromJson("{}")) {
            assertEquals("{\"a\":1}", expression.evaluateJson("{}", FUNCTIONS));
            assertEquals("{\"a\":1}", expression.evaluateJson(document, FUNCTIONS));
            assertEquals(Map.of("a", 1L), expression.evaluate(document, FUNCTIONS));
        }
        // A name holds a value or a function: binding one replaces the other.
        Bindings replaced = Bindings.builder().value("f", 1).function("f", 0, args -> 2).build();
        assertEquals(2L, evaluate("$f()", replaced));
        Bindings back = Bindings.builder().function("f", 0, args -> 2).value("f", 1).build();
        assertEquals(1L, evaluate("$f", back));
    }

    @Test
    void signaturesAreChecked() {
        Bindings tax = Bindings.builder()
                .function("tax", "<n-n?:n>", args -> number(args.get(0))
                        * (args.get(1) == null ? 0.25 : number(args.get(1))))
                .build();
        try (Expression expression = Expression.compile("$tax(100) + price.$tax()")) {
            assertEquals(27L, expression.evaluate(Map.of("price", 8), tax));
        }
        assertEquals("T1006", assertThrows(JlensException.class, () -> evaluate("$tax('x')", Bindings.NONE)).code());
        Bindings typed = Bindings.builder().function("$tax", "<n:n>", args -> args.get(0)).build();
        JlensException error = assertThrows(JlensException.class, () -> evaluate("$tax('x')", typed));
        assertEquals("T0410", error.code());
        assertEquals(OptionalInt.of(5), error.position());
        assertEquals("tax", error.token().orElseThrow());
        JlensException invalid = assertThrows(JlensException.class,
                () -> Bindings.builder().function("f", "<n<s>:n>", args -> 1).build());
        assertTrue(invalid.code().startsWith("S04"), invalid.code());
        assertThrows(IllegalArgumentException.class, () -> Bindings.builder().function("f", -1, args -> 1));
        // One set of bindings serves every thread.
        try (Expression expression = Expression.compile("$tax(100) + price.$tax()");
                ExecutorService pool = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Object>> results = new ArrayList<>();
            for (int i = 0; i < 64; i++) results.add(pool.submit(() -> expression.evaluate(Map.of("price", 8), tax)));
            for (Future<Object> result : results) assertEquals(27L, result.get());
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    @Test
    void exceptionsOfHostFunctionsAreThrownAgain() {
        RateException failing = new RateException("The rate service is down");
        Bindings rate = Bindings.builder().function("rate", 0, args -> { throw failing; }).build();
        try (Expression expression = Expression.compile("1 + $rate()");
                Document document = Document.fromJson("{}")) {
            assertSame(failing, assertThrows(RateException.class, () -> expression.evaluateFromJson((String) null, rate)));
            assertSame(failing, assertThrows(RateException.class, () -> expression.evaluateJson((String) null, rate)));
            assertSame(failing, assertThrows(RateException.class, () -> expression.evaluate(document, rate)));
            assertSame(failing, assertThrows(RateException.class, () -> expression.evaluate(Map.of(), rate)));
            // An Error goes through too.
            AssertionError thrown = new AssertionError("interrupted");
            Bindings interrupt = Bindings.builder().function("rate", 0, args -> { throw thrown; }).build();
            assertSame(thrown, assertThrows(AssertionError.class, () -> expression.evaluateFromJson((String) null, interrupt)));
            // A result that is not a JSON value fails like a thrown exception.
            Map<String, Object> cyclic = new HashMap<>();
            cyclic.put("self", cyclic);
            Bindings returningCycle = Bindings.builder().function("rate", 0, args -> cyclic).build();
            IllegalArgumentException cycle = assertThrows(IllegalArgumentException.class,
                    () -> expression.evaluateFromJson((String) null, returningCycle));
            assertTrue(cycle.getMessage().contains("the result of $rate"), cycle.getMessage());
            Bindings returningDate = Bindings.builder().function("rate", 0, args -> new Date()).build();
            assertThrows(IllegalArgumentException.class, () -> expression.evaluateFromJson((String) null, returningDate));
        }
        // An exception wrapped by $eval is the error of $eval.
        assertEquals("D3121", assertThrows(JlensException.class, () -> evaluate("$eval('$rate()')", rate)).code());
        // Expressions evaluated by a host function have their own functions and exceptions.
        try (Expression inner = Expression.compile("$g()")) {
            Bindings twenty = Bindings.builder().function("g", 0, args -> 20).build();
            Bindings nested = Bindings.builder()
                    .function("f", 0, args -> number(inner.evaluateFromJson((String) null, twenty)) + 1)
                    .build();
            assertEquals(42L, evaluate("$f() * 2", nested));
            Bindings failingInside = Bindings.builder()
                    .function("f", 0, args -> inner.evaluateFromJson((String) null,
                            Bindings.builder().function("g", 0, a -> { throw failing; }).build()))
                    .build();
            assertSame(failing, assertThrows(RateException.class, () -> evaluate("$f()", failingInside)));
        }
    }

    @Test
    void asynchronousHostFunctionsFailWithU1002() {
        Bindings fetch = Bindings.builder()
                .function("fetch", 0, args -> CompletableFuture.completedFuture(1))
                .build();
        JlensException error = assertThrows(JlensException.class, () -> evaluate("$fetch()", fetch));
        assertEquals("U1002", error.code());
        assertEquals("fetch", error.token().orElseThrow());
        assertTrue(error.getMessage().contains("$fetch"), error.getMessage());
    }

    private static Object safe(JsonataFunction f) {
        try {
            return f.call();
        } catch (JlensException e) {
            return e.code();
        }
    }

    private static final Bindings CALLING = Bindings.builder()
            .function("apply", 1, args -> ((JsonataFunction) args.get(0))
                    .call(args.subList(1, args.size()).toArray()))
            .function("shape", 1, args -> List.of(args.get(0).getClass().getSimpleName(),
                    ((JsonataFunction) args.get(0)).arity()))
            .function("safe", 1, args -> safe((JsonataFunction) args.get(0)))
            .function("give", 1, args -> ((JsonataFunction) args.get(0))
                    .call((JsonataFunction) args.get(0), List.of(2)))
            .function("add", 2, args -> number(args.get(0)) + number(args.get(1)))
            .function("rate", 0, args -> { throw new RateException("down"); })
            .build();

    @Test
    void hostFunctionsCallTheFunctionsTheyReceive() {
        assertEquals(3L, evaluate("$apply(function($x, $y) { $x + $y }, 1, 2)", CALLING));
        assertEquals("A", evaluate("$apply($uppercase, 'a')", CALLING));
        assertEquals("bc", evaluate("$apply($substring(?, 1), 'abc')", CALLING));
        assertEquals("A", evaluate("$apply($uppercase ~> $trim, ' a ')", CALLING));
        assertEquals(3L, evaluate("$apply($add, 1, 2)", CALLING));
        assertEquals(6L, evaluate("($n := 5; $apply(function() { $n + 1 }))", CALLING));
        assertEquals(List.of("JsonataFunction", 2L), evaluate("$shape(function($a, $b) { 0 })", CALLING));
        assertEquals("null", evaluate("$apply($string)", CALLING));
        assertEquals(Map.of("a", List.of(2L)),
                evaluate("$give(function($f, $a) { {'f': $type($f), 'a': $a} })", CALLING));
        assertNull(evaluate("$apply(function() { $uppercase })", CALLING));
        // null stands for undefined.
        Object[] none = {null};
        Bindings undefined = Bindings.builder()
                .function("f", 1, args -> ((JsonataFunction) args.get(0)).call(none))
                .build();
        assertEquals(false, evaluate("$f(function($x) { $exists($x) })", undefined));
        Bindings twice = Bindings.builder()
                .function("twice", "<fx:x>", args -> {
                    JsonataFunction f = (JsonataFunction) args.get(0);
                    return f.call(f.call(args.get(1)));
                })
                .build();
        assertEquals(18L, evaluate("$twice(function($n) { $n * 3 }, 2)", twice));
        // Errors are thrown with their codes; the host function may catch them or let them through.
        assertEquals("D3137", evaluate("$safe(function() { $error('no') })", CALLING));
        JlensException error = assertThrows(JlensException.class,
                () -> evaluate("$apply(function() { $error('no') })", CALLING));
        assertEquals("D3137", error.code());
        assertEquals("no", error.getMessage());
        JlensException typed = assertThrows(JlensException.class,
                () -> evaluate("$apply(function() { 1 + 'a' })", CALLING));
        assertEquals("T2002", typed.code());
        assertEquals(OptionalInt.of(23), typed.position(), "the position inside the lambda");
        RateException down = assertThrows(RateException.class, () -> evaluate("$apply($rate)", CALLING));
        assertEquals("down", down.getMessage());
        Options shallow = Options.builder().maxDepth(50).build();
        assertEquals("D1011", assertThrows(JlensException.class,
                () -> evaluate("($f := function($n) { $apply($f, $n + 1) }; $f(0))", CALLING, shallow)).code());
    }

    @Test
    void aFunctionCalledOutsideItsHostFunctionFailsWithU1005() {
        List<JsonataFunction> kept = new ArrayList<>();
        Bindings keep = Bindings.builder()
                .function("keep", 1, args -> {
                    JsonataFunction f = (JsonataFunction) args.get(0);
                    kept.add(f);
                    return f.call(1);
                })
                .build();
        assertEquals(2L, evaluate("$keep(function($x) { $x + 1 })", keep));
        JlensException late = assertThrows(JlensException.class, () -> kept.getFirst().call(1));
        assertEquals("U1005", late.code());
        assertEquals("keep", late.token().orElseThrow());
        // From another thread while the host function runs: U1005 too.
        Bindings other = Bindings.builder()
                .function("other", 1, args -> {
                    try (ExecutorService pool = Executors.newSingleThreadExecutor()) {
                        return pool.submit(() -> safe((JsonataFunction) args.get(0))).get();
                    } catch (Exception e) {
                        throw new IllegalStateException(e);
                    }
                })
                .build();
        assertEquals("U1005", evaluate("$other(function() { 1 })", other));
        // Called again while it runs, through another host function: an error, not a crash.
        List<JsonataFunction> stash = new ArrayList<>();
        Bindings reentry = Bindings.builder()
                .function("outer", 1, args -> {
                    stash.add((JsonataFunction) args.get(0));
                    return stash.getFirst().call();
                })
                .function("inner", 0, args -> stash.getFirst().call())
                .build();
        assertEquals("U1001", assertThrows(JlensException.class,
                () -> evaluate("$outer(function() { $inner() })", reentry)).code());
    }

    @Test
    void functionsAreCalledWithEveryKindOfInput() {
        try (Expression expression = Expression.compile("$apply(function($x) { $x * 2 }, a)");
                Document document = Document.fromJson("{\"a\": 5}")) {
            assertEquals("8", expression.evaluateJson("{\"a\": 4}", CALLING));
            assertEquals(10L, expression.evaluate(document, CALLING));
            Map<String, Object> input = new LinkedHashMap<>();
            input.put("a", 6);
            assertEquals(12L, expression.evaluate(input, CALLING));
        }
        // A function's arguments can be any JSON value, and so can its result.
        Map<String, Object> value = new LinkedHashMap<>();
        value.put("list", List.of(1, "two", 3.5, true));
        value.put("none", null);
        Bindings identity = Bindings.builder().function("id", 1, args -> args.getFirst()).build();
        Map<String, Object> expected = new LinkedHashMap<>();
        expected.put("list", List.of(1L, "two", 3.5, true));
        expected.put("none", null);
        assertEquals(expected, evaluate("$apply(function($v) { $v }, $value)",
                Bindings.builder().value("value", value)
                        .function("apply", 2, args -> ((JsonataFunction) args.get(0)).call(args.get(1)))
                        .build()));
        assertEquals(expected, evaluate("$id({'list': [1, 'two', 3.5, true], 'none': null})", identity));
        Object result = evaluate("$apply(function() { {'a': [1, {'b': null}]} })", CALLING);
        assertInstanceOf(LinkedHashMap.class, result);
        assertFalse(((Map<?, ?>) result).isEmpty());
    }

    /**
     * A host function that evaluates, while the evaluation that called it holds its thread's
     * scratch buffer (roadmap 0.26 #10), and the reply of each reaches its own caller.
     */
    @Test
    void aHostFunctionEvaluatesInside() {
        try (Expression inner = Expression.compile("$string(n * 2)");
                Bindings outer = Bindings.builder()
                        .function("inner", 1, args -> inner.evaluate(Map.of("n", args.get(0))))
                        .build();
                Expression expression = Expression.compile("[$inner(1), $inner(2)] & '/' & n")) {
            for (int i = 0; i < 100; i++) {
                assertEquals("[\"2\",\"4\"]/" + i, expression.evaluate(Map.of("n", i), outer));
            }
        }
    }
}
