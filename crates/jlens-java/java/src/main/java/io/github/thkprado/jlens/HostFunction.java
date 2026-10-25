package io.github.thkprado.jlens;

import java.util.List;

/**
 * A function written in Java, called by an expression as {@code $name(...)} once {@link
 * Bindings} bind it to a name, with an arity or a signature (ADR 0006).
 *
 * {@snippet :
 * Bindings bindings = Bindings.builder()
 *         .function("tax", "<n-n?:n>", args -> {
 *             double amount = ((Number) args.get(0)).doubleValue();
 *             Object rate = args.get(1);
 *             return amount * (rate == null ? 0.25 : ((Number) rate).doubleValue());
 *         })
 *         .build();
 * try (Expression expression = Expression.compile("$tax(200) + price.$tax()")) {
 *     Object total = expression.evaluate(Map.of("price", 8), bindings);
 *     // total is the Long 52
 * }
 * }
 *
 * <p>The arguments are Java objects, as {@link Expression#evaluate(Document)} returns its result:
 * {@link java.util.LinkedHashMap}, {@link java.util.ArrayList}, {@link String}, {@link Long},
 * {@link Double}, {@link Boolean} and {@code null}, which stands for both undefined and {@code
 * null}. A JSONata function among them is a {@link JsonataFunction}, which the host function can
 * call while it runs (ADR 0011). The function does not receive the context of its call: a
 * signature with {@code -} passes it as an argument.
 *
 * <p>The result is read as {@link Expression#evaluate(Object)} reads its input, but whole, when
 * the function returns: maps, lists, strings, numbers, booleans and {@code null}, which makes the
 * result undefined. Any other object, a {@link java.util.concurrent.CompletionStage} or {@link
 * java.util.concurrent.Future} excepted, fails the evaluation with an {@link
 * IllegalArgumentException}; asynchronous functions are not supported, and a function that
 * returns a {@code CompletionStage} or a {@code Future} fails it with {@code U1002}.
 *
 * <p>An exception the function throws stops the evaluation, which throws it again, the same
 * object, unchanged. One a {@link JsonataFunction} threw, let through, does the same.
 */
@FunctionalInterface
public interface HostFunction {
    /**
     * Runs the function.
     *
     * @param arguments the arguments, as many as the call gives; with a signature, at least one
     *     per parameter
     * @return the result, {@code null} for undefined
     */
    Object call(List<Object> arguments);
}
