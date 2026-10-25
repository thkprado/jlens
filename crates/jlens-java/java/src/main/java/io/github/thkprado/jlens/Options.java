package io.github.thkprado.jlens;

import java.time.Duration;
import java.time.Instant;
import java.util.StringJoiner;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The options of an evaluation: its limits, the clock and the seed. A component left
 * {@code null} keeps its default. Build them with {@link #builder()}:
 *
 * {@snippet :
 * Options options = Options.builder().timeout(Duration.ofSeconds(1)).maxSteps(1_000_000).build();
 * }
 *
 * @param timeout how long the evaluation may run before it fails with {@code D1012}
 * @param maxDepth how deep the evaluation may nest before it fails with {@code D1011}
 *     (50,000 by default)
 * @param maxSteps how many steps the evaluation may take before it fails with {@code U1006}
 * @param maxStringLength the longest string the evaluation may build ({@code U1001})
 * @param maxArrayLength the longest array the evaluation may build ({@code U1001})
 * @param maxResultSize the largest result, in bytes of JSON text ({@code U1001})
 * @param now the time {@code $now()} and {@code $millis()} return, the clock's by default
 * @param seed the seed of {@code $random()} and {@code $shuffle()}, random by default
 */
public record Options(
        Duration timeout,
        Integer maxDepth,
        Long maxSteps,
        Integer maxStringLength,
        Integer maxArrayLength,
        Integer maxResultSize,
        Instant now,
        Long seed) {

    /** The defaults: every component {@code null}. */
    public static final Options DEFAULTS = new Options(null, null, null, null, null, null, null, null);

    /** Checks that no limit is negative. */
    public Options {
        if (timeout != null && timeout.isNegative()) throw negative("timeout");
        if (maxDepth != null && maxDepth < 0) throw negative("maxDepth");
        if (maxSteps != null && maxSteps < 0) throw negative("maxSteps");
        if (maxStringLength != null && maxStringLength < 0) throw negative("maxStringLength");
        if (maxArrayLength != null && maxArrayLength < 0) throw negative("maxArrayLength");
        if (maxResultSize != null && maxResultSize < 0) throw negative("maxResultSize");
    }

    private static IllegalArgumentException negative(String name) {
        return new IllegalArgumentException(name + " must not be negative");
    }

    /** {@return a builder starting from the defaults} */
    public static Builder builder() {
        return new Builder(DEFAULTS);
    }

    /** {@return a builder starting from these options} */
    public Builder toBuilder() {
        return new Builder(this);
    }

    /** The options as the C API reads them: JSON text with the WebAssembly package's names. */
    String toJson() {
        StringJoiner json = new StringJoiner(",", "{", "}");
        if (timeout != null) {
            json.add("\"timeout\":" + (timeout.getSeconds() * 1000.0 + timeout.getNano() / 1e6));
        }
        if (maxDepth != null) json.add("\"maxDepth\":" + maxDepth);
        if (maxSteps != null) json.add("\"maxSteps\":" + maxSteps);
        if (maxStringLength != null) json.add("\"maxStringLength\":" + maxStringLength);
        if (maxArrayLength != null) json.add("\"maxArrayLength\":" + maxArrayLength);
        if (maxResultSize != null) json.add("\"maxResultSize\":" + maxResultSize);
        if (now != null) json.add("\"now\":" + now.toEpochMilli());
        if (seed != null) json.add("\"seed\":" + Long.toUnsignedString(seed));
        return json.toString();
    }

    /**
     * The most options kept as native objects. Options are usually a handful of constants, so
     * each is made once; past this count, as when {@code now} changes with each evaluation, the
     * native options live for one evaluation only.
     */
    private static final int KEPT = 64;

    private static final ConcurrentHashMap<Options, Long> NATIVE = new ConcurrentHashMap<>();

    /**
     * Runs {@code evaluation} with the handle of these options' native object, 0 for the
     * defaults.
     */
    long evaluate(java.util.function.LongUnaryOperator evaluation) {
        if (equals(DEFAULTS)) return evaluation.applyAsLong(0);
        Long kept = NATIVE.get(this);
        if (kept != null) return evaluation.applyAsLong(kept);
        long handle = make();
        if (NATIVE.size() < KEPT) {
            Long raced = NATIVE.putIfAbsent(this, handle);
            if (raced == null) return evaluation.applyAsLong(handle);
            Native.optionsFree(handle);
            return evaluation.applyAsLong(raced);
        }
        try {
            return evaluation.applyAsLong(handle);
        } finally {
            Native.optionsFree(handle);
        }
    }

    private long make() {
        long text = Native.buffer(toJson());
        try {
            return Native.handle(Native.optionsNew(text));
        } finally {
            Native.free(text);
        }
    }

    /** Builds {@link Options}, one component at a time. */
    public static final class Builder {
        private Duration timeout;
        private Integer maxDepth;
        private Long maxSteps;
        private Integer maxStringLength;
        private Integer maxArrayLength;
        private Integer maxResultSize;
        private Instant now;
        private Long seed;

        private Builder(Options from) {
            timeout = from.timeout;
            maxDepth = from.maxDepth;
            maxSteps = from.maxSteps;
            maxStringLength = from.maxStringLength;
            maxArrayLength = from.maxArrayLength;
            maxResultSize = from.maxResultSize;
            now = from.now;
            seed = from.seed;
        }

        /**
         * Sets {@link Options#timeout()}.
         *
         * @param timeout the timeout, {@code null} for none
         * @return this builder
         */
        public Builder timeout(Duration timeout) {
            this.timeout = timeout;
            return this;
        }

        /**
         * Sets {@link Options#maxDepth()}.
         *
         * @param maxDepth the limit, {@code null} for the default
         * @return this builder
         */
        public Builder maxDepth(Integer maxDepth) {
            this.maxDepth = maxDepth;
            return this;
        }

        /**
         * Sets {@link Options#maxSteps()}.
         *
         * @param maxSteps the limit, {@code null} for none
         * @return this builder
         */
        public Builder maxSteps(Long maxSteps) {
            this.maxSteps = maxSteps;
            return this;
        }

        /**
         * Sets {@link Options#maxSteps()}.
         *
         * @param maxSteps the limit
         * @return this builder
         */
        public Builder maxSteps(long maxSteps) {
            this.maxSteps = maxSteps;
            return this;
        }

        /**
         * Sets {@link Options#maxStringLength()}.
         *
         * @param maxStringLength the limit, {@code null} for none
         * @return this builder
         */
        public Builder maxStringLength(Integer maxStringLength) {
            this.maxStringLength = maxStringLength;
            return this;
        }

        /**
         * Sets {@link Options#maxArrayLength()}.
         *
         * @param maxArrayLength the limit, {@code null} for none
         * @return this builder
         */
        public Builder maxArrayLength(Integer maxArrayLength) {
            this.maxArrayLength = maxArrayLength;
            return this;
        }

        /**
         * Sets {@link Options#maxResultSize()}.
         *
         * @param maxResultSize the limit, {@code null} for none
         * @return this builder
         */
        public Builder maxResultSize(Integer maxResultSize) {
            this.maxResultSize = maxResultSize;
            return this;
        }

        /**
         * Sets {@link Options#now()}.
         *
         * @param now the time, {@code null} for the clock's
         * @return this builder
         */
        public Builder now(Instant now) {
            this.now = now;
            return this;
        }

        /**
         * Sets {@link Options#seed()}.
         *
         * @param seed the seed, {@code null} for a random one
         * @return this builder
         */
        public Builder seed(Long seed) {
            this.seed = seed;
            return this;
        }

        /**
         * Sets {@link Options#seed()}.
         *
         * @param seed the seed
         * @return this builder
         */
        public Builder seed(long seed) {
            this.seed = seed;
            return this;
        }

        /**
         * The options.
         *
         * @return the options built
         */
        public Options build() {
            return new Options(timeout, maxDepth, maxSteps, maxStringLength, maxArrayLength,
                    maxResultSize, now, seed);
        }
    }
}
