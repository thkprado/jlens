package io.github.thkprado.jlens;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.Future;

/**
 * Runs host functions for the C API ({@code host.rs} in {@code jlens-capi}), and keeps what they
 * threw for one evaluation: the evaluation throws it again once it fails with it.
 *
 * <p>The C API calls {@link #call} back on the evaluating thread, which finds the evaluation's
 * host in {@link #CURRENT}, and the function by the context its {@link Bindings} were made with.
 * An exception gets a number, which the error the evaluation fails with carries back ({@code
 * hostException}). The error of a function a host function called gets a number from the C API
 * ({@code failure}); when the host function lets it through, it replies with that number, so the
 * error keeps its code and position.
 */
final class Host {
    /** The host of the evaluation running on this thread with host functions. */
    static final ScopedValue<Host> CURRENT = ScopedValue.newInstance();

    private static final byte REPLY_UNDEFINED = 0;
    private static final byte REPLY_ERROR = 2;
    private static final byte REPLY_VALUES = 4;

    /** The exceptions the host functions threw, by their numbers. */
    private final List<Throwable> exceptions = new ArrayList<>();
    /** The exceptions thrown by {@link JsonataFunction#call}, and the C API's numbers of their errors. */
    private final Map<Throwable, Long> failures = new IdentityHashMap<>();

    /** A call of a host function in progress, which its {@link JsonataFunction}s call through. */
    static final class Call {
        private final String name;
        private final long frame;
        private final Thread thread;
        private boolean live = true;

        private Call(String name, long frame, Thread thread) {
            this.name = name;
            this.frame = frame;
            this.thread = thread;
        }

        /** Calls argument {@code index}, a function, with {@code arguments}. */
        Object apply(int index, Object[] arguments) {
            if (thread != Thread.currentThread() || !live) throw JlensException.expired(name);
            long buffer = Native.buffer(Writer.arguments(arguments));
            long reply;
            try {
                reply = Native.call(frame, index, buffer);
            } finally {
                Native.free(buffer);
            }
            try {
                return Native.values(reply);
            } catch (JlensException e) {
                if (!CURRENT.isBound()) throw e;
                throw CURRENT.get().raise(e);
            }
        }
    }

    /**
     * The C API's call back: runs function {@code index} of the bindings made with {@code
     * context}, with the arguments in buffer {@code arguments}, and returns the buffer of its
     * reply. It never throws, since an exception cannot cross into Rust: a failure is a reply.
     */
    static long call(long context, int index, long frame, long arguments) {
        try {
            Host host = CURRENT.isBound() ? CURRENT.get() : new Host();
            return Native.buffer(host.run(context, index, frame, arguments));
        } catch (Throwable e) {
            return 0;
        }
    }

    private byte[] run(long context, int index, long frame, long arguments) {
        try {
            Bindings.Function function = Bindings.function(context, index);
            @SuppressWarnings("unchecked")
            List<Object> args = (List<Object>) Values.decode(Native.bytes(arguments), 0);
            Call call = new Call(function.name(), frame, Thread.currentThread());
            for (int i = 0; i < args.size(); i++) {
                if (args.get(i) instanceof Values.Function f) {
                    args.set(i, new JsonataFunction(call, i, f.arity()));
                }
            }
            Object result;
            try {
                result = function.implementation().call(args);
            } finally {
                call.live = false;
            }
            if (result instanceof CompletionStage<?> || result instanceof Future<?>) {
                return error("{\"code\":\"U1002\",\"token\":" + Json.quote(function.name()) + "}");
            }
            if (result == null) return new byte[] {REPLY_UNDEFINED};
            return Writer.value(REPLY_VALUES, result, "the result of $" + function.name());
        } catch (Throwable e) {
            return keep(e);
        }
    }

    /** The reply of exception {@code e}: the failure it stands for, or a new number for it. */
    private byte[] keep(Throwable e) {
        Long failure = failures.get(e);
        if (failure != null) return error("{\"failure\":" + failure + "}");
        exceptions.add(e);
        String code = e instanceof JlensException jlens ? jlens.code() : e.getClass().getSimpleName();
        if (code.isEmpty()) code = e.getClass().getName();
        String message = e.getMessage() == null ? "" : e.getMessage();
        return error("{\"code\":" + Json.quote(code) + ",\"message\":" + Json.quote(message)
                + ",\"exception\":" + (exceptions.size() - 1) + "}");
    }

    private static byte[] error(String envelope) {
        byte[] text = envelope.getBytes(StandardCharsets.UTF_8);
        byte[] reply = new byte[1 + text.length];
        reply[0] = REPLY_ERROR;
        System.arraycopy(text, 0, reply, 1, text.length);
        return reply;
    }

    /**
     * Throws what a {@link JsonataFunction} call failed with: the exception of a host function,
     * or {@code e}, remembering the C API's number of the error.
     */
    private RuntimeException raise(JlensException e) {
        Throwable thrown = thrown(e);
        failures.put(thrown, e.failure());
        throw sneaky(thrown);
    }

    /** The exception a host function threw that {@code e} stands for, or {@code e}. */
    Throwable thrown(JlensException e) {
        int number = e.hostException();
        return number >= 0 && number < exceptions.size() ? exceptions.get(number) : e;
    }

    /** Throws {@code e} as it is, checked or not. */
    @SuppressWarnings("unchecked")
    static <T extends Throwable> RuntimeException sneaky(Throwable e) throws T {
        throw (T) e;
    }
}
