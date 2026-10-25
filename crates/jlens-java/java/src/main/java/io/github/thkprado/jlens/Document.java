package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.nio.charset.StandardCharsets;

/**
 * JSON text parsed once, for evaluating several expressions, or one several times, against it.
 * It can be shared between threads: each OS thread converts it once, on its first evaluation,
 * and keeps that conversion until the document is closed.
 *
 * {@snippet :
 * try (Expression expression = Expression.compile("items[0].name");
 *      Document document = Document.fromJson(text, expression.reads().input())) {
 *     String name = expression.evaluateJson(document);
 * }
 * }
 *
 * <p>A document can keep only the fields some expressions read ({@link Expression#reads()},
 * {@link Reads#union}), which saves memory and the time of converting the rest.
 *
 * <p>A document holds native memory: close it when done. One never closed is freed once it
 * becomes unreachable, by a {@link Cleaner}. Once it is closed, each thread drops its conversion
 * at its next call into jlens; a thread that never calls jlens again keeps it until it exits.
 */
public final class Document implements AutoCloseable {
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
        long buffer = Native.buffer(text);
        long readBuffer = read == null ? 0 : Native.buffer(read.toJson());
        try {
            return new Document(Native.handle(Native.documentNew(buffer, readBuffer)));
        } finally {
            Native.free(buffer);
            if (readBuffer != 0) Native.free(readBuffer);
        }
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
