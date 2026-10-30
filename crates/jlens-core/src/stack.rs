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
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    musl::map_main_stack();
    #[cfg(not(target_arch = "wasm32"))]
    {
        stacker::maybe_grow(128 * 1024, 4 * 1024 * 1024, f)
    }
    #[cfg(target_arch = "wasm32")]
    {
        f()
    }
}

/// The main thread on musl. `stacker` asks `pthread_getattr_np` where a thread's stack ends,
/// once for each thread. For the main thread, musl answers with the part of the stack mapped so
/// far, not the part the kernel would map on demand up to `RLIMIT_STACK`: after a shallow start,
/// often less than the 128 KiB [`grow`] keeps. Then every evaluation ran on a new stack segment,
/// mapped and unmapped each time, about 13 µs. Using 1 MiB of the main thread's stack before
/// `stacker` first asks makes the kernel map it, and musl count it.
///
/// Only the first thread to reach [`grow`] is looked at, so that later calls cost an atomic
/// load rather than a thread-local, which a library loaded at run time reaches through a call
/// (group-by 2% slower from Java). A process whose main thread evaluates only after another
/// thread has keeps the cost on it. (The `java` launcher runs `main` on a thread of its own.)
#[cfg(all(target_os = "linux", target_env = "musl"))]
mod musl {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The stack used, and so mapped, before `stacker` first asks.
    const MAPPED: usize = 1024 * 1024;

    /// The smallest `RLIMIT_STACK` under which the stack is used: room for [`MAPPED`] whatever
    /// depth the host had reached.
    const SMALLEST_LIMIT: u64 = 4 * 1024 * 1024;

    /// Whether no thread has reached [`grow`](super::grow) yet.
    static FIRST: AtomicBool = AtomicBool::new(true);

    /// Maps [`MAPPED`] of the stack, when the first thread to call it is the main thread.
    #[inline]
    pub(super) fn map_main_stack() {
        if FIRST.load(Ordering::Relaxed) && FIRST.swap(false, Ordering::Relaxed) {
            first_call();
        }
    }

    #[cold]
    #[inline(never)]
    fn first_call() {
        // Other threads have the stack they were created with, which musl knows.
        if rustix::thread::gettid() != rustix::process::getpid() {
            return;
        }
        let limit = rustix::process::getrlimit(rustix::process::Resource::Stack).current;
        if limit.is_none_or(|limit| limit >= SMALLEST_LIMIT) {
            use_stack();
        }
    }

    /// Uses [`MAPPED`] bytes of stack: the stack probes touch each page, from the top down.
    #[inline(never)]
    fn use_stack() {
        let pages = [0u8; MAPPED];
        std::hint::black_box(&pages);
    }
}
