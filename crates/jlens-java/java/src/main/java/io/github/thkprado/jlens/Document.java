package io.github.thkprado.jlens;

import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import java.lang.ref.Cleaner;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Map;

/**
 * JSON text parsed once, for evaluating several expressions, or one several times, against it.
 * It can be shared between threads: each OS thread converts it once, on its first evaluation,
 * and keeps that conversion until the document is closed.
 *
 * {@snippet :
 * try (Expression expression = Expression.compile("items[0].name");
 *      Document document = Document.fromJson(text, expression.reads().input())) {
 *     Object name = expression.evaluate(document);
 * }
 * }
 *
 * <p>A document can keep only the fields some expressions read ({@link Expression#reads()},
 * {@link Reads#union}), which saves memory and the time of converting the rest.
 *
 * <p>A document is the value of a variable too, for an expression that reads more than one:
 * {@link Expression#evaluate(Document, Map)} binds documents by name, none of them copied.
 *
 * <p>A document holds native memory: close it when done. One never closed is freed once it
 * becomes unreachable, by a {@link Cleaner}. Once it is closed, each thread drops its conversion
 * at its next call into jlens; a thread that never calls jlens again keeps it until it exits.
 */
public final class Document implements AutoCloseable {
    private static final VarHandle I32 =
            MethodHandles.byteArrayViewVarHandle(int[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle I64 =
            MethodHandles.byteArrayViewVarHandle(long[].class, ByteOrder.LITTLE_ENDIAN);

    private final long handle;
    private final Cleaner.Cleanable cleanable;
    private volatile boolean closed;

    private Document(long handle) {
        this.handle = handle;
        this.cleanable = Native.CLEANER.register(this, new Free(handle));
    }

    /** Frees the native document; never refers to the {@code Document}. */
    private record Free(long handle) implements Runnable {
        @Override
        public void run() {
            Native.documentFree(handle);
        }
    }

    /**
     * Parses JSON text into a document.
     *
     * @param text the JSON text
     * @return the document
     * @throws JlensException when the text is not JSON ({@code U2001})
     */
    public static Document fromJson(String text) {
        return fromJson(text, null);
    }

    /**
     * Parses JSON text into a document, keeping only the fields {@code read} names.
     *
     * @param text the JSON text
     * @param read the fields to keep, {@code null} for all
     * @return the document
     * @throws JlensException when the text is not JSON ({@code U2001})
     */
    public static Document fromJson(String text, Read read) {
        return fromJson(text.getBytes(StandardCharsets.UTF_8), read);
    }

    /**
     * Parses UTF-8 JSON text into a document.
     *
     * @param text the JSON text, in UTF-8
     * @return the document
     * @throws JlensException when the text is not JSON or not UTF-8 ({@code U2001})
     */
    public static Document fromJson(byte[] text) {
        return fromJson(text, null);
    }

    /**
     * Parses UTF-8 JSON text into a document, keeping only the fields {@code read} names.
     *
     * @param text the JSON text, in UTF-8
     * @param read the fields to keep, {@code null} for all
     * @return the document
     * @throws JlensException when the text is not JSON or not UTF-8 ({@code U2001})
     */
    public static Document fromJson(byte[] text, Read read) {
        long readBuffer = read == null ? 0 : Native.buffer(read.toJson());
        try {
            // The document takes the buffer's bytes for its text, rather than a copy.
            return new Document(Native.handle(Native.documentTake(Native.buffer(text), readBuffer)));
        } finally {
            if (readBuffer != 0) Native.free(readBuffer);
        }
    }

    /**
     * The list of the documents an evaluation binds to variables, as the C API reads it ({@code
     * documents.rs} in {@code jlens-capi}): their number, then the handle of each and its name,
     * without the leading {@code $}, in UTF-8.
     *
     * @throws IllegalStateException when a document was closed
     * @throws NullPointerException when a name or a document is {@code null}
     */
    static byte[] listed(Map<String, Document> documents) {
        byte[] list = new byte[64];
        int at = 4;
        int count = 0;
        for (Map.Entry<String, Document> entry : documents.entrySet()) {
            String key = entry.getKey();
            byte[] name = (key.startsWith("$") ? key.substring(1) : key)
                    .getBytes(StandardCharsets.UTF_8);
            long handle = entry.getValue().live();
            int end = at + 12 + name.length;
            if (end > list.length) list = Arrays.copyOf(list, Math.max(end, list.length * 2));
            I64.set(list, at, handle);
            I32.set(list, at + 8, name.length);
            System.arraycopy(name, 0, list, at + 12, name.length);
            at = end;
            count++;
        }
        I32.set(list, 0, count);
        return Arrays.copyOf(list, at);
    }

    /** Frees the native document. Closing it again does nothing. */
    @Override
    public void close() {
        closed = true;
        cleanable.clean();
    }

    long live() {
        if (closed) throw new IllegalStateException("jlens: the document was closed");
        return handle;
    }
}
