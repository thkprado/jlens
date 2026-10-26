package io.github.thkprado.jlens;

import java.util.List;

/**
 * One evaluation of one node of the expression, in an {@link AuditTree}. Captured values ({@link
 * #input()}, {@link #output()}, the operands of a comparison) are Java objects, as {@link
 * Expression#evaluate(Object)} returns results, cut to the {@link CaptureLimits}; {@code null}
 * stands for both {@code null} and undefined.
 *
 * <p>What the node decided, when it decided something, is a {@link Decision}:
 *
 * {@snippet :
 * String why = switch (node.decision()) {
 *     case AuditNode.Comparison c -> c.left() + " against " + c.right();
 *     case AuditNode.ShortCircuit s -> "the right side was not evaluated";
 *     case AuditNode.Conditional c -> "took " + c.branch();
 *     case AuditNode.Binding b -> "bound " + b.variable();
 *     case null -> "no decision";
 * };
 * }
 *
 * @param node the number of the syntax-tree node
 * @param kind the node's type: {@code binary}, {@code path}, {@code function}, {@code
 *     condition}...
 * @param op the operator, function or variable name, {@code null} when there is none
 * @param span where the node is in the expression's source, in UTF-16 code units
 * @param source the node's source text, possibly shortened ({@link CaptureLimits#maxSource()})
 * @param label the {@code @label} of the node's annotation, or {@code null}
 * @param message the {@code @message} of the node's annotation, or {@code null}
 * @param checkId the {@code @id} of the node's annotation, or {@code null}
 * @param input the context value, recorded at {@link AuditLevel#FULL} only
 * @param output the value produced
 * @param outcome how the evaluation of the node ended
 * @param error the error, when {@code outcome} is {@link Outcome#ERROR}, or {@code null}
 * @param decision what the node decided, or {@code null}
 * @param filters the predicate filters applied while evaluating the node
 * @param children the evaluations recorded inside this one, in order
 */
public record AuditNode(
        int node,
        String kind,
        String op,
        Span span,
        String source,
        String label,
        String message,
        String checkId,
        Object input,
        Object output,
        Outcome outcome,
        AuditError error,
        Decision decision,
        List<Filter> filters,
        List<AuditNode> children) {

    /** Copies the lists, unmodifiable. */
    public AuditNode {
        filters = List.copyOf(filters);
        children = List.copyOf(children);
    }

    /**
     * {@return whether the node evaluated to {@code true} ({@code TRUE}), {@code false} ({@code
     * FALSE}), or something else ({@code null})}
     */
    public Boolean passed() {
        return switch (outcome) {
            case TRUE -> Boolean.TRUE;
            case FALSE -> Boolean.FALSE;
            default -> null;
        };
    }

    /**
     * {@return the node without its descendants: its kind, source, outcome and number of
     * children} An audit nests as deep as the evaluation did, so the text of a whole tree could
     * be long, and building it deep.
     */
    @Override
    public String toString() {
        return "AuditNode[node=" + node + ", kind=" + kind + ", source=" + source + ", outcome="
                + outcome + ", children=" + children.size() + "]";
    }

    /** How the evaluation of a node ended. */
    public enum Outcome {
        /** It produced {@code true}. */
        TRUE,
        /** It produced {@code false}. */
        FALSE,
        /** It produced another value. */
        VALUE,
        /** It produced undefined. */
        UNDEFINED,
        /** It raised an error. */
        ERROR
    }

    /** The branch a conditional took. */
    public enum Branch {
        /** The condition held. */
        THEN,
        /** The condition did not hold, and there is an {@code else} part. */
        ELSE,
        /** The condition did not hold, and there is no {@code else} part. */
        NONE
    }

    /** What a node decided: one of four records. */
    public sealed interface Decision permits Comparison, ShortCircuit, Conditional, Binding {}

    /**
     * A comparison ({@code =}, {@code !=}, {@code <}, {@code <=}, {@code >}, {@code >=}, {@code
     * in}) and the values it compared, {@code null} standing for undefined too.
     *
     * @param left the left operand
     * @param right the right operand
     */
    public record Comparison(Object left, Object right) implements Decision {}

    /** An {@code and} or {@code or} whose right side was not evaluated. */
    public record ShortCircuit() implements Decision {}

    /**
     * A conditional ({@code ? :}) and the branch it took.
     *
     * @param branch the branch taken
     */
    public record Conditional(Branch branch) implements Decision {}

    /**
     * A variable binding ({@code $name := value}).
     *
     * @param variable the variable bound, with its {@code $}
     */
    public record Binding(String variable) implements Decision {}

    /**
     * The statistics of a predicate filter ({@code items[price > 10]}).
     *
     * @param node the number of the predicate's syntax-tree node
     * @param source the predicate's source text
     * @param label the {@code @label} of the predicate's annotation, or {@code null}
     * @param tested the items tested
     * @param passed the items that passed
     * @param rejected a sample of the items that did not pass ({@link CaptureLimits#maxRejected()})
     */
    public record Filter(int node, String source, String label, long tested, long passed,
            List<Object> rejected) {
        /** Copies {@code rejected}, unmodifiable; it may hold {@code null}. */
        public Filter {
            rejected = java.util.Collections.unmodifiableList(new java.util.ArrayList<>(rejected));
        }
    }
}
