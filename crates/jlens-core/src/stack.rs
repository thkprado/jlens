//! Stack headroom checks, so that deep recursion fails with `D1011` instead of overflowing the
//! stack.
//!
//! On native targets the evaluator grows the stack on demand ([`stacker`]). WebAssembly has a
//! fixed stack, and an overflow there traps and leaves the module instance unusable, so the
//! recursive parts of jlens stop while some headroom is left.
//!
//! Crates that walk syntax trees recursively (such as `jlens-schema`) use the same helpers.

/// Stack bytes that must remain free when entering a recursive step.
#[cfg(target_arch = "wasm32")]
const RED_ZONE: usize = 128 * 1024;

/// Whether the stack is close to exhaustion. Only meaningful on WebAssembly; native targets
/// grow the stack instead and always return `false`.
#[inline]
pub fn exhausted() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        // rustc links wasm32 modules with `--stack-first`: the stack occupies the start of
        // linear memory and grows down towards address 0, so the address of a local is the
        // space left.
        let marker = 0u8;
        (std::hint::black_box(&marker) as *const u8 as usize) < RED_ZONE
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Runs `f`, first growing the stack if little of it is left (native targets only).
#[inline]
pub fn grow<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    {
        stacker::maybe_grow(128 * 1024, 4 * 1024 * 1024, f)
    }
    #[cfg(target_arch = "wasm32")]
    {
        f()
    }
}
