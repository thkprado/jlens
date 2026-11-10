package io.github.thkprado.jlens;

import java.lang.ref.Cleaner;
import java.lang.ref.Reference;
import java.math.BigDecimal;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.StringJoiner;

/**
 * Checks the text of expressions without evaluating them (ADR 0023): their syntax, the fields
 * they name against a {@link Schema}, and the functions and variables they use against what the
 * application declares. It also tells what an expression uses of its schemas ({@link
 * #uses(Expression)}), to compare when a schema changes.
 *
 * {@snippet :
 * try (Schema order = Schema.fromJson(orderSchema);
 *      Schema customer = Schema.fromJson(customerSchema);
 *      LanguageService service = LanguageService.builder()
 *              .schema(order)
 *              .variable("customer", customer)
 *              .function("discount", "<n:n>")
 *              .build()) {
 *     for (Diagnostic diagnostic : service.diagnostics("$discount(totl) > $customer.limit")) {
 *         // W0201, a warning: the schema has no field "totl"
 *     }
 * }
 * }
 *
 * <p>A service never changes once built, and can be shared between threads, virtual threads
 * included: calls on several threads proceed in parallel. It keeps what it needs of its schemas,
 * which can be closed as soon as it is built. A new version of a schema is a new service.
 *
 * <p>A service holds native memory: close it when done. One never closed is freed once it becomes
 * unreachable, by a {@link Cleaner}. Using a closed service throws an {@link
 * IllegalStateException}.
 */
public final class LanguageService implements AutoCloseable {
    private final long handle;
    private final Cleaner.Cleanable cleanable;
    private volatile boolean closed;

    private LanguageService(long handle) {
        this.handle = handle;
        this.cleanable = Native.CLEANER.register(this, new Free(handle));
    }

    /** Frees the native service; never refers to the {@code LanguageService}. */
    private record Free(long handle) implements Runnable {
        @Override
        public void run() {
            Native.serviceFree(handle);
        }
    }

    /**
     * A service for expressions over the documents a schema describes, with no variable and no
     * host function declared.
     *
     * @param schema the schema of the input, {@code null} for none
     * @return the service
     * @throws IllegalStateException when the schema was closed
     */
    public static LanguageService of(Schema schema) {
        return builder().schema(schema).build();
    }

    /** {@return a builder of a service, with no schema and nothing declared} */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * The problems of an expression's text: its syntax errors, and warnings for a field the
     * schema does not have ({@code W0201}), a function that is not known ({@code W0202}) or is
     * given the wrong number or types of arguments ({@code W0203}, {@code W0205}), a variable
     * that is not declared ({@code W0204}) and a value compared with an enumeration that does
     * not list it ({@code W0206}). A text with a syntax error is not a failure: its errors are
     * diagnostics.
     *
     * @param text the expression's text
     * @return the diagnostics, which cannot be changed; empty when nothing was found
     * @throws IllegalStateException when the service was closed
     */
    public List<Diagnostic> diagnostics(String text) {
        long service = live();
        long buffer = Native.buffer(text);
        String json;
        try {
            json = Native.json(Native.serviceDiagnostics(service, buffer));
        } finally {
            Native.free(buffer);
            Reference.reachabilityFence(this);
        }
        return decode(json);
    }

    /**
     * What an expression uses of the schemas of this service: for each field it reads of its
     * input and of the variables declared, what the schema says there ({@link Use}). An
     * application keeps the answer beside the expression, as {@link Uses#toJson()} writes it,
     * and when a new version of the schema arrives compares it with the answer of the new
     * service ({@link Uses#changes(Uses)}).
     *
     * <p>The answer follows the fields the expression reads ({@link Expression#reads()}), an
     * upper bound: the fields it names, those of a predicate included, and the whole value
     * where the analysis cannot follow it ({@code *}, {@code **}, {@code $eval}, the call of a
     * host function), with every path the schema has under it.
     *
     * @param expression the expression
     * @return what it uses
     * @throws IllegalStateException when the service or the expression was closed
     */
    public Uses uses(Expression expression) {
        long service = live();
        try {
            return Uses.fromJson(Native.json(Native.serviceUses(service, expression.live())));
        } finally {
            Reference.reachabilityFence(expression);
            Reference.reachabilityFence(this);
        }
    }

    /** The diagnostics of the JSON array the C API replies with. */
    private static List<Diagnostic> decode(String json) {
        if (!(Json.parse(json) instanceof List<?> items)) throw malformed(json);
        List<Diagnostic> diagnostics = new ArrayList<>(items.size());
        for (Object item : items) {
            if (!(item instanceof Map<?, ?> fields
                    && fields.get("code") instanceof String code
                    && fields.get("message") instanceof String message
                    && fields.get("span") instanceof Map<?, ?> span
                    && span.get("start") instanceof BigDecimal start
                    && span.get("end") instanceof BigDecimal end)) {
                throw malformed(json);
            }
            Diagnostic.Severity severity = switch (fields.get("severity")) {
                case String name when name.equals("error") -> Diagnostic.Severity.ERROR;
                case String name when name.equals("warning") -> Diagnostic.Severity.WARNING;
                default -> throw malformed(json);
            };
            diagnostics.add(new Diagnostic(severity, code, message,
                    new Span(start.intValueExact(), end.intValueExact())));
        }
        return List.copyOf(diagnostics);
    }

    private static IllegalStateException malformed(String json) {
        return new IllegalStateException("jlens: unexpected diagnostics from the C API: " + json);
    }

    /** Frees the native service. Closing it again does nothing. */
    @Override
    public void close() {
        closed = true;
        cleanable.clean();
    }

    private long live() {
        if (closed) throw new IllegalStateException("jlens: the service was closed");
        return handle;
    }

    /**
     * Builds a {@link LanguageService}. A name is given with or without its leading {@code $};
     * declaring one again replaces what it declared.
     */
    public static final class Builder {
        private Schema schema;
        /** The schema of each variable's value, {@code null} for none, by name. */
        private final Map<String, Schema> variables = new LinkedHashMap<>();
        /** The signature of each host function, {@code null} for none, by name. */
        private final Map<String, String> functions = new LinkedHashMap<>();

        private Builder() {}

        /**
         * Sets the schema of the documents the expressions are evaluated against. Without one,
         * no field is unknown.
         *
         * @param schema the schema, {@code null} for none
         * @return this builder
         */
        public Builder schema(Schema schema) {
            this.schema = schema;
            return this;
        }

        /**
         * Declares a variable the expressions are evaluated with, whose value has no schema:
         * nothing is warned under it. Once a variable is declared, a variable that is neither
         * declared nor assigned by the expression is warned about ({@code W0204}).
         *
         * @param name the name
         * @return this builder
         */
        public Builder variable(String name) {
            return variable(name, null);
        }

        /**
         * Declares a variable the expressions are evaluated with, and the schema of its value,
         * which its fields are checked against.
         *
         * @param name the name
         * @param schema the schema of its value, {@code null} for none
         * @return this builder
         */
        public Builder variable(String name, Schema schema) {
            String key = key(name);
            variables.remove(key);
            variables.put(key, schema);
            return this;
        }

        /**
         * Declares a host function with no signature: a call to it is known, and its arguments
         * are not checked.
         *
         * @param name the name
         * @return this builder
         */
        public Builder function(String name) {
            String key = key(name);
            functions.remove(key);
            functions.put(key, null);
            return this;
        }

        /**
         * Declares a host function with its signature, such as {@code <n-n?:n>}, the one {@link
         * Bindings.Builder#function(String, String, HostFunction)} binds it with: the number
         * and the types of a call's arguments are checked against it.
         *
         * @param name the name
         * @param signature the signature
         * @return this builder
         */
        public Builder function(String name, String signature) {
            String key = key(name);
            functions.remove(key);
            functions.put(key, Objects.requireNonNull(signature, "signature"));
            return this;
        }

        private static String key(String name) {
            return name.startsWith("$") ? name.substring(1) : name;
        }

        /**
         * Builds the service, which copies what it needs of its schemas.
         *
         * @return the service
         * @throws IllegalStateException when a schema was closed
         */
        public LanguageService build() {
            long input = schema == null ? 0 : schema.live();
            StringJoiner members = new StringJoiner(",", "{", "}");
            if (!variables.isEmpty()) {
                StringJoiner json = new StringJoiner(",", "\"variables\":{", "}");
                // A handle is an unsigned number, which a `long` may hold as a negative one.
                variables.forEach((name, value) -> json.add(Json.quote(name) + ":"
                        + (value == null ? "null" : Long.toUnsignedString(value.live()))));
                members.add(json.toString());
            }
            if (!functions.isEmpty()) {
                StringJoiner json = new StringJoiner(",", "\"functions\":[", "]");
                functions.forEach((name, signature) -> json.add("{\"name\":" + Json.quote(name)
                        + (signature == null ? "" : ",\"signature\":" + Json.quote(signature))
                        + "}"));
                members.add(json.toString());
            }
            long declarations = variables.isEmpty() && functions.isEmpty()
                    ? 0 : Native.buffer(members.toString());
            try {
                return new LanguageService(Native.handle(Native.serviceNew(input, declarations)));
            } finally {
                if (declarations != 0) Native.free(declarations);
                Reference.reachabilityFence(schema);
                Reference.reachabilityFence(variables);
            }
        }
    }
}
