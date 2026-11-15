package compare;

import io.github.thkprado.jlens.Bindings;
import io.github.thkprado.jlens.Document;
import io.github.thkprado.jlens.Expression;
import java.util.List;
import java.util.Map;

/**
 * The part {@code variables} of {@link Compare}, run only when named: jlens alone, on an
 * expression that reads a large variable, {@code $table}, the spike's document. The variable is
 * bound in {@code Bindings}, built once from Java objects, of which each evaluation converts
 * what the expression reads, and then as a {@code Document}, which no evaluation converts
 * (roadmap 0.35 #7). The input is a small document in both.
 *
 * <p>A class of its own, as the package has had documents as variables since 0.35 only: a
 * comparison with an older JAR ({@code java-compare ab}) runs as long as this part is not named.
 */
final class Variables {
    private Variables() {}

    static void run(int[] sizes) throws Exception {
        System.out.println();
        System.out.println("Microseconds per evaluation of jlens "
            + Engine.VERSIONS.getProperty("jlens") + " on a variable, the spike's documents:");
        System.out.println();
        System.out.println("| Expression | The variable in `Bindings` | The variable a `Document` |");
        System.out.println("|---|---:|---:|");
        for (int n : sizes) {
            String json = Compare.document(n);
            System.out.printf("| %,d items (%,d KB of JSON) | | |%n", n, json.length() / 1000);
            List<String> expressions = List.of("$table.items[0].name",
                "$table.items[id = $$.id].name", "$count($table.items)",
                "$sum($table.items.price)");
            Object objects = Engine.JACKSON.readValue(json, Object.class);
            try (Bindings bindings = Bindings.of(Map.of("table", objects));
                    Document table = Document.fromJson(json);
                    Document input = Document.fromJson("{\"id\": " + n / 2 + "}")) {
                Map<String, Document> documents = Map.of("table", table);
                for (String source : expressions) {
                    try (Expression expression = Expression.compile(source)) {
                        Compare.Op bound = () -> expression.evaluate(input, bindings);
                        Compare.Op document = () -> expression.evaluate(input, documents);
                        String expected = Engine.JACKSON.writeValueAsString(bound.run());
                        String got = Engine.JACKSON.writeValueAsString(document.run());
                        if (!Compare.same(got, expected)) {
                            System.err.println(source + ": " + Compare.shorten(got) + " instead of "
                                + Compare.shorten(expected));
                            System.out.println("| `" + source + "` | differs | differs |");
                            continue;
                        }
                        System.out.println("| `" + source + "` | "
                            + Compare.microseconds(Compare.measure(bound)) + " | "
                            + Compare.microseconds(Compare.measure(document)) + " |");
                    }
                }
            }
        }
    }
}
