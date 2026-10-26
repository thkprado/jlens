package io.github.thkprado.jlens;

/**
 * Bounds on what an audit captures, so that a large input gives a bounded audit. A captured
 * array cut short ends with a string {@code "… N more"}, an object with a field {@code "…"},
 * and a string with {@code …}.
 *
 * {@snippet :
 * CaptureLimits limits = CaptureLimits.DEFAULTS.withMaxNodes(100).withMaxItems(3);
 * }
 *
 * @param maxItems the items kept of an array, and the fields of an object, in captured values
 *     (10 by default)
 * @param maxString the characters kept of a string in captured values (200 by default)
 * @param maxDepth the nesting depth kept in captured values (4 by default)
 * @param maxRejected the rejected items kept as samples per filter (5 by default)
 * @param maxNodes the nodes recorded in the tree; past them, nodes are dropped and {@link
 *     AuditTree#truncated()} is set (10,000 by default)
 * @param maxSource the characters of source text kept per node (200 by default)
 */
public record CaptureLimits(
        int maxItems, int maxString, int maxDepth, int maxRejected, int maxNodes, int maxSource) {

    /** The defaults: 10 items, 200 characters, depth 4, 5 rejected items, 10,000 nodes, 200 characters of source. */
    public static final CaptureLimits DEFAULTS = new CaptureLimits(10, 200, 4, 5, 10_000, 200);

    /** Checks that no limit is negative. */
    public CaptureLimits {
        if (maxItems < 0) throw negative("maxItems");
        if (maxString < 0) throw negative("maxString");
        if (maxDepth < 0) throw negative("maxDepth");
        if (maxRejected < 0) throw negative("maxRejected");
        if (maxNodes < 0) throw negative("maxNodes");
        if (maxSource < 0) throw negative("maxSource");
    }

    private static IllegalArgumentException negative(String name) {
        return new IllegalArgumentException(name + " must not be negative");
    }

    /**
     * These limits with another {@link #maxItems()}.
     *
     * @param maxItems the items kept
     * @return the limits
     */
    public CaptureLimits withMaxItems(int maxItems) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /**
     * These limits with another {@link #maxString()}.
     *
     * @param maxString the characters kept
     * @return the limits
     */
    public CaptureLimits withMaxString(int maxString) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /**
     * These limits with another {@link #maxDepth()}.
     *
     * @param maxDepth the depth kept
     * @return the limits
     */
    public CaptureLimits withMaxDepth(int maxDepth) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /**
     * These limits with another {@link #maxRejected()}.
     *
     * @param maxRejected the rejected items kept
     * @return the limits
     */
    public CaptureLimits withMaxRejected(int maxRejected) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /**
     * These limits with another {@link #maxNodes()}.
     *
     * @param maxNodes the nodes recorded
     * @return the limits
     */
    public CaptureLimits withMaxNodes(int maxNodes) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /**
     * These limits with another {@link #maxSource()}.
     *
     * @param maxSource the characters of source kept
     * @return the limits
     */
    public CaptureLimits withMaxSource(int maxSource) {
        return new CaptureLimits(maxItems, maxString, maxDepth, maxRejected, maxNodes, maxSource);
    }

    /** The limits as the C API reads them: JSON text with the WebAssembly package's names. */
    String toJson() {
        return "{\"maxItems\":" + maxItems + ",\"maxString\":" + maxString + ",\"maxDepth\":"
                + maxDepth + ",\"maxRejected\":" + maxRejected + ",\"maxNodes\":" + maxNodes
                + ",\"maxSource\":" + maxSource + "}";
    }
}
