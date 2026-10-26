package io.github.thkprado.jlens;

import java.util.Objects;

/**
 * What an audited evaluation records ({@link Expression#evaluateWithAudit(Object, AuditOptions)}),
 * and within which limits.
 *
 * {@snippet :
 * AuditOptions options = AuditOptions.of(AuditLevel.FULL).withLimits(CaptureLimits.DEFAULTS.withMaxNodes(100));
 * }
 *
 * @param level what is recorded
 * @param limits the bounds on what is captured
 */
public record AuditOptions(AuditLevel level, CaptureLimits limits) {
    /** The defaults: {@link AuditLevel#DECISIONS}, within {@link CaptureLimits#DEFAULTS}. */
    public static final AuditOptions DEFAULTS =
            new AuditOptions(AuditLevel.DECISIONS, CaptureLimits.DEFAULTS);

    /** Checks that neither component is {@code null}. */
    public AuditOptions {
        Objects.requireNonNull(level, "level");
        Objects.requireNonNull(limits, "limits");
    }

    /**
     * The options of {@code level}, within the default limits.
     *
     * @param level what is recorded
     * @return the options
     */
    public static AuditOptions of(AuditLevel level) {
        return new AuditOptions(level, CaptureLimits.DEFAULTS);
    }

    /**
     * These options with another {@link #level()}.
     *
     * @param level what is recorded
     * @return the options
     */
    public AuditOptions withLevel(AuditLevel level) {
        return new AuditOptions(level, limits);
    }

    /**
     * These options with other {@link #limits()}.
     *
     * @param limits the bounds on what is captured
     * @return the options
     */
    public AuditOptions withLimits(CaptureLimits limits) {
        return new AuditOptions(level, limits);
    }

    /** The options as the C API reads them. */
    String toJson() {
        return "{\"level\":\"" + level.json() + "\",\"limits\":" + limits.toJson() + "}";
    }
}
