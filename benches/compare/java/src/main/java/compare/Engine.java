package compare;

import com.fasterxml.jackson.databind.ObjectMapper;
import io.github.thkprado.jlens.Document;
import io.github.thkprado.jlens.Expression;
import io.github.thkprado.jlens.JlensException;
import io.github.thkprado.jlens.Options;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
import java.time.Duration;
import java.util.List;
import java.util.Properties;

/**
 * One JSONata implementation, as the harness times it: an input converted once (a jlens
 * {@code Document}, a Jackson tree, Java objects), or JSON text parsed at each evaluation.
 */
abstract class Engine {
    /** An expression compiled by one engine. */
    interface Compiled {
        /** Evaluates against an input that {@link Engine#convert} returned. */
        Object evaluate(Object converted) throws Exception;

        /** Evaluates against JSON text, parsing it first. */
        Object evaluateText(String json) throws Exception;
    }

    static final Properties VERSIONS = new Properties();
    static final ObjectMapper JACKSON = new ObjectMapper();

    static {
        try (InputStream in = Engine.class.getResourceAsStream("/versions.properties")) {
            VERSIONS.load(in);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
    }

    /** The engines, by the names the command line uses. */
    static List<Engine> all() {
        return List.of(new Jlens(), new JlensObjects(), new JsonataJava(), new JvmCompiler());
    }

    /** The name the command line uses. */
    abstract String key();

    /** The name and version, for the tables. */
    String title() {
        return key() + " " + VERSIONS.getProperty(key());
    }

    /** The input converted once. */
    abstract Object convert(String json) throws Exception;

    abstract Compiled compile(String expression) throws Exception;

    /** Whether one compiled expression may be evaluated on several threads at once. */
    abstract boolean threadSafe();

    /** The JSON text of a result, {@code null} when it is undefined. */
    abstract String json(Object result) throws Exception;

    /** How {@link Limits} builds this engine's input: maps and boxed values by default. */
    Limits.Builder builder() { return Limits.MAPS; }

    /**
     * Compiles and evaluates once for {@link Limits}, with a timeout and a depth limit when they
     * are not {@code null}; throws {@link Limits.Unsupported} for an option the engine lacks.
     */
    abstract Object limited(String expression, Object input, Integer timeoutMs, Integer maxDepth)
        throws Exception;

    /** The JSONata error code an exception of this engine carries, or {@code null}. */
    String code(Throwable error) { return null; }

    /** jlens over Java objects, as {@link Limits} runs it (cyclic input needs them). */
    static Object jlensLimited(String expression, Object input, Integer timeoutMs, Integer maxDepth) {
        Options.Builder options = Options.builder().maxDepth(maxDepth);
        if (timeoutMs != null) options.timeout(Duration.ofMillis(timeoutMs));
        return Expression.compile(expression).evaluate(input, options.build());
    }

    /** jlens: results come out as Java objects (maps, lists, boxed values), as the others'. */
    static final class Jlens extends Engine {
        String key() { return "jlens"; }

        Object convert(String json) { return Document.fromJson(json); }

        Compiled compile(String source) {
            Expression expression = Expression.compile(source);
            return new Compiled() {
                public Object evaluate(Object converted) { return expression.evaluate((Document) converted); }

                public Object evaluateText(String json) { return expression.evaluateFromJson(json); }
            };
        }

        boolean threadSafe() { return true; }

        String json(Object result) throws Exception {
            return result == null ? null : JACKSON.writeValueAsString(result);
        }

        Object limited(String expression, Object input, Integer timeoutMs, Integer maxDepth) {
            return jlensLimited(expression, input, timeoutMs, maxDepth);
        }

        String code(Throwable error) {
            return error instanceof JlensException j ? j.code() : null;
        }
    }

    /**
     * jlens over Java objects, the maps, lists and boxed values Jackson parses, read one level
     * at a time (0.25 #6); from JSON text, Jackson parses them first.
     */
    static final class JlensObjects extends Engine {
        String key() { return "jlens-objects"; }

        String title() { return "jlens " + VERSIONS.getProperty("jlens") + " (objects)"; }

        Object convert(String json) throws Exception { return JACKSON.readValue(json, Object.class); }

        Compiled compile(String source) {
            Expression expression = Expression.compile(source);
            return new Compiled() {
                public Object evaluate(Object converted) { return expression.evaluate(converted); }

                public Object evaluateText(String json) throws Exception {
                    return expression.evaluate(JACKSON.readValue(json, Object.class));
                }
            };
        }

        boolean threadSafe() { return true; }

        String json(Object result) throws Exception {
            return result == null ? null : JACKSON.writeValueAsString(result);
        }

        Object limited(String expression, Object input, Integer timeoutMs, Integer maxDepth) {
            return jlensLimited(expression, input, timeoutMs, maxDepth);
        }

        String code(Throwable error) {
            return error instanceof JlensException j ? j.code() : null;
        }
    }

    /** jsonata-java (Dashjoin), over maps, lists and boxed values. */
    static final class JsonataJava extends Engine {
        String key() { return "jsonata-java"; }

        Object convert(String json) { return com.dashjoin.jsonata.json.Json.parseJson(json); }

        Compiled compile(String source) {
            var expression = com.dashjoin.jsonata.Jsonata.jsonata(source);
            return new Compiled() {
                public Object evaluate(Object converted) { return expression.evaluate(converted); }

                public Object evaluateText(String json) {
                    return expression.evaluate(com.dashjoin.jsonata.json.Json.parseJson(json));
                }
            };
        }

        boolean threadSafe() { return false; }

        String json(Object result) throws Exception {
            return result == null ? null : JACKSON.writeValueAsString(result);
        }

        Object limited(String expression, Object input, Integer timeoutMs, Integer maxDepth) {
            var compiled = com.dashjoin.jsonata.Jsonata.jsonata(expression);
            var frame = compiled.createFrame();
            if (timeoutMs != null || maxDepth != null)
                frame.setRuntimeBounds(timeoutMs != null ? timeoutMs : 86_400_000L,
                    maxDepth != null ? maxDepth : Integer.MAX_VALUE);
            return compiled.evaluate(input, frame);
        }

        String code(Throwable error) {
            return error instanceof com.dashjoin.jsonata.JException j ? j.getError() : null;
        }
    }

    /** jsonata-jvm-compiler, which compiles an expression to a Java class, over Jackson 2. */
    static final class JvmCompiler extends Engine {
        static final org.json_kula.jsonata_jvm.JsonataExpressionFactory FACTORY =
            new org.json_kula.jsonata_jvm.JsonataExpressionFactory();

        String key() { return "jsonata-jvm-compiler"; }

        Object convert(String json) throws Exception { return JACKSON.readTree(json); }

        Compiled compile(String source) throws Exception {
            var expression = FACTORY.compile(source);
            return new Compiled() {
                public Object evaluate(Object converted) throws Exception {
                    return expression.evaluate((com.fasterxml.jackson.databind.JsonNode) converted);
                }

                public Object evaluateText(String json) throws Exception {
                    return expression.evaluate(JACKSON.readTree(json));
                }
            };
        }

        boolean threadSafe() { return true; }

        String json(Object result) {
            // A missing node is how it says undefined.
            return result == null || ((com.fasterxml.jackson.databind.JsonNode) result).isMissingNode()
                ? null : result.toString();
        }

        Limits.Builder builder() {
            var nodes = com.fasterxml.jackson.databind.node.JsonNodeFactory.instance;
            return new Limits.Builder() {
                public Object object() { return nodes.objectNode(); }

                public void put(Object object, String key, Object value) {
                    ((com.fasterxml.jackson.databind.node.ObjectNode) object)
                        .set(key, (com.fasterxml.jackson.databind.JsonNode) value);
                }

                public Object number(int n) { return nodes.numberNode(n); }
            };
        }

        Object limited(String expression, Object input, Integer timeoutMs, Integer maxDepth)
                throws Exception {
            if (maxDepth != null) throw new Limits.Unsupported();
            var compiled = FACTORY.compile(expression);
            if (timeoutMs != null) compiled.setTimeout(timeoutMs);
            // It takes no null: a missing node is its undefined.
            return compiled.evaluate(input == null
                ? com.fasterxml.jackson.databind.node.MissingNode.getInstance()
                : (com.fasterxml.jackson.databind.JsonNode) input);
        }

        String code(Throwable error) {
            return error instanceof org.json_kula.jsonata_jvm.JsonataException j ? j.getErrorCode() : null;
        }
    }
}
