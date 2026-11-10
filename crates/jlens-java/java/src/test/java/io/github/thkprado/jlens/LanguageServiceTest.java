package io.github.thkprado.jlens;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.nio.charset.StandardCharsets;
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
