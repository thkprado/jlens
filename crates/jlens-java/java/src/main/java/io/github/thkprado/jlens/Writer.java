package io.github.thkprado.jlens;

import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

/**
 * Writes Java objects as values for the C API ({@code values.rs} in {@code jlens-capi}), the
 * encoding {@link Values} decodes: the variables of {@link Bindings}, the results of host
 * functions and the arguments of the {@link JsonataFunction}s they call. The objects written are
 * those {@link Expression#evaluate(Object)} reads; any other, a key that is not a string, or a
 * map or list inside itself throws an {@link IllegalArgumentException}. The walk keeps its own
 * stack, so a deep value writes on a thread with a small stack; the C API refuses one nested
 * more than 1,000 levels deep ({@code U1001}).
 */
final class Writer {
    private static final byte UNDEFINED = Values.UNDEFINED;

    private static final VarHandle I32 =
            MethodHandles.byteArrayViewVarHandle(int[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle I64 =
            MethodHandles.byteArrayViewVarHandle(long[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle F64 =
            MethodHandles.byteArrayViewVarHandle(double[].class, ByteOrder.LITTLE_ENDIAN);

    /** What is written, for the messages of its failures. */
    private final String what;
    private byte[] out = new byte[64];
    private int at;
    private final Map<String, Integer> keys = new HashMap<>();

    // The maps and lists being written: their iterators, where their counts go, their counts so
    // far, and for a map the key of the value being written, written after it.
    private Object[] containers = new Object[8];
    private Iterator<?>[] iterators = new Iterator<?>[8];
    private int[] countAt = new int[8];
    private int[] counts = new int[8];
    private String[] pending = new String[8];
    private int depth;

    private Writer(String what) {
        this.what = what;
    }

    /** {@code first}, then {@code value}. */
    static byte[] value(byte first, Object value, String what) {
        Writer writer = new Writer(what);
        writer.out[writer.at++] = first;
        writer.write(value);
        return writer.bytes();
    }

    /** {@code value}, alone. */
    static byte[] value(Object value, String what) {
        Writer writer = new Writer(what);
        writer.write(value);
        return writer.bytes();
    }

    /**
     * The arguments of a call: an array in which {@code null} and a {@link JsonataFunction} are
     * undefined.
     */
    static byte[] arguments(Object[] arguments) {
        Writer writer = new Writer("the arguments of the function");
        writer.room(5);
        writer.out[writer.at++] = Values.ARRAY;
        writer.u32(arguments.length);
        for (int i = 0; i < arguments.length; i++) {
            Object argument = arguments[i];
            if (argument == null || argument instanceof JsonataFunction) {
                writer.room(1);
                writer.out[writer.at++] = UNDEFINED;
            } else {
                writer.write(argument);
            }
        }
        return writer.bytes();
    }

    private byte[] bytes() {
        return Arrays.copyOf(out, at);
    }

    private void write(Object root) {
        Object value = root;
        while (true) {
            if (!open(value)) written();
            // The next value: the next entry of the innermost container not done yet.
            value = null;
            boolean next = false;
            while (depth > 0) {
                int top = depth - 1;
                Iterator<?> iterator = iterators[top];
                if (iterator.hasNext()) {
                    Object entry = iterator.next();
                    counts[top]++;
                    if (containers[top] instanceof Map<?, ?>) {
                        Map.Entry<?, ?> field = (Map.Entry<?, ?>) entry;
                        if (!(field.getKey() instanceof String key)) {
                            throw fail("a key of type " + typeName(field.getKey())
                                    + " is not a string");
                        }
                        pending[top] = key;
                        value = field.getValue();
                    } else {
                        value = entry;
                    }
                    next = true;
                    break;
                }
                I32.set(out, countAt[top], counts[top]);
                containers[top] = null;
                iterators[top] = null;
                depth--;
                written();
            }
            if (!next) return;
        }
    }

    /** After a value: its key, when it is a field. */
    private void written() {
        if (depth == 0 || !(containers[depth - 1] instanceof Map<?, ?>)) return;
        String key = pending[depth - 1];
        Integer index = keys.get(key);
        room(4);
        if (index != null) {
            u32(index);
        } else {
            u32(keys.size());
            keys.put(key, keys.size());
            text(key);
        }
    }

    /** Writes a scalar, or opens a map or list: {@code true} when one was opened. */
    private boolean open(Object value) {
        room(9);
        switch (value) {
            case null -> out[at++] = Values.NULL;
            case String s -> {
                out[at++] = Values.STRING;
                text(s);
            }
            case Boolean b -> out[at++] = b ? Values.TRUE : Values.FALSE;
            case Integer n -> integer(n);
            case Long n -> integer(n);
            case Short n -> integer(n);
            case Byte n -> integer(n);
            case Double n -> number(n);
            case Float n -> number(n);
            case BigInteger n -> number(n.doubleValue());
            case BigDecimal n -> number(n.doubleValue());
            case Map<?, ?> map -> {
                push(map, map.entrySet().iterator(), Values.OBJECT);
                return true;
            }
            case List<?> list -> {
                push(list, list.iterator(), Values.ARRAY);
                return true;
            }
            default -> throw fail("an object of type " + typeName(value) + " is not a JSON value");
        }
        return false;
    }

    private void push(Object container, Iterator<?> iterator, byte tag) {
        for (int up = 0; up < depth; up++) {
            if (containers[up] == container) throw fail("Circular reference detected");
        }
        if (depth == containers.length) {
            int size = depth * 2;
            containers = Arrays.copyOf(containers, size);
            iterators = Arrays.copyOf(iterators, size);
            countAt = Arrays.copyOf(countAt, size);
            counts = Arrays.copyOf(counts, size);
            pending = Arrays.copyOf(pending, size);
        }
        out[at++] = tag;
        containers[depth] = container;
        iterators[depth] = iterator;
        countAt[depth] = at;
        counts[depth] = 0;
        depth++;
        at += 4;
    }

    private void integer(long n) {
        out[at++] = Values.INTEGER;
        I64.set(out, at, n);
        at += 8;
    }

    /** A number; one that is not finite becomes {@code null} in the C API, as JSON has none. */
    private void number(double n) {
        out[at++] = Values.FLOAT;
        F64.set(out, at, n);
        at += 8;
    }

    private void text(String s) {
        byte[] utf8 = s.getBytes(StandardCharsets.UTF_8);
        room(4 + utf8.length);
        u32(utf8.length);
        System.arraycopy(utf8, 0, out, at, utf8.length);
        at += utf8.length;
    }

    private void u32(int n) {
        I32.set(out, at, n);
        at += 4;
    }

    private void room(int more) {
        if (at + more > out.length) out = Arrays.copyOf(out, Math.max(out.length * 2, at + more));
    }

    private IllegalArgumentException fail(String message) {
        return new IllegalArgumentException(what + ": " + message);
    }

    private static String typeName(Object value) {
        return value == null ? "null" : value.getClass().getName();
    }
}
