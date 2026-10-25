package io.github.thkprado.jlens;

import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.RandomAccess;

/**
 * Reads an evaluation's input, Java objects, one level at a time as the C API asks for it
 * ({@code source.rs} in {@code jlens-capi}): a whole map or list, one field of a map, the length
 * of a list or one of its items. Each map and list met is numbered, so that the C API asks for it
 * by its number and a result can give it back unchanged.
 *
 * <p>A reader serves one evaluation, on the thread that runs it: the C API calls back on that
 * thread, which finds its reader in {@link #CURRENT}. {@link Native#READER} is the function the
 * C API calls, {@link #read}.
 */
final class Reader implements AutoCloseable {
    /** The reader of the evaluation running on this thread. */
    static final ScopedValue<Reader> CURRENT = ScopedValue.newInstance();

    // The operations of the C API's reader.
    private static final int READ = 0;
    private static final int FIELD = 1;
    private static final int LENGTH = 2;
    private static final int ITEM = 3;

    // The kinds of a level.
    private static final byte VALUES = 0;
    private static final byte FIELDS = 1;
    private static final byte FAILED = 2;

    private static final VarHandle I32 =
            MethodHandles.byteArrayViewVarHandle(int[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle I64 =
            MethodHandles.byteArrayViewVarHandle(long[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle F64 =
            MethodHandles.byteArrayViewVarHandle(double[].class, ByteOrder.LITTLE_ENDIAN);

    /** The maps and lists met, by their numbers, and the number of the level each is in. */
    private Object[] nodes = new Object[16];
    private int[] parents = new int[16];
    private int count;

    /** The level being written, then copied into {@link #buffer}. */
    private byte[] out = new byte[256];
    private int at;

    /** The C API's buffer the levels are copied into, reused from one level to the next. */
    private long buffer;
    private MemorySegment bytes;

    /** The maps and lists met, by their numbers: what a result's {@code INPUT} values name. */
    Object[] nodes() {
        return nodes;
    }

    /** Writes the input, a level of one value, and returns the buffer that holds it. */
    long input(Object input) {
        try {
            start(VALUES, 1);
            value(input, -1);
        } catch (RuntimeException e) {
            fail(e);
        }
        return publish();
    }

    /**
     * The C API's call back: a level, or a length. It never throws, since an exception cannot
     * cross into Rust: a failure is written as a level the evaluation fails with ({@code U1004}).
     */
    static long read(long context, int op, int node, long index, long key, long keyLength) {
        Reader reader = CURRENT.isBound() ? CURRENT.get() : null;
        if (reader == null) return 0;
        try {
            return reader.answer(op, node, index, key, keyLength);
        } catch (Throwable e) {
            try {
                reader.fail(e);
                return reader.publish();
            } catch (Throwable again) {
                return 0;
            }
        }
    }

    private long answer(int op, int node, long index, long key, long keyLength) {
        Object value = nodes[node];
        switch (op) {
            case READ -> {
                if (value instanceof Map<?, ?> map) {
                    start(FIELDS, map.size());
                    for (Map.Entry<?, ?> field : map.entrySet()) {
                        if (!(field.getKey() instanceof String name)) {
                            throw new IllegalArgumentException("a key of type "
                                    + typeName(field.getKey()) + " is not a string");
                        }
                        text(name);
                        value(field.getValue(), node);
                    }
                } else {
                    List<?> list = (List<?>) value;
                    start(VALUES, list.size());
                    for (Object item : list) value(item, node);
                }
            }
            case FIELD -> {
                Map<?, ?> map = (Map<?, ?>) value;
                String field = new String(Native.bytesAt(key, keyLength), StandardCharsets.UTF_8);
                Object found = map.get(field);
                if (found == null && !map.containsKey(field)) {
                    start(VALUES, 0);
                } else {
                    start(VALUES, 1);
                    value(found, node);
                }
            }
            // A list that is not random access reads its items in order, whole.
            case LENGTH -> {
                return value instanceof RandomAccess ? ((List<?>) value).size() : -1;
            }
            case ITEM -> {
                List<?> list = (List<?>) value;
                if (index < list.size()) {
                    start(VALUES, 1);
                    value(list.get((int) index), node);
                } else {
                    start(VALUES, 0);
                }
            }
            default -> throw new IllegalStateException("no operation " + op);
        }
        return publish();
    }

    /** Starts a level of {@code kind} holding {@code count} values or fields. */
    private void start(byte kind, int count) {
        at = 4;
        room(5);
        out[at++] = kind;
        I32.set(out, at, count);
        at += 4;
    }

    /** Replaces the level being written with the failure {@code e}. */
    private void fail(Throwable e) {
        at = 4;
        byte[] message = (e instanceof Unreadable ? e.getMessage() : e.toString())
                .getBytes(StandardCharsets.UTF_8);
        room(1 + message.length);
        out[at++] = FAILED;
        System.arraycopy(message, 0, out, at, message.length);
        at += message.length;
    }

    private void value(Object value, int parent) {
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
                out[at++] = Values.OBJECT;
                I32.set(out, at, node(map, parent));
                at += 4;
            }
            case List<?> list -> {
                out[at++] = Values.ARRAY;
                I32.set(out, at, node(list, parent));
                at += 4;
            }
            default -> throw new Unreadable("an object of type " + typeName(value)
                    + " is not a JSON value");
        }
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
        I32.set(out, at, utf8.length);
        at += 4;
        System.arraycopy(utf8, 0, out, at, utf8.length);
        at += utf8.length;
    }

    /**
     * The number of map or list {@code value}, in level {@code parent}: a new one, since a value
     * read again from its level is a new node. A value inside itself fails, as {@code
     * json.dumps} does.
     */
    private int node(Object value, int parent) {
        for (int up = parent; up >= 0; up = parents[up]) {
            if (nodes[up] == value) throw new Unreadable("Circular reference detected");
        }
        if (count == nodes.length) {
            nodes = Arrays.copyOf(nodes, count * 2);
            parents = Arrays.copyOf(parents, count * 2);
        }
        nodes[count] = value;
        parents[count] = parent;
        return count++;
    }

    private void room(int more) {
        if (at + more > out.length) out = Arrays.copyOf(out, Math.max(out.length * 2, at + more));
    }

    /** Copies the level into the C API's buffer, made larger when it does not fit. */
    private long publish() {
        I32.set(out, 0, at - 4);
        if (buffer == 0 || bytes.byteSize() < at) {
            if (buffer != 0) Native.free(buffer);
            buffer = 0;
            long size = Math.max(at, Math.max(4096, bytes == null ? 0 : bytes.byteSize() * 2));
            buffer = Native.bufferNew(size);
            bytes = Native.bufferBytes(buffer, size);
        }
        MemorySegment.copy(out, 0, bytes, ValueLayout.JAVA_BYTE, 0, at);
        return buffer;
    }

    private static String typeName(Object value) {
        return value == null ? "null" : value.getClass().getName();
    }

    /** Frees the buffer. */
    @Override
    public void close() {
        if (buffer != 0) Native.free(buffer);
        buffer = 0;
    }

    /** A value that is not one the reader reads, its message the error's. */
    private static final class Unreadable extends RuntimeException {
        @java.io.Serial
        private static final long serialVersionUID = 1L;

        Unreadable(String message) {
            super(message, null, false, false);
        }
    }
}
