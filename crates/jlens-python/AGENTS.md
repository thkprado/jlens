# AGENTS.md — jlens-python

Guidance for working in the Python package. The repository-wide rules are in the root
[AGENTS.md](../../AGENTS.md).

## Map

- PyO3 bindings built by maturin (`pyproject.toml`).
- `src/source.rs` reads `evaluate`'s input one level at a time.
- `src/host.rs` calls Python functions as host functions.
- `python/jlens/` holds the package, the stubs (`_jlens.pyi`), the audit types (`_types.py`)
  and `py.typed`.
- `tests/` holds the pytest suite.

## Testing

Its Rust code is not unit-tested by `cargo test`: `maturin develop` in a virtual environment,
then `python -m pytest -p no:cacheprovider tests`.

`.github/workflows/wheels.yml` builds the abi3 and abi3t wheels of every release target and the
source distribution, and tests each (CI, on pushes to `main`).
