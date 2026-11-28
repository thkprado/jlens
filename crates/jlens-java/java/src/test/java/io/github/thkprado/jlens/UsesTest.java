package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.math.BigDecimal;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

class UsesTest {
    private static final String ORDER = """
            {"type": "object", "required": ["status"], "properties": {
                "status": {"enum": ["open", null, 1.5, ["a"], {"b": true}]},
                "active": {"type": "boolean"},
                "customer": {"type": "object", "properties": {
                    "name": {"type": "string", "format": "email"},
                    "country": {"type": "string", "x-jlens-domain": "countries"}}}}}""";

    /** A service made of the schemas a fixture gives as parsed JSON. */
    private static LanguageService service(Object schema, Object variables, Object functions) {
        List<Schema> schemas = new ArrayList<>();
        try {
            Schema input = Schema.fromJson(Json.write(schema));
            schemas.add(input);
            LanguageService.Builder builder = LanguageService.builder().schema(input);
            Json.object(variables, "variables").forEach((name, document) -> {
                if (document == null) {
                    builder.variable(name);
                } else {
                    schemas.add(Schema.fromJson(Json.write(document)));
                    builder.variable(name, schemas.getLast());
                }
            });
            for (Object function : (List<?>) functions) {
                Map<String, Object> declared = Json.object(function, "function");
                builder.function((String) declared.get("name"), (String) declared.get("signature"));
            }
            return builder.build();
        } finally {
            schemas.forEach(Schema::close);
        }
    }

    /** The cases every package answers with the same JSON (roadmap 0.33 #7). */
    @Test
    void theSharedCases() throws IOException {
        Path path = Path.of("..", "..", "..", "fixtures", "uses", "cases.json");
        Map<String, Object> fixture = Json.object(Json.parse(Files.readString(path)), "fixture");
        Object functions = fixture.get("functions");
        try (LanguageService old = service(
                        fixture.get("schema"), fixture.get("variables"), functions);
                LanguageService current = service(
                        fixture.get("newSchema"), fixture.get("newVariables"), functions)) {
            for (Object item : (List<?>) fixture.get("cases")) {
                Map<String, Object> expected = Json.object(item, "case");
                String text = (String) expected.get("expression");
                try (Expression expression = Expression.compile(text)) {
                    Uses uses = old.uses(expression);
                    assertEquals(expected.get("uses"), Json.parse(uses.toJson()), text);
                    assertEquals(uses, Uses.fromJson(uses.toJson()), text);
                    UsesChanges changes = uses.changes(current.uses(expression));
                    assertEquals(expected.get("changes"), Json.parse(changes.toJson()), text);
                    assertEquals(changes, UsesChanges.fromJson(changes.toJson()), text);
                    assertEquals(Map.of().equals(expected.get("changes")), changes.isEmpty(), text);
                    assertTrue(uses.changes(uses).isEmpty(), text);
                    assertEquals("{}", uses.changes(uses).toJson(), text);
                }
            }
        }
    }

    /** The fields every package lists with the same JSON (roadmap 0.37 #9). */
    @Test
    void theSharedFields() throws IOException {
        Path path = Path.of("..", "..", "..", "fixtures", "fields", "cases.json");
        Map<String, Object> fixture = Json.object(Json.parse(Files.readString(path)), "fixture");
        LanguageService closed;
        try (Schema schema = Schema.fromJson(Json.write(fixture.get("schema")));
                LanguageService service = LanguageService.of(schema)) {
            List<ListedField> fields = service.fields();
            assertEquals(fixture.get("fields"), fields.stream().map(ListedField::tree).toList());
            ListedField id = fields.stream().filter(f -> f.path().equals("id")).findFirst().get();
            assertEquals(new BigDecimal(1), id.extensions().get("x-attribute-id"));
            assertEquals(Boolean.TRUE, id.required());
            closed = service;
        }
        assertThrows(IllegalStateException.class, closed::fields);
        try (LanguageService alone = LanguageService.builder().build()) {
            assertEquals(List.of(), alone.fields());
        }
    }

    /** The JSON form of a span, as the C API writes it. */
    private static Map<String, Object> span(Span span) {
        Map<String, Object> out = new java.util.LinkedHashMap<>();
        out.put("start", new BigDecimal(span.start()));
        out.put("end", new BigDecimal(span.end()));
        return out;
    }

    /** The expressions every package renames with the same answer (roadmap 0.37 #13). */
    @Test
    void theSharedRenames() throws IOException {
        Path path = Path.of("..", "..", "..", "fixtures", "rename", "cases.json");
        Map<String, Object> fixture = Json.object(Json.parse(Files.readString(path)), "fixture");
        LanguageService closed;
        try (LanguageService service = LanguageService.builder().function("describe").build()) {
            for (Object item : (List<?>) fixture.get("cases")) {
                Map<String, Object> expected = Json.object(item, "case");
                String text = (String) expected.get("expression");
                Map<String, String> renames = new java.util.LinkedHashMap<>();
                Json.object(expected.get("renames"), "renames")
                        .forEach((old, renamed) -> renames.put(old, (String) renamed));
                try (Expression expression = Expression.compile(text)) {
                    Renamed renamed = service.rename(expression, renames);
                    Map<String, Object> tree = new java.util.LinkedHashMap<>();
                    tree.put("text", renamed.text());
                    tree.put("edits", renamed.edits().stream().map(edit -> {
                        Map<String, Object> out = new java.util.LinkedHashMap<>();
                        out.put("span", span(edit.span()));
                        out.put("text", edit.text());
                        return out;
                    }).toList());
                    tree.put("unsure", renamed.unsure().stream().map(place -> {
                        Map<String, Object> out = new java.util.LinkedHashMap<>();
                        out.put("span", span(place.span()));
                        out.put("path", place.path());
                        out.put("reason", place.reason().name().toLowerCase(java.util.Locale.ROOT));
                        return out;
                    }).toList());
                    assertEquals(expected.get("renamed"), tree, text);
                    // A span counts UTF-16 code units, as Java strings do.
                    for (Renamed.Edit edit : renamed.edits()) {
                        String name = text.substring(edit.span().start(), edit.span().end());
                        assertFalse(name.isEmpty() || name.contains(" & "), name);
                    }
                }
            }
            try (Expression expression = Expression.compile("a.b")) {
                JlensException moved = assertThrows(JlensException.class,
                        () -> service.rename(expression, Map.of("a.b", "c.b")));
                assertEquals("U2005", moved.code());
                assertEquals("a.c", service.rename(expression, Map.of("a.b", "a.c")).text());
            }
            closed = service;
        }
        try (Expression expression = Expression.compile("a")) {
            assertThrows(IllegalStateException.class, () -> closed.rename(expression, Map.of()));
        }
    }

    /** The schema of two documents that every package loads (roadmap 0.33 #10). */
    @Test
    void theSharedSchemaOfTwoDocuments() throws IOException {
        Path path = Path.of("..", "..", "..", "fixtures", "uses", "cases.json");
        Map<String, Object> fixture = Json.object(
                Json.object(Json.parse(Files.readString(path)), "fixture").get("references"),
                "references");
        String text = Json.write(fixture.get("schema"));
        Schema.Loader loader = Schema.loader();
        Json.object(fixture.get("documents"), "documents")
                .forEach((uri, document) -> loader.document(uri, Json.write(document)));
        try (Schema schema = loader.load(text);
                Schema alone = Schema.fromJson(text);
                LanguageService service = LanguageService.of(schema);
                LanguageService serviceAlone = LanguageService.of(alone)) {
            assertEquals(fixture.get("report"), report(schema));
            assertEquals(fixture.get("reportAlone"), report(alone));
            assertEquals(
                    List.of("https://example.com/schemas/address.json",
                            "https://example.com/schemas/carrier.json"),
                    schema.missingDocuments());
            assertEquals(
                    List.of("https://example.com/schemas/common.json",
                            "https://example.com/schemas/carrier.json"),
                    alone.missingDocuments());
            for (Object item : (List<?>) fixture.get("cases")) {
                Map<String, Object> expected = Json.object(item, "case");
                String source = (String) expected.get("expression");
                try (Expression expression = Expression.compile(source)) {
                    Uses uses = service.uses(expression);
                    assertEquals(expected.get("uses"), Json.parse(uses.toJson()), source);
                    UsesChanges changes = serviceAlone.uses(expression).changes(uses);
                    assertEquals(expected.get("changes"), Json.parse(changes.toJson()), source);
                }
            }
        }
    }

    /** The recursive schema that every package answers the same about (roadmap 0.33 #12). */
    @Test
    void theSharedRecursiveSchema() throws IOException {
        Path path = Path.of("..", "..", "..", "fixtures", "uses", "cases.json");
        Map<String, Object> fixture = Json.object(
                Json.object(Json.parse(Files.readString(path)), "fixture").get("recursive"),
                "recursive");
        List<?> cases = (List<?>) fixture.get("cases");
        assertFalse(cases.isEmpty());
        try (Schema schema = Schema.fromJson(Json.write(fixture.get("schema")));
                Schema newSchema = Schema.fromJson(Json.write(fixture.get("newSchema")));
                LanguageService old = LanguageService.of(schema);
                LanguageService current = LanguageService.of(newSchema)) {
            for (Object item : cases) {
                Map<String, Object> expected = Json.object(item, "case");
                String source = (String) expected.get("expression");
                try (Expression expression = Expression.compile(source)) {
                    Uses uses = old.uses(expression);
                    assertEquals(expected.get("uses"), Json.parse(uses.toJson()), source);
                    assertEquals(uses, Uses.fromJson(uses.toJson()), source);
                    UsesChanges changes = uses.changes(current.uses(expression));
                    assertEquals(expected.get("changes"), Json.parse(changes.toJson()), source);
                }
            }
        }
    }

    /** What the load of a schema tells, in the JSON form of the other packages. */
    private static Map<String, Object> report(Schema schema) {
        List<Object> unresolved = new ArrayList<>();
        for (UnresolvedRef reference : schema.unresolved()) {
            Map<String, Object> members = new java.util.LinkedHashMap<>();
            members.put("place", reference.place());
            members.put("reference", reference.reference());
            members.put("uri", reference.uri());
            if (reference.document() != null) members.put("document", reference.document());
            unresolved.add(members);
        }
        List<Object> unread = new ArrayList<>();
        for (UnreadKeyword keyword : schema.unread()) {
            unread.add(Map.of("place", keyword.place(), "keyword", keyword.keyword()));
        }
        Map<String, Object> report = new java.util.LinkedHashMap<>();
        if (!unresolved.isEmpty()) report.put("unresolved", unresolved);
        if (!unread.isEmpty()) report.put("unread", unread);
        List<Object> openTuples = new ArrayList<>();
        for (OpenTuple tuple : schema.openTuples()) {
            openTuples.add(Map.of("place", tuple.place()));
        }
        if (!openTuples.isEmpty()) report.put("openTuples", openTuples);
        return report;
    }

    @Test
    void whatTheSchemaSaysOfEachPath() {
        try (Schema schema = Schema.fromJson(ORDER);
                LanguageService service = LanguageService.of(schema);
                Expression expression = Expression.compile(
                        "status = 'open' and customer.name and customer.country and nope")) {
            Uses uses = service.uses(expression);
            assertFalse(uses.everyBinding());
            assertEquals(Map.of(), uses.bindings());
            assertEquals(
                    List.of("$", "customer", "customer.country", "customer.name", "nope", "status"),
                    List.copyOf(uses.input().keySet()));
            Use status = uses.input().get("status");
            assertEquals(
                    Arrays.asList("open", null, new BigDecimal("1.5"), List.of("a"),
                            Map.of("b", true)),
                    status.enumeration());
            assertTrue(status.closed() && status.whole() && status.required());
            // An `enum` with no `type` has the types of its values.
            assertEquals(List.of("null", "number", "string", "array", "object"), status.types());
            Use name = uses.input().get("customer.name");
            assertEquals(List.of("string"), name.types());
            assertEquals("email", name.format());
            assertNull(name.enumeration());
            assertFalse(name.required());
            assertEquals("countries", uses.input().get("customer.country").domain());
            assertEquals(new Use(true, true, List.of(), null, false, null, null, null),
                    uses.input().get("nope"));
            assertNull(uses.input().get("$").required());
            // The answer cannot be changed, and its text gives it back.
            assertThrows(UnsupportedOperationException.class, () -> uses.input().clear());
            assertThrows(UnsupportedOperationException.class, () -> status.enumeration().clear());
            assertThrows(UnsupportedOperationException.class, () -> name.types().clear());
            assertEquals(Uses.fromJson(uses.toJson()), uses);
        }
        // An expression that does not read its input, and one that reads every variable.
        try (LanguageService service = LanguageService.builder().variable("free").build();
                Expression constant = Expression.compile("1 + 1");
                Expression eval = Expression.compile("$eval('1')")) {
            assertEquals(new Uses(null, Map.of(), false), service.uses(constant));
            assertEquals("{\"bindings\":{}}", service.uses(constant).toJson());
            assertTrue(service.uses(eval).everyBinding());
            assertTrue(service.uses(eval).toJson().endsWith(",\"everyBinding\":true}"));
        }
    }

    @Test
    void theChangesOfAnEnumerationAndOfAType() {
        String changed = ORDER
                .replace("[\"open\", null, 1.5, [\"a\"], {\"b\": true}]", "[null, \"open\", \"held\"]")
                .replace("\"boolean\"", "\"string\"")
                .replace("\"required\": [\"status\"], ", "");
        try (Schema before = Schema.fromJson(ORDER);
                Schema after = Schema.fromJson(changed);
                LanguageService old = LanguageService.of(before);
                LanguageService current = LanguageService.of(after);
                Expression expression = Expression.compile("active = true and status = 'open'")) {
            // The answer kept as text is compared with no schema at hand.
            Uses kept = Uses.fromJson(old.uses(expression).toJson());
            UsesChanges changes = kept.changes(current.uses(expression));
            assertFalse(changes.isEmpty());
            assertEquals(Map.of(), changes.bindings());
            assertNull(changes.everyBinding());
            UsesChanges.PathChanges input = changes.input();
            assertTrue(input.added().isEmpty() && input.removed().isEmpty());
            assertEquals(List.of("active", "status"), List.copyOf(input.changed().keySet()));
            UsesChanges.UseChanges active = input.changed().get("active");
            assertEquals(
                    new UsesChanges.Change<>(List.of("boolean"), List.of("string")), active.types());
            assertNull(active.enumeration());
            UsesChanges.UseChanges status = input.changed().get("status");
            assertEquals(new UsesChanges.Change<>(true, false), status.required());
            assertEquals(Arrays.asList(null, "open", "held"), status.enumeration().after());
            assertEquals(List.of("held"), status.enumeration().added());
            assertEquals(
                    List.of(new BigDecimal("1.5"), List.of("a"), Map.of("b", true)),
                    status.enumeration().removed());
            // The types of the values listed, with no `type` beside them.
            assertEquals(
                    new UsesChanges.Change<>(
                            List.of("null", "number", "string", "array", "object"),
                            List.of("null", "string")),
                    status.types());
            assertThrows(UnsupportedOperationException.class, () -> input.changed().clear());
        }
    }

    @Test
    void textThatIsNotAnAnswerIsRefused() {
        for (String json : List.of(
                "{\"input\": ", "[]", "{\"inputs\": {}}", "{\"input\": []}",
                "{\"input\": {\"a\": []}}", "{\"input\": {\"a\": {\"type\": []}}}",
                "{\"input\": {\"a\": {\"types\": [1]}}}", "{\"input\": {\"a\": {\"whole\": 1}}}",
                "{\"bindings\": {\"x\": 1}}", "{\"everyBinding\": \"yes\"}")) {
            assertThrows(IllegalArgumentException.class, () -> Uses.fromJson(json), json);
        }
        for (String json : List.of(
                "{", "[]", "{\"input\": {\"new\": {}}}", "{\"input\": {\"changed\": {\"a\": 1}}}",
                "{\"input\": {\"changed\": {\"a\": {\"types\": {\"old\": [1]}}}}}",
                "{\"everyBinding\": {\"was\": true}}")) {
            assertThrows(IllegalArgumentException.class, () -> UsesChanges.fromJson(json), json);
        }
        assertEquals(new Uses(null, Map.of(), false), Uses.fromJson("{}"));
        assertTrue(UsesChanges.fromJson("{}").isEmpty());
        // A type that JSON Schema does not have is refused by the comparison.
        Uses odd = Uses.fromJson("{\"input\": {\"a\": {\"types\": [\"text\"]}}}");
        JlensException refused = assertThrows(JlensException.class, () -> odd.changes(odd));
        assertEquals("U2005", refused.code());
    }

    @Test
    void aClosedServiceOrExpressionIsRefused() {
        LanguageService service = LanguageService.of(null);
        Expression expression = Expression.compile("a");
        assertEquals(2, service.uses(expression).input().size());
        expression.close();
        assertThrows(IllegalStateException.class, () -> service.uses(expression));
        service.close();
        try (Expression other = Expression.compile("a")) {
            assertThrows(IllegalStateException.class, () -> service.uses(other));
        }
    }
}
