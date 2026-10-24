//! jlens-regress: measures how often the interrupt is checked (ADR 0009). Run it in release:
//! `cargo run --release -p jlens-regress --example interrupt_cost`.
//!
//! On patterns that backtrack catastrophically, it gives the time between two calls to the
//! interrupt (target: under 1 ms). What the check costs on ordinary matches is measured by the
//! engine's benchmark, `benches/engine.rs`.

use regress::{INTERRUPT_INTERVAL, Regex};
use std::time::{Duration, Instant};

fn main() {
    println!("INTERRUPT_INTERVAL = {INTERRUPT_INTERVAL}");
    println!("\ntime between two checks, stopping after 500 ms");
    let long_a = format!("{}!", "a".repeat(64));
    let words = format!("{}!", "word ".repeat(40));
    for (pattern, text) in [
        (r"^(a+)+$", long_a.as_str()),
        (r"(a|aa)+$", long_a.as_str()),
        (r"^(a+)+\1$", long_a.as_str()),
        (r"^(?!(a+)+$)", long_a.as_str()),
        (r"^(\w+\s?)*$", words.as_str()),
    ] {
        let re = Regex::new(pattern).unwrap();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(500);
        let mut last = start;
        let mut gaps = Vec::new();
        let result = re.find_from_interruptible(text, 0, || {
            let now = Instant::now();
            gaps.push(now - last);
            last = now;
            now >= deadline
        });
        let overshoot = start.elapsed().saturating_sub(Duration::from_millis(500));
        assert!(result.is_err());
        let checks = gaps.len();
        gaps.sort();
        println!(
            "{pattern:>14}: {checks:>6} checks, median {:>8.1?}, max {:>8.1?}, overshoot {:>8.1?}",
            gaps[checks / 2],
            gaps[checks - 1],
            overshoot,
        );
    }
}
