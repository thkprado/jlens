package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.nio.charset.StandardCharsets;

/**
 * What a JSON Schema document says of the values some expressions are evaluated against: the
 * fields, their types and their enumerations (ADR 0023). A {@link LanguageService} made of it
 * checks expressions against it, without a document.
 *
 * {@snippet :
 * try (Schema schema = Schema.fromJson(jsonSchemaText);
 *      LanguageService service = LanguageService.of(schema)) {
 *     List<Diagnostic> found = service.diagnostics("items.nme");
 * }
 * }
 *
 * <p>A schema never changes once read, and can be shared between threads. A new version of the
 * JSON Schema is a new {@code Schema}.
 *
 * <p>A schema holds native memory: close it when done. One never closed is freed once it becomes
 * unreachable, by a {@link Cleaner}. A service keeps what it needs of the schemas it was made of,
 * so they can be closed as soon as it is built.
 */
public final class Schema implements AutoCloseable {
    private final long handle;
    private final Cleaner.Cleanable cleanable;
    private volatile boolean closed;

    private Schema(long handle) {
        this.handle = handle;
        this.cleanable = Native.CLEANER.register(this, new Free(handle));
    }

    /** Frees the native schema; never refers to the {@code Schema}. */
    private record Free(long handle) implements Runnable {
        @Override
        public void run() {
            Native.schemaFree(handle);
        }
    }

    /**
     * Reads a JSON Schema document.
     *
     * @param text the JSON Schema, as JSON text
     * @return the schema
     * @throws JlensException when the text is not JSON ({@code U2001}), or is a JSON Schema that
     *     jlens refuses ({@code U2005}, with the token {@code schema})
     */
    public static Schema fromJson(String text) {
        return fromJson(text.getBytes(StandardCharsets.UTF_8));
    }

    /**
     * Reads a JSON Schema document in UTF-8.
     *
     * @param text the JSON Schema, as JSON text in UTF-8
     * @return the schema
     * @throws JlensException when the text is not JSON or not UTF-8 ({@code U2001}), or is a
     *     JSON Schema that jlens refuses ({@code U2005}, with the token {@code schema})
     */
    public static Schema fromJson(byte[] text) {
        long buffer = Native.buffer(text);
        try {
            return new Schema(Native.handle(Native.schemaNew(buffer)));
        } finally {
            Native.free(buffer);
        }
    }

    /** Frees the native schema. Closing it again does nothing. */
    @Override
    public void close() {
        closed = true;
        cleanable.clean();
    }

    long live() {
        if (closed) throw new IllegalStateException("jlens: the schema was closed");
        return handle;
    }
}
