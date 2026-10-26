package io.github.thkprado.jlens;

import java.util.List;

/**
 * Why an evaluation produced its result, from its {@link AuditTree}: for a false {@code and},
 * the operands that failed; for a true {@code or}, those that held; for a conditional, the
 * condition and the branch taken; for a filter, how many items passed and a sample of those
 * rejected.
 *
 * @param result the result, or the error, as text
 * @param lines the lines, in order
 * @param text the explanation as text, as {@code jlens eval --explain} prints it
 */
public record Explanation(String result, List<Line> lines, String text) {
    /** Copies {@code lines}, unmodifiable. */
    public Explanation {
        lines = List.copyOf(lines);
    }

    /** {@return the explanation as text, {@link #text()}} */
    @Override
    public String toString() {
        return text;
    }

    /**
     * One line of an explanation.
     *
     * @param depth the nesting depth, 0 for the reasons at the top
     * @param passed {@code TRUE} for a check that held, {@code FALSE} for one that failed,
     *     {@code null} for a line that is not a check
     * @param text the text of the line
     * @param message the {@code @message} of the annotation, shown for a failed check, or {@code
     *     null}
     * @param node the number of the syntax-tree node the line is about
     */
    public record Line(int depth, Boolean passed, String text, String message, int node) {}
}
