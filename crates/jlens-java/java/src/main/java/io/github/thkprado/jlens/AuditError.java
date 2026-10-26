package io.github.thkprado.jlens;

/**
 * An error recorded in an audit, by the node that raised it or in the {@link AuditTree.Header}.
 *
 * @param code the JSONata error code, such as {@code T2001}
 * @param message the message
 */
public record AuditError(String code, String message) {}
