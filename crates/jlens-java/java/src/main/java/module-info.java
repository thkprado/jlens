/**
 * jlens, a JSONata engine in Rust, called through Panama (ADR 0021).
 *
 * <p>Start with {@link io.github.thkprado.jlens.Expression}. The package calls native code:
 * start the JVM with {@code --enable-native-access=io.github.thkprado.jlens} (or {@code
 * ALL-UNNAMED} on the class path), or Java prints a warning at first use.
 */
module io.github.thkprado.jlens {
    exports io.github.thkprado.jlens;
}
