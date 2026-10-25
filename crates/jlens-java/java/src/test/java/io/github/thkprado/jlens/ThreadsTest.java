package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

class ThreadsTest {
    @Test
    void virtualThreadsShareAnExpressionAndADocument() throws Exception {
        StringBuilder text = new StringBuilder("{\"items\":[");
        for (int i = 0; i < 100; i++) text.append(i == 0 ? "" : ",").append("{\"price\":").append(i).append('}');
        text.append("]}");
        try (Expression expression = Expression.compile("$count(items[price > 49])");
                Expression twice = Expression.compile("$ * 2");
                Document document = Document.fromJson(text.toString());
                ExecutorService threads = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Boolean>> results = new ArrayList<>();
            for (int t = 0; t < 1000; t++) {
                int n = t;
                results.add(threads.submit(() ->
                        twice.evaluateJson(Integer.toString(n)).equals(Integer.toString(2 * n))
                                && expression.evaluateJson(document).equals("50")));
            }
            for (Future<Boolean> result : results) assertTrue(result.get());
        }
    }

    /**
     * An evaluation on a thread with a small stack stops with {@code D1011} or succeeds, never
     * crashes the JVM (ADR 0021): the C API grows the stack it runs on as it does natively.
     */
    @Test
    void smallStacks() throws Exception {
        String loop = "($f := function($n) { $n = 0 ? 0 : 1 + $f($n - 1) }; $f(%d))";
        for (long stack : new long[] {128 << 10, 256 << 10, 1 << 20}) {
            for (int depth : new int[] {1_000, 10_000, 100_000}) {
                AtomicReference<String> outcome = new AtomicReference<>();
                Thread thread = new Thread(null, () -> {
                    try (Expression expression = Expression.compile(loop.formatted(depth))) {
                        outcome.set(expression.evaluateJson());
                    } catch (JlensException e) {
                        outcome.set(e.code());
                    } catch (Throwable e) {
                        outcome.set(e.toString());
                    }
                }, "small stack", stack);
                thread.start();
                thread.join();
                String expected = depth > 50_000 ? "D1011" : Integer.toString(depth);
                assertEquals(expected, outcome.get(), "stack " + stack + ", depth " + depth);
            }
        }
    }

    /**
     * Without {@code --enable-native-access}, Java prints a warning at the package's first
     * native call, which the installation section of the Java chapter shows; with it, nothing.
     */
    @Test
    void nativeAccessWarning() throws Exception {
        assertTrue(stderr().contains("WARNING: A restricted method"), "warned without the flag");
        assertFalse(stderr("--enable-native-access=ALL-UNNAMED").contains("WARNING"), "quiet with it");
    }

    private static String stderr(String... flags) throws IOException, InterruptedException {
        List<String> command = new ArrayList<>();
        command.add(ProcessHandle.current().info().command().orElseThrow());
        command.addAll(List.of(flags));
        command.add("-cp");
        command.add(System.getProperty("jlens.test.classes") + File.pathSeparator
                + System.getProperty("jlens.test.testClasses"));
        command.add(NativeAccessProbe.class.getName());
        Process process = new ProcessBuilder(command).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
        String stderr = new String(process.getErrorStream().readAllBytes(), StandardCharsets.UTF_8);
        assertEquals(0, process.waitFor(), stderr);
        return stderr;
    }
}
