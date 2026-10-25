package io.github.thkprado.jlens;

import java.io.Serial;
import java.math.BigDecimal;
import java.util.Map;
import java.util.Optional;
import java.util.OptionalInt;

/**
 * An error of jlens, with its JSONata code ({@code S0201}, {@code T2001}, {@code D1011}...) and,
 * when the error has them, the position and the range of the expression's source it concerns,
 * the token and the value. Offsets count UTF-16 code units, as Java strings and jsonata-js do.
 *
 * <p>Codes are the stable part: match on {@link #code()}, never on the message. The package's
 * own failures use {@code U} codes: {@code U1001} for a size limit, {@code U1006} for the step
 * limit, {@code U2001} for input that is not JSON, {@code U2005} for a read that is not in its
 * form.
 */
public final class JlensException extends RuntimeException {
    @Serial
    private static final long serialVersionUID = 1L;

    private final String code;
    private final Integer position;
    private final Integer start;
    private final Integer end;
    private final String token;
    private final String value;

    private JlensException(String code, String message, Integer position, Integer start,
            Integer end, String token, String value) {
        super(message);
        this.code = code;
        this.position = position;
        this.start = start;
        this.end = end;
        this.token = token;
        this.value = value;
    }

    /** The exception of the C API's error envelope {@code envelope}. */
    static JlensException fromEnvelope(String envelope) {
        Map<String, String> members = Json.members(envelope);
        return new JlensException(
                (String) Json.parse(members.get("code")),
                (String) Json.parse(members.get("message")),
                integer(members.get("position")),
                integer(members.get("start")),
                integer(members.get("end")),
                members.containsKey("token") ? (String) Json.parse(members.get("token")) : null,
                members.get("value"));
    }

    private static Integer integer(String json) {
        return json == null ? null : ((BigDecimal) Json.parse(json)).intValueExact();
    }

    /** {@return the JSONata error code, such as {@code S0201}} */
    public String code() {
        return code;
    }

    /** {@return the class, the code and the message} */
    @Override
    public String toString() {
        return getClass().getName() + ": " + code + ": " + getMessage();
    }

    /** {@return where in the expression's source the error was found, in UTF-16 code units} */
    public OptionalInt position() {
        return position == null ? OptionalInt.empty() : OptionalInt.of(position);
    }

    /** {@return the start of the range of the source the error concerns, in UTF-16 code units} */
    public OptionalInt start() {
        return start == null ? OptionalInt.empty() : OptionalInt.of(start);
    }

    /** {@return the end of the range of the source the error concerns, in UTF-16 code units} */
    public OptionalInt end() {
        return end == null ? OptionalInt.empty() : OptionalInt.of(end);
    }

    /** {@return the token the error concerns, such as a function's name or an unexpected symbol} */
    public Optional<String> token() {
        return Optional.ofNullable(token);
    }

    /** {@return the value the error concerns, as JSON text} */
    public Optional<String> valueJson() {
        return Optional.ofNullable(value);
    }
}
