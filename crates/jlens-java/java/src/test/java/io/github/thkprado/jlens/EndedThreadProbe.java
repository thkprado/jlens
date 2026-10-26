package io.github.thkprado.jlens;

/**
 * Loads the library on a thread that ends, then evaluates on new threads, one after another,
 * in the JVM that {@link ThreadsTest#loadedOnAThreadThatEnds} starts.
 */
final class EndedThreadProbe {
    private EndedThreadProbe() {}

    public static void main(String[] args) throws InterruptedException {
        Thread loader = new Thread(EndedThreadProbe::evaluate);
        loader.start();
        loader.join();
        for (int i = 0; i < 8; i++) {
            Thread thread = new Thread(EndedThreadProbe::evaluate);
            thread.start();
            thread.join();
        }
    }

    private static void evaluate() {
        try (Expression expression = Expression.compile("$sum(items)")) {
            if (!"3".equals(expression.evaluateJson("{\"items\":[1,2]}"))) System.exit(1);
        }
    }
}
