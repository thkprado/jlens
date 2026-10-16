//! Lets `cargo build` link the extension module on macOS, as maturin does.

fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
