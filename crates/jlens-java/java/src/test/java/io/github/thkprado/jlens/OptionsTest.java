package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.time.Duration;
import java.time.Instant;
import org.junit.jupiter.api.Test;

class OptionsTest {
    private static final String LOOP =
            "($f := function($n) { $n = 0 ? 0 : 1 + $f($n - 1) }; $f(%d))";

    private static String evaluate(String expression, Options options) {
        try (Expression compiled = Expression.compile(expression)) {
            return compiled.evaluateJson((String) null, options);
        }
    }

    private static String code(String expression, Options options) {
        return assertThrows(JlensException.class, () -> evaluate(expression, options)).code();
    }

    @Test
    void limits() {
        assertEquals("U1006", code("$sum([1..100])", Options.builder().maxSteps(5).build()));
        assertEquals("D1011", code(LOOP.formatted(100), Options.builder().maxDepth(20).build()));
        assertEquals("U1001", code("$pad('', 100)", Options.builder().maxStringLength(10).build()));
        assertEquals("U1001", code("[1..100]", Options.builder().maxArrayLength(10).build()));
        assertEquals("U1001", code("[1..100]", Options.builder().maxResultSize(10).build()));
        assertEquals("100", evaluate(LOOP.formatted(100), Options.DEFAULTS));
    }

    @Test
    void timeout() {
        Options options = Options.builder().timeout(Duration.ofMillis(10)).build();
        assertEquals("D1012", code("$count([1..10000000].($ * 2))", options));
    }

    @Test
    void clockAndSeed() {
        Options options = Options.builder()
                .now(Instant.parse("2026-10-25T12:00:00Z"))
                .seed(42)
                .build();
        assertEquals("\"2026-10-25T12:00:00.000Z\"", evaluate("$now()", options));
        assertEquals(evaluate("$random()", options), evaluate("$random()", options));
        assertNotEquals(evaluate("$random()", options),
                evaluate("$random()", options.toBuilder().seed(-1L).build()));
    }

    @Test
    void manyDistinctOptions() {
        // Past the options kept as native objects, each lives for one evaluation.
        for (long ms = 0; ms < 200; ms++) {
            Options options = Options.builder().now(Instant.ofEpochMilli(ms)).build();
            assertEquals(Long.toString(ms), evaluate("$millis()", options));
        }
    }

    @Test
    void negativeLimitsAreRefused() {
        assertThrows(IllegalArgumentException.class, () -> Options.builder().maxDepth(-1).build());
        assertThrows(IllegalArgumentException.class,
                () -> Options.builder().timeout(Duration.ofMillis(-1)).build());
    }

    @Test
    void builderRoundTrips() {
        Options options = Options.builder().maxSteps(7).maxDepth(3).build();
        assertEquals(options, options.toBuilder().build());
        assertEquals(Options.DEFAULTS, Options.builder().build());
    }
}
