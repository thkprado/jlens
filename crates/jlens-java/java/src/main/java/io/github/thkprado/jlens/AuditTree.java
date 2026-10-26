package io.github.thkprado.jlens;

import java.time.Duration;
import java.time.Instant;

/**
 * The audit of one evaluation: its header, and the tree of the nodes it recorded, in the form
 * of {@code jlens eval --audit json}.
 *
 * @param header what was evaluated, when, and how it ended
 * @param root the root of the recorded tree, {@code null} at {@link AuditLevel#OFF}
 * @param nodeCount the number of nodes recorded
 * @param truncated whether nodes were dropped because of {@link CaptureLimits#maxNodes()}
 */
public record AuditTree(Header header, AuditNode root, int nodeCount, boolean truncated) {

    /**
     * The header of an audit.
     *
     * @param engine {@code jlens-core} and its version
     * @param expressionHash the FNV-1a 64-bit hash of the expression's source, in hexadecimal
     * @param inputHash the FNV-1a 64-bit hash of the input's JSON text, in hexadecimal
     * @param timestamp the time of the evaluation, as {@code $now()} gave it
     * @param duration how long the evaluation took
     * @param steps the steps the evaluation took, which an evaluation without an audit takes
     *     too: the number to compare with {@link Options#maxSteps()}
     * @param level the level the audit was recorded at
     * @param result the result, {@code null} when it is undefined or {@code null}, or when the
     *     evaluation failed
     * @param error the error, when the evaluation failed, or {@code null}
     */
    public record Header(
            String engine,
            String expressionHash,
            String inputHash,
            Instant timestamp,
            Duration duration,
            long steps,
            AuditLevel level,
            Object result,
            AuditError error) {}
}
