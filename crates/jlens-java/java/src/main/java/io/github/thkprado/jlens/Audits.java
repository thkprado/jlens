package io.github.thkprado.jlens;

import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * Builds an {@link AuditResult} from the C API's reply ({@code audit.rs} in {@code jlens-capi}),
 * decoded as Java objects: an object with {@code result}, {@code error} (the envelope as JSON
 * text), {@code audit}, {@code explanation} and {@code explanationText}. The nodes are built with
 * a queue of their own, never by recursion, as an audit nests as deep as the evaluation did.
 */
final class Audits {
    private Audits() {}

    /** The result of reply {@code reply}; a host function's exception is found in {@code host}. */
    static AuditResult result(Map<String, Object> reply, Host host) {
        JlensException error = null;
        if (reply.get("error") instanceof String envelope) {
            error = JlensException.fromEnvelope(envelope);
            Throwable thrown = host == null ? error : host.thrown(error);
            if (thrown != error) error.initCause(thrown);
        }
        Map<String, Object> explanation = map(reply.get("explanation"));
        List<Explanation.Line> lines = new ArrayList<>();
        for (Object line : list(explanation.get("lines"))) {
            Map<String, Object> fields = map(line);
            lines.add(new Explanation.Line(
                    integer(fields.get("depth")),
                    (Boolean) fields.get("passed"),
                    (String) fields.get("text"),
                    (String) fields.get("message"),
                    integer(fields.get("node"))));
        }
        return new AuditResult(
                reply.get("result"),
                error,
                tree(map(reply.get("audit"))),
                new Explanation((String) explanation.get("result"), lines,
                        (String) reply.get("explanationText")));
    }

    private static AuditTree tree(Map<String, Object> tree) {
        Map<String, Object> header = map(tree.get("header"));
        double milliseconds = ((Number) header.get("durationMs")).doubleValue();
        return new AuditTree(
                new AuditTree.Header(
                        (String) header.get("engine"),
                        (String) header.get("expressionHash"),
                        (String) header.get("inputHash"),
                        Instant.parse((String) header.get("timestamp")),
                        Duration.ofNanos(Math.round(milliseconds * 1e6)),
                        ((Number) header.get("steps")).longValue(),
                        AuditLevel.of((String) header.get("level")),
                        header.get("result"),
                        error(header.get("error"))),
                tree.get("root") == null ? null : nodes(map(tree.get("root"))),
                integer(tree.get("nodeCount")),
                (Boolean) tree.get("truncated"));
    }

    /**
     * The node of {@code root} and its descendants. They are listed breadth first, so that the
     * children of each node are next to each other, then built from the last, children first.
     */
    private static AuditNode nodes(Map<String, Object> root) {
        List<Map<String, Object>> order = new ArrayList<>();
        List<Integer> firstChild = new ArrayList<>();
        order.add(root);
        for (int i = 0; i < order.size(); i++) {
            firstChild.add(order.size());
            for (Object child : list(order.get(i).get("children"))) order.add(map(child));
        }
        AuditNode[] built = new AuditNode[order.size()];
        for (int i = order.size() - 1; i >= 0; i--) {
            int first = firstChild.get(i);
            int end = i + 1 < order.size() ? firstChild.get(i + 1) : order.size();
            built[i] = node(order.get(i), java.util.Arrays.asList(built).subList(first, end));
        }
        return built[0];
    }

    private static AuditNode node(Map<String, Object> fields, List<AuditNode> children) {
        Map<String, Object> span = map(fields.get("span"));
        List<AuditNode.Filter> filters = new ArrayList<>();
        for (Object filter : list(fields.get("filters"))) {
            Map<String, Object> f = map(filter);
            filters.add(new AuditNode.Filter(
                    integer(f.get("node")),
                    (String) f.get("source"),
                    (String) f.get("label"),
                    ((Number) f.get("tested")).longValue(),
                    ((Number) f.get("passed")).longValue(),
                    list(f.get("rejected"))));
        }
        return new AuditNode(
                integer(fields.get("node")),
                (String) fields.get("kind"),
                (String) fields.get("op"),
                new Span(integer(span.get("start")), integer(span.get("end"))),
                (String) fields.get("source"),
                (String) fields.get("label"),
                (String) fields.get("message"),
                (String) fields.get("checkId"),
                fields.get("input"),
                fields.get("output"),
                AuditNode.Outcome.valueOf(((String) fields.get("outcome")).toUpperCase(java.util.Locale.ROOT)),
                error(fields.get("error")),
                decision(fields),
                filters,
                children);
    }

    /** What a node decided: the core sets one of these fields at most. */
    private static AuditNode.Decision decision(Map<String, Object> fields) {
        if (fields.containsKey("operands")) {
            List<Object> operands = list(fields.get("operands"));
            return new AuditNode.Comparison(operands.get(0), operands.get(1));
        }
        if (Boolean.TRUE.equals(fields.get("shortCircuited"))) return new AuditNode.ShortCircuit();
        if (fields.get("branch") instanceof String branch) {
            return new AuditNode.Conditional(
                    AuditNode.Branch.valueOf(branch.toUpperCase(java.util.Locale.ROOT)));
        }
        if (fields.get("binding") instanceof String variable) return new AuditNode.Binding(variable);
        return null;
    }

    private static AuditError error(Object error) {
        if (error == null) return null;
        Map<String, Object> fields = map(error);
        return new AuditError((String) fields.get("code"), (String) fields.get("message"));
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> map(Object value) {
        return (Map<String, Object>) value;
    }

    @SuppressWarnings("unchecked")
    private static List<Object> list(Object value) {
        return value == null ? List.of() : (List<Object>) value;
    }

    private static int integer(Object value) {
        return Math.toIntExact(((Number) value).longValue());
    }
}
