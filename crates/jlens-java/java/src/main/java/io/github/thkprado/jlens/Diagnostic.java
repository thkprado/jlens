package io.github.thkprado.jlens;

/**
 * A problem a {@link LanguageService} found in an expression's text, the same that the other
 * packages, the language server and {@code jlens check} report for it.
 *
 * @param severity whether the expression can be evaluated
 * @param code a JSONata error code ({@code S0xxx}) or a jlens warning code ({@code W0xxx}):
 *     match on it, never on the message
 * @param message the description
 * @param span the part of the text it is about, in UTF-16 code units
 */
public record Diagnostic(Severity severity, String code, String message, Span span) {
    /** How serious a {@link Diagnostic} is. */
    public enum Severity {
        /** The expression cannot be evaluated: a syntax error. */
        ERROR,
        /**
         * The expression can be evaluated, but something looks wrong: a field the schema does
         * not have, a function that is not known, an argument of the wrong type.
         */
        WARNING
    }
}
