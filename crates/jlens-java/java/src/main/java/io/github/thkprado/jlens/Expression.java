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
 *     Object total = expression.evaluateFromJson("{\"items\": [{\"price\": 2}, {\"price\": 3.5}]}");
 *     // total is the Double 5.5
 *     String text = expression.evaluateJson("{\"items\": [{\"price\": 2}, {\"price\": 3}]}");
 *     // text is "5"
 * }
 * }
 *
 * <p>An expression holds native memory: close it when done. One never closed is freed once it
 * becomes unreachable, by a {@link Cleaner}. Evaluating a closed expression throws an {@link
 * IllegalStateException}.
 *
 * <p>The {@code evaluate} and {@code evaluateFromJson} methods return the result as Java
 * objects, built for each evaluation, which the caller may modify:
 *
 * <ul>
 *   <li>an object as a {@link java.util.LinkedHashMap}, its fields in their order;
 *   <li>an array as an {@link java.util.ArrayList};
 *   <li>a string as a {@link String};
 *   <li>a number as a {@link Long} when it is integral and in the range of a {@code long},
 *       and as a {@link Double} otherwise ({@code 1.5}, {@code 1e20});
 *   <li>a boolean as a {@link Boolean};
 *   <li>{@code null} as {@code null}.
 * </ul>
 *
 * <p>A result that is undefined (JSONata's "no match") is {@code null} too. The {@code
 * evaluateJson} methods tell them apart: they return the result as JSON text, {@code null} when
 * it is undefined and the text {@code "null"} for {@code null}.
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
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the evaluation fails
     */
    public Object evaluate() {
        return Native.values(onText(null, Options.DEFAULTS, Native.VALUES));
    }

    /**
     * Evaluates the expression against a document.
     *
     * @param input the document
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the evaluation fails
     */
    public Object evaluate(Document input) {
        return evaluate(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against a document, with options.
     *
     * @param input the document
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the evaluation fails
     */
    public Object evaluate(Document input, Options options) {
        return Native.values(onDocument(input, options, Native.VALUES));
    }

    /**
     * Evaluates the expression against JSON text. Only the fields the expression reads are
     * parsed ({@link #reads()}).
     *
     * @param input the input as JSON text, {@code null} for no input
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(String input) {
        return evaluateFromJson(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text, with options.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(String input, Options options) {
        return Native.values(onText(utf8(input), options, Native.VALUES));
    }

    /**
     * Evaluates the expression against JSON text in UTF-8.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(byte[] input) {
        return evaluateFromJson(input, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with options.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(byte[] input, Options options) {
        return Native.values(onText(input, options, Native.VALUES));
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
        return evaluateJson(utf8(input), options);
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
        return Native.json(onText(input, options, Native.JSON));
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
        return Native.json(onDocument(input, options, Native.JSON));
    }

    /** The reply of an evaluation against UTF-8 JSON text, or no input, in {@code format}. */
    private long onText(byte[] input, Options options, int format) {
        long expression = live();
        long buffer = input == null ? 0 : Native.buffer(input);
        try {
            return options.evaluate(o -> Native.evaluate(expression, buffer, o, format));
        } finally {
            if (buffer != 0) Native.free(buffer);
            Reference.reachabilityFence(this);
        }
    }

    /** The reply of an evaluation against a document, in {@code format}. */
    private long onDocument(Document input, Options options, int format) {
        long expression = live();
        long document = input.live();
        try {
            return options.evaluate(o -> Native.evaluateDocument(expression, document, o, format));
        } finally {
            Reference.reachabilityFence(this);
            Reference.reachabilityFence(input);
        }
    }

    private static byte[] utf8(String text) {
        return text == null ? null : text.getBytes(StandardCharsets.UTF_8);
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
