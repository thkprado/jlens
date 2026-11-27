package io.github.thkprado.jlens;

/**
 * Evaluates to a reply larger than the heap, in the JVM with a small heap that {@link
 * ThreadsTest#aReplyThatCannotBeCopiedIsFreed} starts: the copy fails, and the reply's buffer
 * is freed all the same. The C API reuses a freed buffer's entry first, so the reply takes the
 * entry of a buffer freed just before, and a buffer made after it takes it again only if the
 * reply was freed.
 */
final class ReplyFreedProbe {
    private ReplyFreedProbe() {}

    public static void main(String[] args) {
        try (Expression large = Expression.compile("[1..3].$pad('x', 9000000)");
                Expression small = Expression.compile("1")) {
            // The thread's scratch buffer, made now, so that the evaluation makes no other.
            small.evaluateJson();
            long before = Native.bufferNew(1);
            Native.free(before);
            try {
                large.evaluateJson();
                System.exit(2);
            } catch (OutOfMemoryError expected) {
                // The copy of the reply.
            }
            long after = Native.bufferNew(1);
            Native.free(after);
            if ((int) after != (int) before) {
                System.err.println("the reply's buffer was not freed");
                System.exit(1);
            }
        }
    }
}
