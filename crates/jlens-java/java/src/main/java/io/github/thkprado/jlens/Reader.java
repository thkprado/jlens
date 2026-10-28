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
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.RandomAccess;

/**
 * Reads an evaluation's input, Java objects, one level at a time as the C API asks for it
 * ({@code source.rs} in {@code jlens-capi}): a whole map or list, one field of a map, the length
 * of a list or one of its items. Each map and list met is numbered, so that the C API asks for it
 * by its number and a result can give it back unchanged.
 *
 * <p>A map or list that is a value of the level written is written with its contents, so that
 * the C API reads it without calling back: a small one whole when the expression reads it whole,
 * a map with the fields the expression reads (its {@link Shape}, from {@link Expression#reads()})
 * when it reads some. The C API calls back for any other field, so that a result never depends
 * on the fields written.
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

    // The tags of a map or list written with its contents, or some of them.
    private static final byte ARRAY_WITH = 11;
    private static final byte OBJECT_WITH = 12;
    private static final byte OBJECT_SOME = 13;
    /** The most entries of a map or list written with its contents, or fields with some. */
    private static final int SMALL = 16;

    private static final VarHandle I32 =
            MethodHandles.byteArrayViewVarHandle(int[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle I64 =
            MethodHandles.byteArrayViewVarHandle(long[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle F64 =
            MethodHandles.byteArrayViewVarHandle(double[].class, ByteOrder.LITTLE_ENDIAN);

    /**
     * The maps and lists met, by their numbers, the number of the level each is in, and what the
     * expression reads of each ({@code null}: nothing).
     */
    private Object[] nodes = new Object[16];
    private int[] parents = new int[16];
    private Shape[] shapes = new Shape[16];
    private int count;

    /** What the expression reads of the input, {@code null} when it does not read it. */
    private final Shape input;

    /** The level being written, then copied into {@link #buffer}. */
    private byte[] out = new byte[256];
    private int at;

    /** The C API's buffer the levels are copied into, reused from one level to the next. */
    private long buffer;
    private MemorySegment bytes;

    /** A reader of an input of which the expression reads {@code input}. */
    Reader(Shape input) {
        this.input = input;
    }

    /** The maps and lists met, by their numbers: what a result's {@code INPUT} values name. */
    Object[] nodes() {
        return nodes;
    }

    /** Writes the input, a level of one value, and returns the buffer that holds it. */
    long input(Object input) {
        try {
            start(VALUES, 1);
            value(input, -1, this.input, true);
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
        Shape shape = shapes[node];
        switch (op) {
            case READ -> {
                if (value instanceof Map<?, ?> map) {
                    start(FIELDS, map.size());
                    fields(map, node, shape, true);
                } else {
                    List<?> list = (List<?>) value;
                    start(VALUES, list.size());
                    items(list, node, shape, true);
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
                    value(found, node, Shape.field(shape, field), true);
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
                    value(list.get((int) index), node, shape, true);
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

    /** The fields of {@code map}, node {@code node} read as {@code shape}, after their count. */
    private void fields(Map<?, ?> map, int node, Shape shape, boolean with) {
        for (Map.Entry<?, ?> field : map.entrySet()) {
            if (!(field.getKey() instanceof String name)) {
                throw new IllegalArgumentException("a key of type " + typeName(field.getKey())
                        + " is not a string");
            }
            text(name);
            value(field.getValue(), node, Shape.field(shape, name), with);
        }
    }

    /** The items of {@code list}, node {@code node} read as {@code shape}, after their count. */
    private void items(List<?> list, int node, Shape shape, boolean with) {
        for (Object item : list) value(item, node, shape, with);
    }

    /**
     * A value of level {@code parent}, read as {@code shape}. A map or list the expression reads
     * is written {@code with} its contents, or some of them, when it is a value of the level
     * itself, not of the contents of another. Contents that cannot be written, a value that is
     * not JSON or a cycle among them, leave the map or list to be read alone, so that only what
     * the evaluation reads fails.
     */
    private void value(Object value, int parent, Shape shape, boolean with) {
        if (with && shape != null && (value instanceof Map<?, ?> || value instanceof List<?>)) {
            int start = at;
            int nodesBefore = count;
            try {
                contents(value, parent, shape);
                return;
            } catch (RuntimeException e) {
                at = start;
                count = nodesBefore;
            }
        }
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
                I32.set(out, at, node(map, parent, shape));
                at += 4;
            }
            case List<?> list -> {
                out[at++] = Values.ARRAY;
                I32.set(out, at, node(list, parent, shape));
                at += 4;
            }
            default -> throw new Unreadable("an object of type " + typeName(value)
                    + " is not a JSON value");
        }
    }

    /**
     * Writes map or list {@code value}, read as {@code shape}: a map with the fields read when
     * there are few, a map read whole or a list with its contents when it is small, a reference
     * otherwise; throws when the contents cannot be written.
     */
    private void contents(Object value, int parent, Shape shape) {
        room(9);
        if (value instanceof Map<?, ?> map && shape.keys != null && shape.keys.length <= SMALL) {
            out[at] = OBJECT_SOME;
            int node = node(map, parent, shape);
            I32.set(out, at + 1, node);
            I32.set(out, at + 5, shape.keys.length);
            at += 9;
            for (int i = 0; i < shape.keys.length; i++) {
                key(shape.keys[i]);
                Object found = map.get(shape.names[i]);
                if (found == null && !map.containsKey(shape.names[i])) {
                    room(1);
                    out[at++] = Values.UNDEFINED;
                } else {
                    value(found, node, shape.shapes[i], false);
                }
            }
        } else if (value instanceof Map<?, ?> map && shape.keys == null && map.size() <= SMALL) {
            out[at] = OBJECT_WITH;
            int node = node(map, parent, shape);
            I32.set(out, at + 1, node);
            I32.set(out, at + 5, map.size());
            at += 9;
            fields(map, node, shape, false);
        } else if (value instanceof List<?> list && list.size() <= SMALL) {
            out[at] = ARRAY_WITH;
            int node = node(list, parent, shape);
            I32.set(out, at + 1, node);
            I32.set(out, at + 5, list.size());
            at += 9;
            items(list, node, shape, false);
        } else {
            value(value, parent, shape, false);
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
        key(s.getBytes(StandardCharsets.UTF_8));
    }

    /** A text already in UTF-8. */
    private void key(byte[] utf8) {
        room(4 + utf8.length);
        I32.set(out, at, utf8.length);
        at += 4;
        System.arraycopy(utf8, 0, out, at, utf8.length);
        at += utf8.length;
    }

    /**
     * The number of map or list {@code value}, in level {@code parent}, read as {@code shape}: a
     * new one, since a value read again from its level is a new node. A value inside itself
     * fails, as {@code json.dumps} does.
     */
    private int node(Object value, int parent, Shape shape) {
        for (int up = parent; up >= 0; up = parents[up]) {
            if (nodes[up] == value) throw new Unreadable("Circular reference detected");
        }
        if (count == nodes.length) {
            nodes = Arrays.copyOf(nodes, count * 2);
            parents = Arrays.copyOf(parents, count * 2);
            shapes = Arrays.copyOf(shapes, count * 2);
        }
        nodes[count] = value;
        parents[count] = parent;
        shapes[count] = shape;
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

    /**
     * What an expression reads of a value, a {@link Read} made ready for writing levels: whole,
     * or some fields, their keys in UTF-8. {@code null} stands for a value not read.
     */
    static final class Shape {
        /** A value read whole. */
        static final Shape WHOLE = new Shape(null, null, null);

        /** The names of the fields read, their UTF-8 and their shapes; {@code null} when whole. */
        final String[] names;
        final byte[][] keys;
        final Shape[] shapes;
        private final Map<String, Shape> byName = new HashMap<>();

        private Shape(String[] names, byte[][] keys, Shape[] shapes) {
            this.names = names;
            this.keys = keys;
            this.shapes = shapes;
            if (names != null) {
                for (int i = 0; i < names.length; i++) byName.put(names[i], shapes[i]);
            }
        }

        /** The shape of {@code read}, {@code null} for a value not read. */
        static Shape of(Read read) {
            return switch (read) {
                case null -> null;
                case Read.Whole _ -> WHOLE;
                case Read.Fields(Map<String, Read> fields) -> {
                    int n = fields.size();
                    String[] names = fields.keySet().toArray(new String[n]);
                    byte[][] keys = new byte[n][];
                    Shape[] shapes = new Shape[n];
                    for (int i = 0; i < n; i++) {
                        keys[i] = names[i].getBytes(StandardCharsets.UTF_8);
                        shapes[i] = of(fields.get(names[i]));
                    }
                    yield new Shape(names, keys, shapes);
                }
            };
        }

        /** The shape of field {@code name} of a map read as {@code shape}. */
        static Shape field(Shape shape, String name) {
            if (shape == null || shape == WHOLE) return shape;
            return shape.byName.get(name);
        }
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
