package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.lang.ref.Reference;
import java.nio.charset.StandardCharsets;
import java.util.Map;
import java.util.function.LongFunction;
import java.util.function.LongSupplier;

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
 *
 * <p>Each method takes {@link Bindings} too, the variables and {@link HostFunction}s the
 * expression refers to as {@code $name}. An exception a host function throws ends the
 * evaluation, which throws it again, unchanged.
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
        return evaluateFromJson((byte[]) null, Bindings.NONE, Options.DEFAULTS);
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
        return onDocument(input, Bindings.NONE, options, Native.VALUES, Native::values);
    }

    /**
     * Evaluates the expression against Java objects, read as JSON values one level at a time as
     * the evaluation reaches them: a path reads one entry of a map, an index one item of a list.
     * The objects read are:
     *
     * <ul>
     *   <li>a {@link Map} with {@link String} keys, as an object, its fields in the map's order;
     *   <li>a {@link java.util.List}, as an array;
     *   <li>a {@link String}, a {@link Boolean} and {@code null}, as themselves;
     *   <li>an {@link Integer}, {@link Long}, {@link Short}, {@link Byte}, {@link Double}, {@link
     *       Float}, {@link java.math.BigInteger} or {@link java.math.BigDecimal}, as a number, a
     *       {@code double}; one that is not finite as {@code null}, as JSON has none.
     * </ul>
     *
     * <p>Any other object, a key that is not a string, or a map or list inside itself fails the
     * evaluation with {@code U1004} when it is read. A string is a JSON string, not JSON text:
     * {@link #evaluateFromJson(String)} parses text. A {@link Document} is evaluated as one.
     *
     * <p>A map or list of the input that the result holds unchanged is returned as itself, not
     * copied, as {@code $} returns the input: modifying it modifies the input. The objects must
     * not change during the evaluation.
     *
     * @param input the input
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input cannot be read ({@code U1004}) or the evaluation
     *     fails
     */
    public Object evaluate(Object input) {
        return evaluate(input, Bindings.NONE, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against Java objects, with options, as {@link #evaluate(Object)}
     * does.
     *
     * @param input the input
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input cannot be read ({@code U1004}) or the evaluation
     *     fails
     */
    public Object evaluate(Object input, Options options) {
        return evaluate(input, Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against Java objects, with bindings, as {@link #evaluate(Object)}
     * does.
     *
     * @param input the input
     * @param bindings the variables and host functions of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input cannot be read ({@code U1004}) or the evaluation
     *     fails
     */
    public Object evaluate(Object input, Bindings bindings) {
        return evaluate(input, bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against Java objects, with bindings and options, as {@link
     * #evaluate(Object)} does.
     *
     * @param input the input
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input cannot be read ({@code U1004}) or the evaluation
     *     fails
     */
    public Object evaluate(Object input, Bindings bindings, Options options) {
        if (input instanceof Document document) {
            return onDocument(document, bindings, options, Native.VALUES, Native::values);
        }
        long expression = live();
        long bound = bindings.live();
        try (Reader reader = new Reader()) {
            long level = reader.input(input);
            return run(bindings,
                    () -> ScopedValue.where(Reader.CURRENT, reader).call(() -> options.evaluate(
                            o -> Native.evaluateSource(expression, level, bound, o, Native.VALUES))),
                    reply -> Native.values(reply, reader.nodes()));
        } finally {
            Reference.reachabilityFence(this);
            Reference.reachabilityFence(bindings);
        }
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
        return evaluateFromJson(utf8(input), Bindings.NONE, Options.DEFAULTS);
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
        return evaluateFromJson(utf8(input), Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against JSON text, with bindings.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(String input, Bindings bindings) {
        return evaluateFromJson(utf8(input), bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text, with bindings and options.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(String input, Bindings bindings, Options options) {
        return evaluateFromJson(utf8(input), bindings, options);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(byte[] input) {
        return evaluateFromJson(input, Bindings.NONE, Options.DEFAULTS);
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
        return evaluateFromJson(input, Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with bindings.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(byte[] input, Bindings bindings) {
        return evaluateFromJson(input, bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with bindings and options.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as Java objects, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public Object evaluateFromJson(byte[] input, Bindings bindings, Options options) {
        return onText(input, bindings, options, Native.VALUES, Native::values);
    }

    /**
     * Evaluates the expression with no input.
     *
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson() {
        return evaluateJson((byte[]) null, Bindings.NONE, Options.DEFAULTS);
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
        return evaluateJson(utf8(input), Bindings.NONE, Options.DEFAULTS);
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
        return evaluateJson(utf8(input), Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against JSON text, with bindings.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(String input, Bindings bindings) {
        return evaluateJson(utf8(input), bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text, with bindings and options.
     *
     * @param input the input as JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(String input, Bindings bindings, Options options) {
        return evaluateJson(utf8(input), bindings, options);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(byte[] input) {
        return evaluateJson(input, Bindings.NONE, Options.DEFAULTS);
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
        return evaluateJson(input, Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with bindings.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(byte[] input, Bindings bindings) {
        return evaluateJson(input, bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against JSON text in UTF-8, with bindings and options.
     *
     * @param input the input as UTF-8 JSON text, {@code null} for no input
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the input is not JSON ({@code U2001}) or the evaluation fails
     */
    public String evaluateJson(byte[] input, Bindings bindings, Options options) {
        return onText(input, bindings, options, Native.JSON, Native::json);
    }

    /**
     * Evaluates the expression against a document.
     *
     * @param input the document
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson(Document input) {
        return evaluateJson(input, Bindings.NONE, Options.DEFAULTS);
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
        return evaluateJson(input, Bindings.NONE, options);
    }

    /**
     * Evaluates the expression against a document, with bindings.
     *
     * @param input the document
     * @param bindings the variables and host functions of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson(Document input, Bindings bindings) {
        return evaluateJson(input, bindings, Options.DEFAULTS);
    }

    /**
     * Evaluates the expression against a document, with bindings and options.
     *
     * @param input the document
     * @param bindings the variables and host functions of the evaluation
     * @param options the options of the evaluation
     * @return the result as JSON text, {@code null} when it is undefined
     * @throws JlensException when the evaluation fails
     */
    public String evaluateJson(Document input, Bindings bindings, Options options) {
        return onDocument(input, bindings, options, Native.JSON, Native::json);
    }

    /** Evaluates against UTF-8 JSON text, or no input, in {@code format}, and decodes the reply. */
    private <T> T onText(byte[] input, Bindings bindings, Options options, int format,
            LongFunction<T> decode) {
        long expression = live();
        long bound = bindings.live();
        long buffer = input == null ? 0 : Native.buffer(input);
        try {
            return run(bindings,
                    () -> options.evaluate(o -> Native.evaluate(expression, buffer, bound, o, format)),
                    decode);
        } finally {
            if (buffer != 0) Native.free(buffer);
            Reference.reachabilityFence(this);
            Reference.reachabilityFence(bindings);
        }
    }

    /** Evaluates against a document, in {@code format}, and decodes the reply. */
    private <T> T onDocument(Document input, Bindings bindings, Options options, int format,
            LongFunction<T> decode) {
        long expression = live();
        long document = input.live();
        long bound = bindings.live();
        try {
            return run(bindings,
                    () -> options.evaluate(
                            o -> Native.evaluateDocument(expression, document, bound, o, format)),
                    decode);
        } finally {
            Reference.reachabilityFence(this);
            Reference.reachabilityFence(input);
            Reference.reachabilityFence(bindings);
        }
    }

    /**
     * Runs {@code evaluation} and decodes its reply. With host functions, the evaluation runs
     * with a {@link Host}, and an error that stands for an exception of theirs throws it again.
     */
    private static <T> T run(Bindings bindings, LongSupplier evaluation, LongFunction<T> decode) {
        if (!bindings.functions) return decode.apply(evaluation.getAsLong());
        Host host = new Host();
        long reply = ScopedValue.where(Host.CURRENT, host).call(evaluation::getAsLong);
        try {
            return decode.apply(reply);
        } catch (JlensException e) {
            throw Host.sneaky(host.thrown(e));
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
