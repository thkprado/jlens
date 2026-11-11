package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

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
 * <p>A schema whose {@code $ref}s lead to other documents is read by a {@link Loader}, which
 * the application gives those documents, each with its URI: jlens never fetches one. A {@code
 * $ref} that is not followed does not fail the load: its place is unknown, so that nothing is
 * warned under it, and {@link #unresolved()} names it.
 *
 * {@snippet :
 * try (Schema schema = Schema.loader()
 *         .document("https://example.com/schemas/common.json", commonSchemaText)
 *         .load(orderSchemaText)) {
 *     for (String uri : schema.missingDocuments()) {
 *         // a document to give the loader
 *     }
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
    private final List<UnresolvedRef> unresolved;
    private volatile boolean closed;

    private Schema(long handle) {
        this.handle = handle;
        this.cleanable = Native.CLEANER.register(this, new Free(handle));
        try {
            this.unresolved = report(Native.json(Native.schemaReport(handle)));
        } catch (RuntimeException | Error e) {
            cleanable.clean();
            throw e;
        }
    }

    /** The {@code $ref}s of the JSON object the C API replies with. */
    private static List<UnresolvedRef> report(String json) {
        try {
            Map<String, Object> report = Json.object(Json.parse(json), "a load's report");
            List<?> listed = Json.member(report, "unresolved", List.class);
            List<UnresolvedRef> unresolved = new ArrayList<>();
            if (listed != null) {
                for (Object item : listed) unresolved.add(UnresolvedRef.of(item));
            }
            return List.copyOf(unresolved);
        } catch (IllegalArgumentException | NullPointerException e) {
            throw new IllegalStateException(
                    "jlens: unexpected report from the C API: " + json, e);
        }
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
            return new Schema(Native.handle(Native.schemaNew(buffer, 0)));
        } finally {
            Native.free(buffer);
        }
    }

    /** {@return a loader of a schema, with no URI and no other document} */
    public static Loader loader() {
        return new Loader();
    }

    /**
     * The {@code $ref}s that the load did not follow, in the order it met them: those to a
     * document that was not given, and those to a place their document does not have. Their
     * places are unknown to the schema: nothing is warned under them, and what an expression
     * uses there is marked unknown ({@link Use#unknown()}), so that a later load with the
     * document shows as a change. A {@code $ref} that nothing of the schema leads to is not
     * read, and so not listed.
     *
     * <p>The list is read when the schema is made: it is there after the schema is closed.
     *
     * @return the references, which cannot be changed; empty when every one was followed
     */
    public List<UnresolvedRef> unresolved() {
        return unresolved;
    }

    /**
     * The URIs of the documents that were not given, each once, in the order the load met
     * them: with them, a {@link Loader} follows the {@code $ref}s that lead there.
     *
     * @return the URIs, which cannot be changed; empty when no document is missing
     */
    public List<String> missingDocuments() {
        List<String> documents = new ArrayList<>();
        for (UnresolvedRef reference : unresolved) {
            String document = reference.document();
            if (document != null && !documents.contains(document)) documents.add(document);
        }
        return List.copyOf(documents);
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

    /**
     * Reads a JSON Schema whose {@code $ref}s lead to other documents (ADR 0025). The
     * application gives each document with its URI; nothing is fetched.
     *
     * <p>A {@code $ref} is resolved against the {@code $id} of the schema it is in, or else
     * the URI its document was given with, and its fragment is a JSON Pointer or an {@code
     * $anchor}. A document is also known by the {@code $id}s it declares. Two URIs are the
     * same when their texts are equal once resolved.
     *
     * <p>A loader can be used for several loads, and is not safe for use by several threads.
     */
    public static final class Loader {
        private String uri;
        /** The JSON text of each document, in UTF-8, by its URI. */
        private final Map<String, byte[]> documents = new LinkedHashMap<>();

        private Loader() {}

        /**
         * Sets the URI of the schema itself, which its {@code $ref}s are resolved against where
         * no {@code $id} says otherwise. Without one, a reference to another document is taken
         * as written: {@code common.json#/$defs/person} is a place of the document given as
         * {@code common.json}.
         *
         * @param uri the URI, {@code null} for none
         * @return this loader
         */
        public Loader uri(String uri) {
            this.uri = uri;
            return this;
        }

        /**
         * Gives the document that a URI names. Giving one again with the same URI replaces it.
         *
         * @param uri the document's URI
         * @param text the document, as JSON text
         * @return this loader
         */
        public Loader document(String uri, String text) {
            return document(uri, text.getBytes(StandardCharsets.UTF_8));
        }

        /**
         * Gives the document that a URI names, as JSON text in UTF-8. Giving one again with the
         * same URI replaces it.
         *
         * @param uri the document's URI
         * @param text the document, as JSON text in UTF-8, which is not copied: do not change
         *     it before the load
         * @return this loader
         */
        public Loader document(String uri, byte[] text) {
            documents.put(Objects.requireNonNull(uri, "uri"), Objects.requireNonNull(text, "text"));
            return this;
        }

        /**
         * Reads a JSON Schema document, following its {@code $ref}s into the documents given.
         *
         * @param text the JSON Schema, as JSON text
         * @return the schema
         * @throws JlensException when the text, or a document's, is not JSON ({@code U2001}),
         *     or the text is a JSON Schema that jlens refuses ({@code U2005}, with the token
         *     {@code schema})
         */
        public Schema load(String text) {
            return load(text.getBytes(StandardCharsets.UTF_8));
        }

        /**
         * Reads a JSON Schema document in UTF-8, following its {@code $ref}s into the documents
         * given.
         *
         * @param text the JSON Schema, as JSON text in UTF-8
         * @return the schema
         * @throws JlensException when the text, or a document's, is not JSON or not UTF-8
         *     ({@code U2001}), or the text is a JSON Schema that jlens refuses ({@code U2005},
         *     with the token {@code schema})
         */
        public Schema load(byte[] text) {
            List<Long> buffers = new ArrayList<>();
            try {
                buffers.add(Native.buffer(text));
                StringBuilder sources = new StringBuilder("{");
                if (uri != null) sources.append("\"uri\":").append(Json.quote(uri)).append(',');
                sources.append("\"documents\":{");
                for (Map.Entry<String, byte[]> document : documents.entrySet()) {
                    if (buffers.size() > 1) sources.append(',');
                    buffers.add(Native.buffer(document.getValue()));
                    sources.append(Json.quote(document.getKey())).append(':')
                            .append(Long.toUnsignedString(buffers.getLast()));
                }
                buffers.add(Native.buffer(sources.append("}}").toString()));
                return new Schema(
                        Native.handle(Native.schemaNew(buffers.getFirst(), buffers.getLast())));
            } finally {
                buffers.forEach(Native::free);
            }
        }
    }
}
