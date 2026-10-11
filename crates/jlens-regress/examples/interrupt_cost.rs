//! jlens-regress: measures what the interrupt check costs (ADR 0009). Run it in release:
//! `cargo run --release -p jlens-regress --example interrupt_cost`.
//!
//! 1. On patterns that backtrack catastrophically, the time between two calls to the interrupt
//!    (target: under 1 ms).
//! 2. On ordinary matches, the time of `find_from_interruptible`, with an interrupt like jlens's
//!    (a deadline and a cancellation flag), against `find_from` (target: under 2 % slower).

use regress::{INTERRUPT_INTERVAL, Regex};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn main() {
    println!("INTERRUPT_INTERVAL = {INTERRUPT_INTERVAL}");
    println!("\n1. time between two checks, stopping after 500 ms");
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

    println!("\n2. ordinary matches: find_from vs find_from_interruptible");
    let log =
        "2026-10-11T04:03:31Z INFO user=alice id=1234 path=/api/v1/orders status=200\n".repeat(200);
    let prose = "The quick brown fox jumps over the lazy dog, again and again. ".repeat(200);
    let cases: &[(&str, &str, &str)] = &[
        ("anchored", r"^1[0-9]$", "17"),
        ("literal", r"status=500", &log),
        ("captures", r"user=(\w+) id=(\d+)", &log),
        ("alternation", r"(?:fox|dog|cat)s?\b", &prose),
        ("lazy", r"path=(.*?) ", &log),
        ("backtracking", r"(\w+)\s+\1", &prose),
        ("icase", r"LAZY\s+DOG", &prose),
    ];
    let cancel = AtomicBool::new(false);
    for &(name, pattern, text) in cases {
        let re = Regex::new(pattern).unwrap();
        let all = |interruptible: bool| {
            let deadline = Instant::now() + Duration::from_secs(3600);
            let mut start = 0;
            let mut count = 0;
            loop {
                let m = if interruptible {
                    re.find_from_interruptible(text, start, || {
                        cancel.load(Ordering::Relaxed) || Instant::now() >= deadline
                    })
                    .unwrap()
                } else {
                    re.find_from(text, start).next()
                };
                let Some(m) = m else { break };
                count += 1;
                start = if m.end() > m.start() {
                    m.end()
                } else {
                    m.end() + 1
                };
                if start > text.len() {
                    break;
                }
            }
            count
        };
        let base = time(|| all(false));
        let checked = time(|| all(true));
        println!(
            "{name:>13}: {base:>9.1?} vs {checked:>9.1?} ({:+.1} %)",
            (checked.as_secs_f64() / base.as_secs_f64() - 1.0) * 100.0
        );
    }
}

/// The best of 15 runs of 200 calls.
fn time(f: impl Fn() -> usize) -> Duration {
    (0..15)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..200 {
                black_box(f());
            }
            start.elapsed() / 200
        })
        .min()
        .unwrap()
}
