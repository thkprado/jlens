package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.lang.ref.Reference;
import java.nio.charset.StandardCharsets;

/**
 * A compiled JSONata expression. Compile it once and evaluate it as often as needed, from any
 * number of threads, virtual ones included.
 *
 * {@snippet :
 * try (Expression expression = Expression.compile("$sum(items.price)")) {
 *     String total = expression.evaluateJson("{\"items\": [{\"price\": 2}, {\"price\": 3.5}]}");
 *     // total is "5.5"
 * }
 * }
 *
 * <p>An expression holds native memory: close it when done. One never closed is freed once it
 * becomes unreachable, by a {@link Cleaner}. Evaluating a closed expression throws an {@link
 * IllegalStateException}.
 *
 * <p>Results are JSON text, and {@code null} when the result is undefined (JSONata's "no
 * match"), which differs from the JSON text {@code "null"}.
 */
public final class Expression implements AutoCloseable {
    private final String source;
    private final long handle;
    private final Cleaner.Cleanable cleanable;
    private volatile boolean closed;

    private Expression(String source, long handle) {
        this.source = source;
        this.handle = handle;
        this.cleanable = Native.CLEANER.register(this, new Free(handle));
    }

    /** Frees the native expression; never refers to the {@code Expression}. */
    private record Free(long handle) implements Runnable {
        @Override
        public void run() {
            Native.expressionFree(handle);
        }
    }

    /**
     * Compiles {@code source}.
     *
     * @param source the expression's text
     * @return the compiled expression
     * @throws JlensException when the source is not a valid expression ({@code S0xxx})
     */
    public static Expression compile(String source) {
        long buffer = Native.buffer(source);
        try {
            return new Expression(source, Native.handle(Native.compile(buffer)));
        } finally {
            Native.free(buffer);
        }
    }

    /**
     * The source the expression was compiled from.
     *
     * @return the source
     */
    public String source() {
        return source;
    }

    /**
     * Evaluates the expression with no input.
     *
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson() {
        return evaluateJson((String) null, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text. Only the fields the expression reads are
     * parsed ({@link #reads()}).
     *
     * @param input the input as JSON text, {@code null} for no input
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(String input) {
        return evaluateJson(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text, with options.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(String input, Options options) {
        return evaluateJson(input == null ? null : input.getBytes(StandardCharsets.UTF_8), options);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(byte[] input) {
        return evaluateJson(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with options.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(byte[] input, Options options) {
        long expression = live();
        long buffer = input == null ? 0 : Native.buffer(input);
        try {
            return Native.json(options.evaluate(o -> Native.evaluate(expression, buffer, o)));
        } finally {
            if (buffer != 0) Native.free(buffer);
            Reference.reachabilityFence(this);
        }
    }

    /**
     * Evaluates the expression against a document.
     *
     * @param input the document
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson(Document input) {
        return evaluateJson(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against a document, with options.
     *
     * @param input the document
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson(Document input, Options options) {
        long expression = live();
        long document = input.live();
        try {
            return Native.json(options.evaluate(o -> Native.evaluateDocument(expression, document, o)));
        } finally {
            Reference.reachabilityFence(this);
            Reference.reachabilityFence(input);
        }
    }

    /**
     * What the expression reads of its input and of its bindings (ADR 0016): the fields to keep
     * of a {@link Document} only this expression evaluates.
     *
     * @return the reads
     */
    public Reads reads() {
        try {
            return Reads.fromJson(Native.json(Native.reads(live())));
        } finally {
            Reference.reachabilityFence(this);
        }
    }

    /** Frees the native expression. Closing it again does nothing. */
    @Override
    public void close() {
        closed = true;
        cleanable.clean();
    }

    private long live() {
        if (closed) throw new IllegalStateException("jlens: the expression was closed");
        return handle;
    }

    @Override
    public String toString() {
        return "Expression[" + source + "]";
    }
}
