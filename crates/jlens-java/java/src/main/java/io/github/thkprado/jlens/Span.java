package io.github.thkprado.jlens;

/**
 * A range of an expression's source, in UTF-16 code units, as Java strings count them: {@code
 * source.substring(span.start(), span.end())} is its text.
 *
 * @param start the offset of its first code unit
 * @param end the offset one past its last code unit
 */
public record Span(int start, int end) {}
