//! Sets the stack size of the WebAssembly module.

fn main() {
    // rustc's default is 1 MiB. Evaluation is recursive, and jlens stops before the stack runs
    // out (error D1011), so a larger stack allows deeper recursion in expressions.
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size=4194304");
    }
}
