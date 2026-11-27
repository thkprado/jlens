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
    /**
     * A buffer call that drops a freed document's conversion lets the garbage collector run
     * meanwhile (roadmap 0.37 #4): made {@code critical}, it held every other thread's safepoint
     * until the conversion was dropped.
     */
    @Test
    void aBufferCallThatSweepsHoldsNoGarbageCollection() throws Exception {
        StringBuilder text = new StringBuilder("[");
        for (int i = 0; i < 5_000_000; i++) text.append(i == 0 ? "" : ",").append("{\"a\":").append(i).append('}');
        text.append(']');
        long buffer = Native.buffer(new byte[] {1});
        Document document = Document.fromJson(text.toString());
        text = null;
        java.util.concurrent.CountDownLatch converted = new java.util.concurrent.CountDownLatch(1);
        java.util.concurrent.CountDownLatch freed = new java.util.concurrent.CountDownLatch(1);
        java.util.concurrent.CountDownLatch calling = new java.util.concurrent.CountDownLatch(1);
        long[] sweep = new long[1];
        Thread worker = new Thread(() -> {
            try (Expression expression = Expression.compile("$count($)")) {
                assertEquals("5000000", expression.evaluateJson(document));
                converted.countDown();
                freed.await();
                calling.countDown();
                long start = System.nanoTime();
                // The first call since the free drops this thread's conversion.
                Native.bytes(buffer);
                sweep[0] = System.nanoTime() - start;
            } catch (InterruptedException e) {
                throw new IllegalStateException(e);
            }
        });
        worker.start();
        converted.await();
        document.close();
        freed.countDown();
        calling.await();
        Thread.sleep(5);
        long start = System.nanoTime();
        System.gc();
        long gc = System.nanoTime() - start;
        worker.join();
        Native.free(buffer);
        org.junit.jupiter.api.Assumptions.assumeTrue(sweep[0] > 80_000_000L,
                "the sweep took " + sweep[0] / 1_000_000 + " ms, too short to tell");
        assertTrue(gc < sweep[0] / 2, "the collection took " + gc / 1_000_000
                + " ms, waiting for a sweep of " + sweep[0] / 1_000_000 + " ms");
    }

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
     * The package's own JSON and reads, 1,000 levels deep, on a thread of 128 KiB: read and
     * written without recursion, as values are (roadmap 0.37 #4).
     */
    @Test
    void deepJsonAndReadsOnASmallStack() throws Exception {
        int depth = 1_000;
        String array = "[".repeat(depth) + "]".repeat(depth);
        String object = "{\"a\":".repeat(depth) + "true" + "}".repeat(depth);
        // A document cut by a read, as deep as the C API takes one.
        int steps = 100;
        String path = "a" + ".a".repeat(steps - 1);
        String nested = "{\"a\":".repeat(steps) + "true" + "}".repeat(steps);
        AtomicReference<Throwable> failure = new AtomicReference<>();
        Thread thread = new Thread(null, () -> {
            try {
                assertEquals(array, Json.write(Json.parse(array)));
                assertEquals(object, Json.write(Json.parse(object)));
                assertEquals(java.util.Set.of("e"),
                        Json.members("{\"e\":" + array + "}").keySet());
                Read read = Read.fromJson(object);
                assertEquals(object, read.toJson());
                // The analysis of an expression stops short of such depths; reads made by hand.
                Reads reads = Reads.fromJson("{\"input\":" + object + ",\"bindings\":{\"b\":" + object + "}}");
                Reads again = Reads.fromJson(reads.toJson());
                assertEquals(reads, again);
                assertEquals(reads.hashCode(), again.hashCode());
                assertFalse(reads.equals(Reads.fromJson("{\"input\":" + nested + ",\"bindings\":{}}")));
                assertTrue(reads.toString().length() > depth);
                try (Expression expression = Expression.compile(path);
                        Document document = Document.fromJson(nested, Read.fromJson(nested))) {
                    assertEquals("true", expression.evaluateJson(document));
                }
            } catch (Throwable e) {
                failure.set(e);
            }
        }, "small stack", 128 << 10);
        thread.start();
        thread.join();
        if (failure.get() != null) throw new AssertionError(failure.get());
    }

    /**
     * Without {@code --enable-native-access}, Java prints a warning at the package's first
     * native call, which the installation section of the Java chapter shows; with it, nothing.
     */
    @Test
    void nativeAccessWarning() throws Exception {
        assertTrue(stderr(NativeAccessProbe.class).contains("WARNING: A restricted method"), "warned without the flag");
        assertFalse(stderr(NativeAccessProbe.class, "--enable-native-access=ALL-UNNAMED").contains("WARNING"),
                "quiet with it");
    }

    /**
     * The library loaded on a thread that ends, then used on new threads. mimalloc 3 crashed
     * there on glibc, at the third thread (roadmap 0.25 #11). In a new JVM, since this one has
     * the library loaded already.
     */
    @Test
    void loadedOnAThreadThatEnds() throws Exception {
        stderr(EndedThreadProbe.class, "--enable-native-access=ALL-UNNAMED");
    }

    /** A reply whose copy fails frees its buffer all the same (roadmap 0.37 #4). */
    @Test
    void aReplyThatCannotBeCopiedIsFreed() throws Exception {
        stderr(ReplyFreedProbe.class, "--enable-native-access=ALL-UNNAMED", "-Xmx16m");
    }

    private static String stderr(Class<?> probe, String... flags) throws IOException, InterruptedException {
        List<String> command = new ArrayList<>();
        command.add(ProcessHandle.current().info().command().orElseThrow());
        command.addAll(List.of(flags));
        command.add("-cp");
        command.add(System.getProperty("jlens.test.classes") + File.pathSeparator
                + System.getProperty("jlens.test.testClasses"));
        command.add(probe.getName());
        Process process = new ProcessBuilder(command).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
        String stderr = new String(process.getErrorStream().readAllBytes(), StandardCharsets.UTF_8);
        assertEquals(0, process.waitFor(), stderr);
        return stderr;
    }
}
