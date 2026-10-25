package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.StringJoiner;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Variables and host functions made available to an expression as {@code $name}. A name holds
 * a value or a {@link HostFunction}: binding one replaces the other. Bindings take precedence
 * over the built-in functions of the same name, as in jsonata-js.
 *
 * {@snippet :
 * try (Bindings bindings = Bindings.builder()
 *                 .value("minimum", 10)
 *                 .function("add", 2, args -> ((Number) args.get(0)).doubleValue()
 *                         + ((Number) args.get(1)).doubleValue())
 *                 .build();
 *         Expression expression = Expression.compile("$add(total, 1) > $minimum")) {
 *     Object passed = expression.evaluate(Map.of("total", 12), bindings);
 *     // passed is Boolean.TRUE
 * }
 * }
 *
 * <p>Bindings are made once and serve any number of evaluations, of any expression, on any
 * number of threads. Their variables are written to native memory when they are built, so an
 * evaluation does not convert them again. An evaluation with no input takes them through {@link
 * Expression#evaluateFromJson(String, Bindings)} given {@code (String) null}.
 *
 * <p>Bindings hold native memory: close them when done. Bindings never closed are freed once
 * they become unreachable, by a {@link Cleaner}. Evaluating with closed bindings throws an {@link
 * IllegalStateException}.
 */
public final class Bindings implements AutoCloseable {
    /** No bindings. Closing them does nothing. */
    public static final Bindings NONE = new Bindings(0, 0, false);

    /** The host functions of the live bindings, by the context the C API calls them with. */
    private static final Map<Long, Function[]> FUNCTIONS = new ConcurrentHashMap<>();
    private static final AtomicLong CONTEXTS = new AtomicLong();

    /** A host function and its name. */
    record Function(String name, HostFunction implementation) {}

    private final long handle;
    /** Whether the bindings hold host functions, which an evaluation then needs a {@link Host} for. */
    final boolean functions;
    private final Cleaner.Cleanable cleanable;
    private volatile boolean closed;

    private Bindings(long handle, long context, boolean functions) {
        this.handle = handle;
        this.functions = functions;
        this.cleanable = handle == 0 ? null : Native.CLEANER.register(this, new Free(handle, context));
    }

    /** Frees the native bindings and forgets their functions; never refers to the {@code Bindings}. */
    private record Free(long handle, long context) implements Runnable {
        @Override
        public void run() {
            FUNCTIONS.remove(context);
            Native.bindingsFree(handle);
        }
    }

    /** {@return a builder of bindings, empty} */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Bindings of variables only.
     *
     * @param values the values, by name, as {@link Builder#value} takes them
     * @return the bindings
     * @throws IllegalArgumentException when a value is not a JSON value
     */
    public static Bindings of(Map<String, ?> values) {
        Builder builder = builder();
        values.forEach(builder::value);
        return builder.build();
    }

    /** The host function {@code index} of the bindings made with {@code context}. */
    static Function function(long context, int index) {
        Function[] functions = FUNCTIONS.get(context);
        if (functions == null) throw new IllegalStateException("jlens: the bindings were closed");
        return functions[index];
    }

    /** The native bindings' handle, 0 for {@link #NONE}. */
    long live() {
        if (closed) throw new IllegalStateException("jlens: the bindings were closed");
        return handle;
    }

    /** Frees the native bindings. Closing them again does nothing. */
    @Override
    public void close() {
        if (cleanable == null) return;
        closed = true;
        cleanable.clean();
    }

    /** Builds {@link Bindings}. A name is given with or without its leading {@code $}. */
    public static final class Builder {
        /** The values, and the host functions as {@link Entry}s, by name. */
        private final Map<String, Object> entries = new LinkedHashMap<>();

        private record Entry(String signature, int arity, HostFunction function) {}

        private Builder() {}

        /**
         * Binds a variable, replacing what {@code name} held.
         *
         * @param name the name
         * @param value the value: a map with string keys, a list, a string, a number, a boolean
         *     or {@code null}, as {@link Expression#evaluate(Object)} reads its input
         * @return this builder
         */
        public Builder value(String name, Object value) {
            put(name, value);
            return this;
        }

        /**
         * Binds a host function without a signature, replacing what {@code name} held. It
         * receives its arguments as the call gives them, however many there are.
         *
         * @param name the name
         * @param arity the number of parameters it declares, which higher-order functions use to
         *     decide which arguments to pass ({@code $map} passes the index to a function of two
         *     or more), and partial application ({@code $f(?, 2)}) to count the parameters
         * @param function the function
         * @return this builder
         * @throws IllegalArgumentException when {@code arity} is negative
         */
        public Builder function(String name, int arity, HostFunction function) {
            if (arity < 0) throw new IllegalArgumentException("arity must not be negative");
            put(name, new Entry(null, arity, Objects.requireNonNull(function, "function")));
            return this;
        }

        /**
         * Binds a host function with a signature, such as {@code <n-n?:n>}, replacing what
         * {@code name} held. Its arguments are checked against the signature before each call,
         * as for built-in functions ({@code T0410}, {@code T0412}); it receives at least one
         * argument per parameter, {@code null} for a missing optional one, and the context value
         * where a parameter has the {@code -} modifier. The signature is checked by {@link
         * #build()}.
         *
         * @param name the name
         * @param signature the signature
         * @param function the function
         * @return this builder
         */
        public Builder function(String name, String signature, HostFunction function) {
            put(name, new Entry(Objects.requireNonNull(signature, "signature"), 0,
                    Objects.requireNonNull(function, "function")));
            return this;
        }

        private void put(String name, Object entry) {
            String key = name.startsWith("$") ? name.substring(1) : name;
            entries.remove(key);
            entries.put(key, entry);
        }

        /**
         * Builds the bindings, writing their variables to native memory.
         *
         * @return the bindings
         * @throws JlensException when a signature is not valid ({@code S0401}, {@code S0402}),
         *     or a value is nested more than 1,000 levels deep ({@code U1001})
         * @throws IllegalArgumentException when a value is not a JSON value, or holds itself
         */
        public Bindings build() {
            Map<String, Object> values = new LinkedHashMap<>();
            StringJoiner specs = new StringJoiner(",", "[", "]");
            List<Function> functions = new ArrayList<>();
            entries.forEach((name, entry) -> {
                if (entry instanceof Entry function) {
                    specs.add("{\"name\":" + Json.quote(name) + (function.signature() == null
                            ? ",\"arity\":" + function.arity()
                            : ",\"signature\":" + Json.quote(function.signature())) + "}");
                    functions.add(new Function(name, function.function()));
                } else {
                    values.put(name, entry);
                }
            });
            byte[] variables = values.isEmpty() ? null : Writer.value(values, "the bindings");
            long context = CONTEXTS.incrementAndGet();
            long specBuffer = functions.isEmpty() ? 0 : Native.buffer(specs.toString());
            long valueBuffer = 0;
            try {
                if (variables != null) valueBuffer = Native.buffer(variables);
                long handle = Native.handle(Native.bindingsNew(specBuffer, valueBuffer, context));
                if (!functions.isEmpty()) FUNCTIONS.put(context, functions.toArray(Function[]::new));
                return new Bindings(handle, context, !functions.isEmpty());
            } finally {
                if (specBuffer != 0) Native.free(specBuffer);
                if (valueBuffer != 0) Native.free(valueBuffer);
            }
        }
    }

    @Override
    public String toString() {
        return "Bindings[" + (handle == 0 ? "none" : functions ? "with functions" : "values") + "]";
    }
}
