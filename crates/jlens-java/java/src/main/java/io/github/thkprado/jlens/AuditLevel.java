package io.github.thkprado.jlens;

/** What an audit records ({@link AuditOptions#level()}). */
public enum AuditLevel {
    /** Nothing: the audit tree has a header only. */
    OFF,
    /**
     * The nodes that decide something: comparisons, {@code and} and {@code or}, conditionals,
     * variable bindings, nodes producing booleans, filters, and annotated nodes. The default.
     */
    DECISIONS,
    /** Every evaluated node, with its input. */
    FULL;

    /** {@return the level's name in the C API and in JSON: {@code "off"}, {@code "decisions"} or {@code "full"}} */
    String json() {
        return name().toLowerCase(java.util.Locale.ROOT);
    }

    /** The level of its JSON name. */
    static AuditLevel of(String json) {
        return valueOf(json.toUpperCase(java.util.Locale.ROOT));
    }
}
