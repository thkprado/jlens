package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import org.junit.jupiter.api.Test;

class LanguageServiceTest {
    private static final String ORDER = """
            {"type": "object", "properties": {
                "status": {"type": "string", "enum": ["open", "closed"]},
                "total": {"type": "number"},
                "items": {"type": "array", "items": {"type": "object", "properties": {
                    "name": {"type": "string"}, "price": {"type": "number"}}}}}}""";

    private static final String CUSTOMER = """
            {"type": "object", "properties": {"name": {"type": "string"}}}""";

    private static List<String> codes(List<Diagnostic> diagnostics) {
        return diagnostics.stream().map(Diagnostic::code).toList();
    }

    @Test
    void diagnosticsAreThoseOfTheSchema() {
        try (Schema schema = Schema.fromJson(ORDER);
                LanguageService service = LanguageService.of(schema)) {
            assertEquals(List.of(), service.diagnostics("items[price > 10].name"));
            String text = "items.nme";
            List<Diagnostic> unknown = service.diagnostics(text);
            assertEquals(1, unknown.size());
            Diagnostic diagnostic = unknown.get(0);
            assertEquals(Diagnostic.Severity.WARNING, diagnostic.severity());
            assertEquals("W0201", diagnostic.code());
            assertEquals(new Span(6, 9), diagnostic.span());
            assertEquals("nme", text.substring(diagnostic.span().start(), diagnostic.span().end()));
            assertTrue(diagnostic.message().contains("nme"), diagnostic.message());
            assertEquals(List.of("W0206"), codes(service.diagnostics("status = \"shipped\"")));
            assertEquals(List.of("W0202"), codes(service.diagnostics("$nope(total)")));
            assertEquals(List.of("W0203"), codes(service.diagnostics("$sum()")));
            assertThrows(UnsupportedOperationException.class, () -> unknown.add(diagnostic));
        }
    }

    @Test
    void aSyntaxErrorIsADiagnostic() {
        try (LanguageService service = LanguageService.of(null)) {
            List<Diagnostic> found = service.diagnostics("items[");
            assertTrue(found.stream().anyMatch(d -> d.severity() == Diagnostic.Severity.ERROR
                    && d.code().startsWith("S")), found.toString());
            assertTrue(service.diagnostics("").stream().noneMatch(d -> d.code().startsWith("U")));
            assertEquals(List.of(), service.diagnostics("$sum(a.b)"));
        }
    }

    @Test
    void spansCountUtf16CodeUnits() {
        try (Schema schema = Schema.fromJson(ORDER.getBytes(StandardCharsets.UTF_8));
                LanguageService service = LanguageService.of(schema)) {
            // "é" is one code unit and "😀" two, as in a String.
            String text = "\"é😀\" & items.nme";
            Span span = service.diagnostics(text).get(0).span();
            assertEquals(new Span(14, 17), span);
            assertEquals("nme", text.substring(span.start(), span.end()));
        }
    }

    /** A file of the workspace every package checks alike, at the repository's root. */
    private static String workspace(String path) throws Exception {
        return Files.readString(Path.of("..", "..", "..", "fixtures", "workspace", path));
    }

    @Test
    void theResultTypeAndTheContextAreThoseOfTheSharedWorkspace() throws Exception {
        // As `jlens.json` declares them for `conditions/` and `lines/`.
        try (Schema schema = Schema.fromJson(workspace("order.schema.json"));
                LanguageService conditions =
                        LanguageService.builder().schema(schema).result("boolean").build();
                LanguageService lines = LanguageService.builder()
                        .schema(schema).result("number").context("items[]").build();
                LanguageService strings =
                        LanguageService.builder().schema(schema).result("string").build();
                LanguageService plain = LanguageService.of(schema)) {
            assertEquals(List.of(), conditions.diagnostics(workspace("conditions/large.jsonata")));
            String vip = workspace("conditions/vip.jsonata");
            assertEquals(List.of("W0214"), codes(conditions.diagnostics(vip)));
            assertEquals(List.of(), lines.diagnostics(workspace("lines/subtotal.jsonata")));
            assertEquals(List.of("W0212"), codes(lines.diagnostics(workspace("lines/label.jsonata"))));
            assertEquals(new ResultType(List.of("boolean"), new Span(10, 18), null,
                    ResultType.Check.MAYBE), conditions.resultType(vip));
            assertEquals(ResultType.Check.ALWAYS, lines.resultType("unitPrice * quantity").check());
            // The spans count UTF-16 code units, as Java strings do.
            String text = "/* é😀 */ customer.name";
            Span cause = strings.resultType(text).undefined();
            assertEquals("customer", text.substring(cause.start(), cause.end()));
            assertEquals(new Span(0, 5), strings.resultType("items.code").sequence());
            // With no type expected, nothing is checked; with a syntax error, nothing is known.
            assertEquals(new ResultType(List.of("object"), new Span(0, 8), null, null),
                    plain.resultType("customer"));
            assertEquals(new ResultType(null, null, null, ResultType.Check.UNKNOWN),
                    conditions.resultType("a +"));
            assertEquals(List.of("number", "string"), LanguageService.builder()
                    .result("number", "string").build().resultType("[1, 'a'][0]").types());
        }
        assertThrows(IllegalArgumentException.class, () -> LanguageService.builder().result());
        assertThrows(IllegalArgumentException.class,
                () -> LanguageService.builder().result("boolean", "text"));
        assertThrows(NullPointerException.class,
                () -> LanguageService.builder().result((String) null));
    }

    @Test
    void aServiceOutlivesItsSchemas() {
        LanguageService service;
        try (Schema order = Schema.fromJson(ORDER); Schema customer = Schema.fromJson(CUSTOMER)) {
            service = LanguageService.builder()
                    .schema(order)
                    .variable("customer", customer)
                    .variable("$limit")
                    .build();
        }
        try (service) {
            assertEquals(List.of("W0201"), codes(service.diagnostics("items.nme")));
            assertEquals(List.of("W0201"), codes(service.diagnostics("$customer.nme")));
            assertEquals(List.of(), service.diagnostics("$customer.name & $limit"));
            // With variables declared, another is not known.
            assertEquals(List.of("W0204"), codes(service.diagnostics("$other")));
        }
    }

    @Test
    void declaredFunctionsAreKnownWithTheirSignatures() {
        try (LanguageService service = LanguageService.builder()
                .function("$discount", "<n:n>")
                .function("tax")
                .function("rebate", "<s:s>")
                .function("rebate")
                .build()) {
            assertEquals(List.of(), service.diagnostics("$discount(1) + $tax(2, 3) + $rebate(4)"));
            assertEquals(List.of("W0203"), codes(service.diagnostics("$discount()")));
            assertEquals(List.of("W0205"), codes(service.diagnostics("$discount(\"a\")")));
            assertEquals(List.of("W0202"), codes(service.diagnostics("$refund(1)")));
            // No schema: no field is unknown.
            assertEquals(List.of(), service.diagnostics("anything.at.all"));
        }
    }

    @Test
    void namesAreWrittenAsJson() {
        try (Schema customer = Schema.fromJson(CUSTOMER);
                LanguageService service = LanguageService.builder()
                        .variable("a\"b\\c", customer)
                        .function("f\n")
                        .build()) {
            assertEquals(List.of(), service.diagnostics("1"));
        }
    }

    @Test
    void failuresUseTheCodesOfTheCApi() {
        JlensException notJson = assertThrows(JlensException.class, () -> Schema.fromJson("{\"type\": "));
        assertEquals("U2001", notJson.code());
        assertEquals("U2001", assertThrows(JlensException.class,
                () -> Schema.fromJson(new byte[] {(byte) 0xc3})).code());
        JlensException refused = assertThrows(JlensException.class,
                () -> Schema.fromJson("[1]"));
        assertEquals("U2005", refused.code());
        assertEquals("schema", refused.token().orElseThrow());
        assertThrows(NullPointerException.class,
                () -> LanguageService.builder().function("f", null));
    }

    @Test
    void aLoaderFollowsTheDocumentsItIsGiven() {
        String order = """
                {"type": "object", "properties": {
                    "customer": {"$ref": "common.json#/$defs/person"},
                    "carrier": {"$ref": "carrier.json"},
                    "gone": {"$ref": "#/$defs/gone"}}}""";
        String common = """
                {"$defs": {"person": {"type": "object", "properties": {
                    "name": {"type": "string"}}}}}""";
        Schema.Loader loader = Schema.loader()
                .uri("https://example.com/schemas/order.json")
                .document("https://example.com/schemas/common.json", "{}")
                .document("https://example.com/schemas/common.json",
                        common.getBytes(StandardCharsets.UTF_8));
        Schema schema = loader.load(order);
        try (LanguageService service = LanguageService.of(schema)) {
            assertEquals(List.of("W0201"), codes(service.diagnostics("customer.nme")));
            assertEquals(List.of(), codes(service.diagnostics("carrier.anything & gone.x")));
        }
        schema.close();
        // The report was read with the schema.
        assertEquals(
                List.of(
                        new UnresolvedRef(
                                "https://example.com/schemas/order.json#/properties/carrier",
                                "carrier.json",
                                "https://example.com/schemas/carrier.json",
                                "https://example.com/schemas/carrier.json"),
                        new UnresolvedRef(
                                "https://example.com/schemas/order.json#/properties/gone",
                                "#/$defs/gone",
                                "https://example.com/schemas/order.json#/$defs/gone",
                                null)),
                schema.unresolved());
        assertEquals(List.of("https://example.com/schemas/carrier.json"), schema.missingDocuments());
        assertThrows(UnsupportedOperationException.class, () -> schema.unresolved().clear());

        // The loader serves another load, here with no URI: the references are taken as written.
        try (Schema again = loader.uri(null).document("common.json", common).load(order)) {
            assertEquals(List.of("carrier.json"), again.missingDocuments());
            assertEquals("#/properties/carrier", again.unresolved().getFirst().place());
        }
        try (Schema whole = Schema.fromJson(ORDER)) {
            assertEquals(List.of(), whole.unresolved());
            assertEquals(List.of(), whole.missingDocuments());
            assertEquals(List.of(), whole.unread());
        }

        // A keyword the load does not read leaves its place unknown, and is listed.
        String tree = """
                {"type": "object", "properties": {
                    "children": {"type": "array", "items": {"$dynamicRef": "#node"}},
                    "code": {"type": "int"}}}""";
        try (Schema unread = Schema.fromJson(tree);
             LanguageService service = LanguageService.of(unread)) {
            assertEquals(
                    List.of(new UnreadKeyword("#/properties/children/items", "$dynamicRef"),
                            new UnreadKeyword("#/properties/code", "type")),
                    unread.unread());
            assertEquals(List.of(), service.diagnostics("[children.anything, code + 1]"));
            assertThrows(UnsupportedOperationException.class, () -> unread.unread().clear());
        }

        JlensException notJson = assertThrows(JlensException.class,
                () -> Schema.loader().document("a.json", "{\"type\": ").load("{}"));
        assertEquals("U2001", notJson.code());
        assertEquals("U2005", assertThrows(JlensException.class,
                () -> Schema.loader().document("a.json", "{}").load("[1]")).code());
        assertThrows(NullPointerException.class, () -> Schema.loader().document(null, "{}"));
        assertThrows(NullPointerException.class,
                () -> new UnresolvedRef(null, "a", "b", null));
    }

    @Test
    void closedObjectsThrow() {
        Schema schema = Schema.fromJson(ORDER);
        schema.close();
        schema.close();
        assertThrows(IllegalStateException.class, () -> LanguageService.of(schema));
        assertThrows(IllegalStateException.class,
                () -> LanguageService.builder().variable("x", schema).build());
        LanguageService service = LanguageService.of(null);
        service.close();
        service.close();
        assertThrows(IllegalStateException.class, () -> service.diagnostics("1"));
        assertThrows(IllegalStateException.class, () -> service.resultType("1"));
    }

    @Test
    void virtualThreadsShareAService() throws Exception {
        try (Schema schema = Schema.fromJson(ORDER);
                LanguageService service = LanguageService.of(schema);
                ExecutorService threads = Executors.newVirtualThreadPerTaskExecutor()) {
            List<Future<Boolean>> results = new ArrayList<>();
            for (int t = 0; t < 1000; t++) {
                results.add(threads.submit(() ->
                        codes(service.diagnostics("items.nme")).equals(List.of("W0201"))
                                && service.diagnostics("$sum(items.price) > total").isEmpty()));
            }
            for (Future<Boolean> result : results) assertTrue(result.get());
        }
    }
}
