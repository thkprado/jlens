package compare;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;

/**
 * How each engine behaves at the limits, as {@code python.py limits} measures the Python
 * packages: input nested deeply, cyclic input, unbounded recursion and an endless loop. Each
 * case runs in a JVM of its own, so that a crash ends that JVM only, killed after {@link #WALL}
 * seconds; the table says what came out: a result, an error code, an exception, the JVM's exit
 * code, or still running.
 */
final class Limits {
    static final int WALL = 30;

    /** How an engine builds an input: an object, a field of it, a number. */
    interface Builder {
        Object object();

        void put(Object object, String key, Object value);

        Object number(int n);
    }

    /** Maps and boxed values, for jlens and jsonata-java. */
    static final Builder MAPS = new Builder() {
        public Object object() { return new LinkedHashMap<String, Object>(); }

        @SuppressWarnings("unchecked")
        public void put(Object object, String key, Object value) {
            ((Map<String, Object>) object).put(key, value);
        }

        public Object number(int n) { return (long) n; }
    };

    interface Input {
        Object build(Builder b);
    }

    /** {@code {"a": {"a": ... 0}}}, {@code depth} objects deep. */
    static Input nested(int depth) {
        return b -> {
            Object value = b.number(0);
            for (int i = 0; i < depth; i++) {
                Object o = b.object();
                b.put(o, "a", value);
                value = o;
            }
            return value;
        };
    }

    /** {@code {"n": 1, "child": {"parent": <itself>}}}: the cycle is one level down. */
    static Object cyclic(Builder b) {
        Object value = b.object();
        b.put(value, "n", b.number(1));
        Object child = b.object();
        b.put(child, "parent", value);
        b.put(value, "child", child);
        return value;
    }

    static final String RECURSION = "($f := function($n){ 1 + $f($n + 1) }; $f(0))";
    static final String LOOP = "($f := function($n){ $f($n + 1) }; $f(0))";

    /** One case: its input (null for none), expression, timeout and depth limit (null for none). */
    record Case(String description, Input input, String expression, Integer timeoutMs, Integer maxDepth) {}

    static final Map<String, Case> CASES = new LinkedHashMap<>();

    static {
        CASES.put("deep-900", new Case("Input nested 900 levels, `$string($)`", nested(900), "$string($)", null, null));
        CASES.put("deep-2000", new Case("Input nested 2,000 levels", nested(2000), "$string($)", null, null));
        CASES.put("deep-100000", new Case("Input nested 100,000 levels", nested(100_000), "$string($)", null, null));
        CASES.put("cycle-field", new Case("Cyclic input, `n`", Limits::cyclic, "n", null, null));
        CASES.put("cycle-string", new Case("Cyclic input, `$string($)`", Limits::cyclic, "$string($)", null, null));
        CASES.put("recursion", new Case("Unbounded recursion, `1 + $f($n + 1)`", null, RECURSION, null, null));
        CASES.put("recursion-depth", new Case("The same, with a depth limit of 500", null, RECURSION, null, 500));
        CASES.put("loop", new Case("Endless tail call, `$f($n + 1)`", null, LOOP, null, null));
        CASES.put("loop-timeout", new Case("The same, with a timeout of 1 s", null, LOOP, 1000, null));
    }

    /** Thrown by an engine that has no such option. */
    static final class Unsupported extends Exception {
        @java.io.Serial
        private static final long serialVersionUID = 1L;
    }

    /** Runs every case of every engine, each in a JVM of its own, and prints the table. */
    static void run(List<Engine> engines) throws Exception {
        engines = engines.stream().filter(e -> !e.key().equals("jlens-objects")).toList();
        System.out.println();
        System.out.printf("How each engine behaves at the limits (each case in a JVM of its own, killed after %d s):%n%n", WALL);
        StringBuilder header = new StringBuilder("| Case |");
        for (Engine e : engines) header.append(" ").append(e.title()).append(" |");
        System.out.println(header);
        System.out.println("|---|" + "---|".repeat(engines.size()));
        String java = ProcessHandle.current().info().command().orElse("java");
        Path errors = Files.createTempDirectory("jlens-limits");
        for (var entry : CASES.entrySet()) {
            StringBuilder row = new StringBuilder("| " + entry.getValue().description() + " |");
            for (Engine e : engines) {
                System.err.println(entry.getKey() + ": " + e.key() + " ...");
                row.append(" ").append(child(java, errors, e.key(), entry.getKey())).append(" |");
            }
            System.out.println(row);
        }
    }

    static String child(String java, Path errors, String engine, String name)
            throws IOException, InterruptedException {
        Path out = Files.createTempFile(errors, name, ".txt");
        Process p = new ProcessBuilder(java, "-Xmx4g", "--enable-native-access=ALL-UNNAMED",
                "-Dstdout.encoding=UTF-8", "-XX:ErrorFile=" + errors + File.separator + "hs_err_%p.log",
                "-cp", System.getProperty("java.class.path"), "compare.Compare", "--limit", engine, name)
            .redirectErrorStream(true)
            .redirectOutput(out.toFile())
            .start();
        if (!p.waitFor(WALL, TimeUnit.SECONDS)) {
            p.destroyForcibly().waitFor();
            return "still running after " + WALL + " s";
        }
        List<String> lines = Files.readAllLines(out, StandardCharsets.UTF_8);
        for (String line : lines.reversed())
            if (line.startsWith("OUTCOME\t")) return line.substring("OUTCOME\t".length());
        String tail = lines.isEmpty() ? "" : lines.getLast();
        System.err.println(engine + " on " + name + ": exit " + p.exitValue() + ", " + tail);
        return String.format("JVM crashes (exit 0x%X)", p.exitValue());
    }

    /** In the child JVM: runs one case and prints its outcome on a line of its own. */
    static void child(String key, String name) {
        Engine engine = Engine.all().stream().filter(e -> e.key().equals(key)).findFirst().orElseThrow();
        Case c = CASES.get(name);
        Object input = c.input() == null ? null : c.input().build(engine.builder());
        long start = System.nanoTime();
        String outcome;
        try {
            String text = engine.json(engine.limited(c.expression(), input, c.timeoutMs(), c.maxDepth()));
            outcome = text == null ? "undefined"
                : text.length() > 40 ? "result" : "`" + text + "`";
        } catch (Unsupported x) {
            outcome = "no such option";
        } catch (Throwable x) {
            String code = engine.code(x);
            if (code != null && code.matches("[A-Z][0-9]{4}")) {
                outcome = "`" + code + "`";
            } else {
                // An exception with no code: its type and the start of its message.
                String message = String.valueOf(x.getMessage()).replaceAll("\\s+", " ");
                outcome = "`" + x.getClass().getSimpleName() + "`" + (x.getMessage() == null ? ""
                    : ": " + (message.length() > 40 ? message.substring(0, 40) + "…" : message));
            }
        }
        double seconds = (System.nanoTime() - start) / 1e9;
        if (c.timeoutMs() != null) outcome += String.format(" after %.1f s", seconds);
        System.out.println("OUTCOME\t" + outcome);
        System.out.flush();
        // A thread an engine left running (a timer, a pool) must not keep the JVM alive.
        System.exit(0);
    }

    private Limits() {}
}
