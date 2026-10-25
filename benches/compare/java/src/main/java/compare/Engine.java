package compare;

import com.fasterxml.jackson.databind.ObjectMapper;
import io.github.thkprado.jlens.Document;
import io.github.thkprado.jlens.Expression;
import java.io.IOException;
import java.io.InputStream;
import java.io.UncheckedIOException;
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
        return List.of(new Jlens(), new JsonataJava(), new Jsonata4Java(), new JvmCompiler());
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
    }

    /** JSONata4Java (IBM), over Jackson 3's trees. */
    static final class Jsonata4Java extends Engine {
        static final tools.jackson.databind.ObjectMapper MAPPER =
            new tools.jackson.databind.ObjectMapper();

        String key() { return "jsonata4java"; }

        Object convert(String json) { return MAPPER.readTree(json); }

        Compiled compile(String source) throws Exception {
            var expression = com.api.jsonata4java.expressions.Expressions.parse(source);
            return new Compiled() {
                public Object evaluate(Object converted) throws Exception {
                    return expression.evaluate((tools.jackson.databind.JsonNode) converted);
                }

                public Object evaluateText(String json) throws Exception {
                    return expression.evaluate(MAPPER.readTree(json));
                }
            };
        }

        boolean threadSafe() { return false; }

        String json(Object result) { return result == null ? null : result.toString(); }
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
    }
}
