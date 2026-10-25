package compare;

import com.fasterxml.jackson.databind.JsonNode;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.LongAdder;

/**
 * Compares jlens's Java package with the other JSONata implementations for the JVM, in
 * microseconds per evaluation and in evaluations per second on many threads. Run by
 * {@code cargo xtask java-compare}, in this project's directory; the arguments are the parts to
 * run ({@code documents}, {@code cases}, {@code threads}; all when none is given), then
 * optionally {@code --engines jlens,jsonata-java,...} and {@code --sizes 1000,100000}.
 *
 * <ul>
 *   <li>{@code documents}: the documents of ADR 0021's spike, items with five fields each.
 *   <li>{@code cases}: the workload of {@code benches/cases.json}.
 *   <li>{@code threads}: the spike's throughput, one expression on JSON text, on 1 to all
 *       hardware threads and on 1,000 virtual threads.
 * </ul>
 *
 * <p>Each engine is timed on an input converted once and on JSON text parsed at each call,
 * after its result is checked against jlens's. Each returns its result as its own objects: jlens
 * as maps, lists and boxed values (0.25 #5).
 */
public final class Compare {
    static final Path BENCHES = Path.of("..", "..");

    public static void main(String[] args) throws Exception {
        List<String> parts = new ArrayList<>();
        List<Engine> engines = Engine.all();
        int[] sizes = {1000, 100000};
        for (int i = 0; i < args.length; i++) {
            switch (args[i]) {
                case "--engines" -> {
                    List<String> keys = List.of(args[++i].split(","));
                    engines = engines.stream().filter(e -> keys.contains(e.key())).toList();
                }
                case "--sizes" ->
                    sizes = Arrays.stream(args[++i].split(",")).mapToInt(Integer::parseInt).toArray();
                case "documents", "cases", "threads" -> parts.add(args[i]);
                default -> throw new IllegalArgumentException("unknown argument " + args[i]);
            }
        }
        if (parts.isEmpty()) parts = List.of("documents", "cases", "threads");
        if (engines.isEmpty() || !engines.getFirst().key().equals("jlens"))
            throw new IllegalArgumentException("--engines must include jlens, the reference");
        System.out.printf("%s, %s, %d hardware threads%n", System.getProperty("java.vm.name"),
            Runtime.version(), Runtime.getRuntime().availableProcessors());
        for (String part : parts) {
            switch (part) {
                case "documents" -> documents(engines, sizes);
                case "cases" -> cases(engines);
                default -> threads(engines);
            }
        }
        if (sink == 42) System.out.println();
    }

    // ---- The workloads ------------------------------------------------------------------------

    /** The spike's document: items with an id, a name, a price, two tags and a stock object. */
    static String document(int n) {
        StringBuilder sb = new StringBuilder("{\"items\":[");
        for (int i = 0; i < n; i++) {
            if (i > 0) sb.append(',');
            int cents = (i * 7919) % 10000;
            sb.append("{\"id\":").append(i)
                .append(",\"name\":\"item ").append(i).append('"')
                .append(",\"price\":").append(cents / 100).append('.')
                .append(String.format("%02d", cents % 100))
                .append(",\"tags\":[\"t").append(i % 7).append("\",\"t").append(i % 11).append("\"]")
                .append(",\"stock\":{\"warehouse\":\"w").append(i % 5)
                .append("\",\"qty\":").append(i % 13).append("}}");
        }
        return sb.append("]}").toString();
    }

    static void documents(List<Engine> engines, int[] sizes) throws Exception {
        System.out.println();
        System.out.println("Microseconds per evaluation on the spike's documents:");
        System.out.println();
        header(engines, "Expression");
        for (int n : sizes) {
            String json = document(n);
            System.out.printf("| %,d items (%,d KB of JSON) |%s%n", n, json.length() / 1000,
                " |".repeat(2 * engines.size()));
            List<String> expressions = n <= 1000
                ? List.of("items[0].name", "$count(items)", "$sum(items.price)",
                    "items[price > 98].id", "$")
                : List.of("items[0].name", "$sum(items.price)", "$");
            Map<Engine, Object> converted = new HashMap<>();
            for (Engine e : engines) converted.put(e, e.convert(json));
            for (String expression : expressions)
                row("`" + expression + "`", engines, expression, json, converted);
        }
    }

    static void cases(List<Engine> engines) throws Exception {
        JsonNode file = Engine.JACKSON.readTree(BENCHES.resolve("cases.json").toFile());
        String defaultInput = file.get("input").asText();
        System.out.println();
        System.out.println("Microseconds per evaluation on benches/cases.json:");
        System.out.println();
        header(engines, "Case");
        Map<String, Map<Engine, Object>> converted = new HashMap<>();
        for (JsonNode c : file.get("cases")) {
            String input = c.has("input") ? c.get("input").asText() : defaultInput;
            String json = Files.readString(BENCHES.resolve(input));
            String expression = c.has("expr")
                ? c.get("expr").asText()
                : Files.readString(BENCHES.resolve(c.get("file").asText()));
            Map<Engine, Object> inputs = converted.computeIfAbsent(input, k -> new HashMap<>());
            for (Engine e : engines)
                if (!inputs.containsKey(e)) inputs.put(e, e.convert(json));
            row(c.get("name").asText(), engines, expression, json, inputs);
        }
    }

    static void header(List<Engine> engines, String first) {
        StringBuilder header = new StringBuilder("| " + first + " |");
        for (Engine e : engines)
            header.append(" ").append(e.title()).append(", converted | ")
                .append(e.title()).append(", JSON text |");
        System.out.println(header);
        System.out.println("|---|" + "---:|".repeat(2 * engines.size()));
    }

    /** Checks each engine's result against jlens's, then times those that agree. */
    static void row(String name, List<Engine> engines, String expression, String json,
                    Map<Engine, Object> converted) throws Exception {
        StringBuilder row = new StringBuilder("| " + name + " |");
        String expected = null;
        for (Engine e : engines) {
            Engine.Compiled compiled;
            try {
                compiled = e.compile(expression);
            } catch (Exception | StackOverflowError x) {
                System.err.println(e.key() + " on " + name + ": " + x);
                row.append(" error | error |");
                continue;
            }
            Object input = converted.get(e);
            List<Op> ops = List.of(() -> compiled.evaluate(input), () -> compiled.evaluateText(json));
            for (Op op : ops) {
                String got;
                try {
                    got = e.json(op.run());
                } catch (Exception | StackOverflowError x) {
                    System.err.println(e.key() + " on " + name + ": " + x);
                    row.append(" error |");
                    continue;
                }
                if (e == engines.getFirst() && op == ops.getFirst()) {
                    expected = got;
                } else if (!same(got, expected)) {
                    System.err.println(e.key() + " on " + name + ": " + shorten(got)
                        + " instead of " + shorten(expected));
                    row.append(" differs |");
                    continue;
                }
                double us = measure(op);
                row.append(String.format(us < 100 ? " %,.2f |" : " %,.0f |", us));
            }
        }
        System.out.println(row);
    }

    /** Whether two results are the same JSON, numbers compared by value (2 and 2.0). */
    static boolean same(String a, String b) throws Exception {
        if (a == null || b == null) return a == b;
        Comparator<JsonNode> numbers = (x, y) -> x.isNumber() && y.isNumber()
            ? x.decimalValue().compareTo(y.decimalValue())
            : x.equals(y) ? 0 : 1;
        return Engine.JACKSON.readTree(a).equals(numbers, Engine.JACKSON.readTree(b));
    }

    static String shorten(String s) {
        return s == null || s.length() <= 60 ? String.valueOf(s) : s.substring(0, 57) + "...";
    }

    // ---- Timing -------------------------------------------------------------------------------

    interface Op {
        Object run() throws Exception;
    }

    static long sink;

    static void consume(Object result) {
        if (result != null) sink++;
    }

    /** Microseconds per call: warm up for a second, then the best of five runs of 0.3 s each. */
    static double measure(Op op) throws Exception {
        long start = System.nanoTime();
        int warm = 0;
        while (warm < 3 || System.nanoTime() - start < 1_000_000_000L) {
            consume(op.run());
            warm++;
        }
        double per = (System.nanoTime() - start) / (double) warm;
        int iterations = (int) Math.max(1, 300_000_000L / per);
        double best = Double.MAX_VALUE;
        for (int run = 0; run < 5; run++) {
            long t = System.nanoTime();
            for (int i = 0; i < iterations; i++) consume(op.run());
            best = Math.min(best, (System.nanoTime() - t) / (double) iterations);
        }
        return best / 1000.0;
    }

    // ---- Threads ------------------------------------------------------------------------------

    static final String THREADS_EXPRESSION = "$sum(items.price)";

    static void threads(List<Engine> engines) throws Exception {
        String json = document(100);
        int cores = Runtime.getRuntime().availableProcessors();
        int[] counts = {1, 2, 4, cores / 2, cores};
        System.out.println();
        System.out.printf("Evaluations per second of `%s` on %,d bytes of JSON text:%n%n",
            THREADS_EXPRESSION, json.length());
        StringBuilder header = new StringBuilder("| Engine |");
        for (int c : counts) header.append(" ").append(c).append(c == 1 ? " thread |" : " threads |");
        System.out.println(header + " 1,000 virtual threads |");
        System.out.println("|---|" + "---:|".repeat(counts.length + 1));
        String expected = null;
        for (Engine e : engines) {
            // An engine whose expressions are not thread-safe compiles one per thread.
            Engine.Compiled shared = e.compile(THREADS_EXPRESSION);
            ThreadLocal<Engine.Compiled> local = ThreadLocal.withInitial(() -> {
                try {
                    return e.threadSafe() ? shared : e.compile(THREADS_EXPRESSION);
                } catch (Exception x) {
                    throw new RuntimeException(x);
                }
            });
            Op op = () -> local.get().evaluateText(json);
            String got = e.json(op.run());
            if (expected == null) expected = got;
            if (!same(got, expected)) {
                System.err.println(e.key() + ": " + got + " instead of " + expected);
                continue;
            }
            StringBuilder row = new StringBuilder("| " + e.title() + " |");
            double single = 0;
            for (int c : counts) {
                double r = rate(op, c, false);
                if (c == 1) single = r;
                row.append(String.format(" %,.0f (%.1f×) |", r, r / single));
            }
            double v = rate(op, 1000, true);
            row.append(String.format(" %,.0f (%.1f×) |", v, v / single));
            System.out.println(row);
        }
    }

    /** Evaluations per second over 3 s, after 2 s of warm-up, with {@code threads} workers. */
    static double rate(Op op, int threads, boolean virtual) throws InterruptedException {
        LongAdder done = new LongAdder();
        long warmEnd = System.nanoTime() + 2_000_000_000L, end = warmEnd + 3_000_000_000L;
        Runnable work = () -> {
            try {
                while (true) {
                    consume(op.run());
                    long now = System.nanoTime();
                    if (now >= end) break;
                    if (now >= warmEnd) done.increment();
                }
            } catch (Exception x) {
                throw new RuntimeException(x);
            }
        };
        List<Thread> workers = new ArrayList<>();
        for (int t = 0; t < threads; t++)
            workers.add(virtual ? Thread.ofVirtual().start(work) : Thread.ofPlatform().start(work));
        for (Thread w : workers) w.join();
        return done.sum() / 3.0;
    }
}
