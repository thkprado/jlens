package io.github.thkprado.jlens;

/**
 * Evaluates one expression, in the JVMs that {@link ThreadsTest#nativeAccessWarning} and
 * {@link LoaderTest} start.
 */
final class NativeAccessProbe {
    private NativeAccessProbe() {}

    public static void main(String[] args) {
        try (Expression expression = Expression.compile("1 + 1")) {
            if (!"2".equals(expression.evaluateJson())) System.exit(1);
        }
    }
}
