# AGENTS.md — jlens-capi

Guidance for working on the C API under the Java package (ADR 0021; not published). The
repository-wide rules are in the root [AGENTS.md](../../AGENTS.md).

## Map

- Buffers, expressions, options, documents and bindings are known by handle.
- The host's own values are read through a reader it calls back (`src/source.rs`); host
  functions are called back too (`src/host.rs`).
- Audits are replied as values (`src/audit.rs`); replies carry a status byte.
- mimalloc is its allocator.

## Rules

- **`unsafe_code` is `deny` here**, the one crate where it is not `forbid`: it is allowed only on
  `src/exports.rs` for `#[unsafe(no_mangle)]`, which `tests/no_unsafe.rs` checks.
- **Every export catches panics**: a panic in `extern "C"` aborts the JVM.
