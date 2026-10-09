//! Rough evaluation throughput measurement: `cargo run --release --example bench_eval`.
//!
//! Used to compare evaluator changes (for example the cost of tracing hooks when no tracer is
//! attached). Prints the fastest time per evaluation over several rounds (the least noisy statistic).

use std::time::Instant;

use jlens_core::{Bindings, Expression};
use serde_json::json;

const COLORS: [&str; 3] = ["blue", "black", "white"];

fn main() {
    let items: Vec<_> = (0..200)
        .map(|i| {
            json!({
                "code": i.to_string(),
                "type": if i % 3 == 0 { "electronic" } else { "apparel" },
                "color": COLORS[i % 3],
                "unitPrice": 100.0 + i as f64,
                "quantity": 1 + i % 4,
                "tax": 10.0 + (i % 7) as f64,
            })
        })
        .collect();
    let doc = json!({"customer": {"login": "alice.walker99"}, "items": items});
    let rule = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/shipping-rule.jsonata"),
    )
    .unwrap();
    let cases = [
        ("shipping rule", rule.as_str()),
        (
            "filter + aggregate",
            "$sum(items[type = 'electronic' and quantity > 1].(unitPrice * quantity))",
        ),
        (
            "group-by",
            "items{type: {'total': $sum($.(unitPrice * quantity)), 'n': $count($)}}",
        ),
        (
            "sort + map",
            "$map(items^(>tax, code), function($i){ $i.code & ':' & $string($i.tax) })",
        ),
        (
            "recursion",
            "($f := function($n){ $n <= 1 ? $n : $f($n - 1) + $f($n - 2) }; $f(15))",
        ),
    ];
    for (name, src) in cases {
        let expr = Expression::compile(src).unwrap();
        let mut rounds = Vec::new();
        for _ in 0..15 {
            let start = Instant::now();
            let n = 200;
            for _ in 0..n {
                std::hint::black_box(expr.evaluate(Some(&doc), &Bindings::new()).unwrap());
            }
            rounds.push(start.elapsed().as_secs_f64() / f64::from(n) * 1e6);
        }
        rounds.sort_by(f64::total_cmp);
        println!("{name:20} {:9.1} µs/eval", rounds[0]);
    }
}
