package io.github.thkprado.jlens;

/**
 * A JSONata function passed to a {@link HostFunction}, such as the lambda in {@code
 * $retry(function() { $fetch() }, 3)}: a lambda, a built-in function, a partial application, a
 * chain or a host function (ADR 0011).
 *
 * {@snippet :
 * // $twice($f, $x) gives $f($f($x)).
 * Bindings bindings = Bindings.builder()
 *         .function("twice", "<fx:x>", args -> {
 *             JsonataFunction f = (JsonataFunction) args.get(0);
 *             return f.call(f.call(args.get(1)));
 *         })
 *         .build();
 * }
 *
 * <p>It can be called while the host function it was passed to runs, on that thread: once the
 * host function has returned, or from another thread, a call throws a {@link JlensException}
 * {@code U1005}.
 */
public final class JsonataFunction {
    private final Host.Call call;
    private final int index;
    private final int arity;

    JsonataFunction(Host.Call call, int index, int arity) {
        this.call = call;
        this.index = index;
        this.arity = arity;
    }

    /** {@return the number of parameters the function declares} */
    public int arity() {
        return arity;
    }

    /**
     * Applies the function, as {@code $map} applies its function: extra arguments are passed on,
     * missing ones are undefined, and the context value is {@code null}. A lambda keeps the
     * variables of its definition. The arguments are written as a host function's result is,
     * {@code null} and a {@code JsonataFunction} as undefined; the result is given as a host
     * function's arguments are.
     *
     * @param arguments the arguments
     * @return the result, {@code null} when it is undefined or {@code null}
     * @throws JlensException when the function fails, with the error's code, or with {@code
     *     U1005} once the host function it was passed to has returned
     * @throws IllegalArgumentException when an argument is not a JSON value
     */
    public Object call(Object... arguments) {
        return call.apply(index, arguments);
    }

    @Override
    public String toString() {
        return "JsonataFunction[arity=" + arity + "]";
    }
}
