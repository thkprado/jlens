package io.github.thkprado.jlens;

import java.lang.invoke.MethodHandles;
import java.lang.invoke.VarHandle;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Decodes a result written as values by the C API ({@code values.rs} in {@code jlens-capi}):
 * a tag byte per value, integers little-endian, each distinct key once. Objects become {@link
 * LinkedHashMap}s in their order, arrays {@link ArrayList}s, integral numbers in a {@code
 * long}'s range {@link Long}s and the others {@link Double}s. The walk keeps its own stack, so
 * a result nested 1,000 levels deep decodes on a thread with a small stack.
 */
final class Values {
    static final byte NULL = 0;
    static final byte FALSE = 1;
    static final byte TRUE = 2;
    static final byte INTEGER = 3;
    static final byte FLOAT = 4;
    static final byte STRING = 5;
    static final byte ARRAY = 6;
    static final byte OBJECT = 7;

    private static final VarHandle I32 =
            MethodHandles.byteArrayViewVarHandle(int[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle I64 =
            MethodHandles.byteArrayViewVarHandle(long[].class, ByteOrder.LITTLE_ENDIAN);
    private static final VarHandle F64 =
            MethodHandles.byteArrayViewVarHandle(double[].class, ByteOrder.LITTLE_ENDIAN);

    private final byte[] bytes;
    private int at;
    private String[] keys = new String[16];
    private int keyCount;

    private Values(byte[] bytes, int at) {
        this.bytes = bytes;
        this.at = at;
    }

    /** The value written in {@code bytes} from offset {@code at}. */
    static Object decode(byte[] bytes, int at) {
        return new Values(bytes, at).decode();
    }

    @SuppressWarnings("unchecked")
    private Object decode() {
        // The arrays and objects being filled, and how many values each still takes.
        Object[] open = new Object[8];
        int[] remaining = new int[8];
        int depth = 0;
        while (true) {
            byte tag = bytes[at++];
            Object value;
            if (tag == ARRAY || tag == OBJECT) {
                int count = u32();
                Object container = tag == ARRAY
                        ? new ArrayList<>(count)
                        : LinkedHashMap.newLinkedHashMap(count);
                if (count > 0) {
                    if (depth == open.length) {
                        open = Arrays.copyOf(open, depth * 2);
                        remaining = Arrays.copyOf(remaining, depth * 2);
                    }
                    open[depth] = container;
                    remaining[depth++] = count;
                    continue;
                }
                value = container;
            } else {
                value = scalar(tag);
            }
            // Puts the value in its container, and each container it fills in its own.
            while (true) {
                if (depth == 0) return value;
                Object container = open[depth - 1];
                if (container instanceof ArrayList<?> list) {
                    ((List<Object>) list).add(value);
                } else {
                    ((Map<String, Object>) container).put(key(), value);
                }
                if (--remaining[depth - 1] > 0) break;
                open[--depth] = null;
                value = container;
            }
        }
    }

    private Object scalar(byte tag) {
        return switch (tag) {
            case NULL -> null;
            case FALSE -> Boolean.FALSE;
            case TRUE -> Boolean.TRUE;
            case INTEGER -> {
                long value = (long) I64.get(bytes, at);
                at += 8;
                yield value;
            }
            case FLOAT -> {
                double value = (double) F64.get(bytes, at);
                at += 8;
                yield value;
            }
            case STRING -> text();
            default -> throw new IllegalStateException("jlens: unknown value tag " + tag + " at " + (at - 1));
        };
    }

    private String key() {
        int index = u32();
        if (index < keyCount) return keys[index];
        if (index != keyCount) throw new IllegalStateException("jlens: key " + index + " out of order");
        if (keyCount == keys.length) keys = Arrays.copyOf(keys, keyCount * 2);
        return keys[keyCount++] = text();
    }

    private String text() {
        int len = u32();
        String text = new String(bytes, at, len, StandardCharsets.UTF_8);
        at += len;
        return text;
    }

    private int u32() {
        int value = (int) I32.get(bytes, at);
        at += 4;
        return value;
    }
}
