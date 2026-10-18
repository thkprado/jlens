//! Criterion benchmarks of a document an application rebuilds often from its own Rust state:
//! `cargo bench -p jlens-core --bench host_state`.
//!
//! The state is ten records of twenty fields (strings, numbers, booleans, small lists). Each
//! iteration builds a document from it and evaluates 1, 4 or 10 small expressions (filters,
//! sorts, `[0]`) against it, as an application does before it changes its state again.
//!
//! Groups:
//! - `host build`: building the document alone, through each path;
//! - `host rebuild/<n>`: building it and evaluating the first `n` expressions, through
//!   `serde_json::Value` and `Document::new`, `Document::from_serialize`, lazy `evaluate` over
//!   the `serde_json::Value`, and a [`Source`] written over the state;
//! - `host binding`: an expression that reads one record through a variable of
//!   `evaluate_document`, converted from a `serde_json::Value` or bound to the record's part of
//!   the document (`evaluate_document_with`), against the same expression reading it from the
//!   document.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use jlens_core::input::{Sink, Source};
use jlens_core::{Bindings, Document, Expression, Options};
use serde::Serialize;

#[derive(Serialize)]
struct State {
    turn: u32,
    items: Vec<Record>,
}

#[derive(Serialize)]
struct Record {
    id: u32,
    name: String,
    kind: String,
    status: String,
    owner: String,
    group: String,
    active: bool,
    priority: u8,
    level: u8,
    score: f64,
    ratio: f64,
    weight: f64,
    count: u32,
    limit: u32,
    x: i32,
    y: i32,
    created: u64,
    tags: Vec<String>,
    refs: Vec<u32>,
    note: Option<String>,
}

/// Ten records with varied values, the same on each run.
fn state() -> State {
    let mut seed = 0x2545_f491_u64;
    let mut next = move |n: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) % n
    };
    let words = ["red", "green", "blue", "amber", "violet"];
    let items = (0..10)
        .map(|i| Record {
            id: i,
            name: format!("record-{i}"),
            kind: ["a", "b", "c"][next(3) as usize].to_owned(),
            status: ["open", "done", "held"][next(3) as usize].to_owned(),
            owner: ["x", "y"][next(2) as usize].to_owned(),
            group: format!("group-{}", next(4)),
            active: next(2) == 0,
            priority: next(5) as u8,
            level: next(10) as u8,
            score: next(1000) as f64 / 10.0,
            ratio: next(100) as f64 / 100.0,
            weight: next(500) as f64 / 7.0,
            count: next(20) as u32,
            limit: next(20) as u32,
            x: next(64) as i32 - 32,
            y: next(64) as i32 - 32,
            created: 1_700_000_000_000 + next(1_000_000),
            tags: (0..1 + next(3))
                .map(|_| words[next(5) as usize].to_owned())
                .collect(),
            refs: (0..3).map(|_| next(10) as u32).collect(),
            note: (next(2) == 0).then(|| format!("note {i}")),
        })
        .collect();
    State { turn: 3, items }
}

const EXPRESSIONS: [&str; 10] = [
    r#"items[status = "open"].id"#,
    r#"items[score > 50 and kind = "b"][0].name"#,
    "(items^(>score))[0].id",
    r#"$count(items[tags = "red"])"#,
    "items[priority <= 2].name",
    "(items^(priority, name))[0]",
    "$max(items.score)",
    "items[ratio < 0.5 and active][0].id",
    r#"$sum(items[owner = "x"].count)"#,
    "items[limit > count].id",
];

/// The state read as a [`Source`]: its records one level at a time, and a record's field alone
/// when a path step asks for it.
struct Input<'s>(&'s State);
struct Root<'s>(&'s State);
struct Items<'s>(&'s [Record]);
struct Rec<'s>(&'s Record);
struct Strings<'s>(&'s [String]);
struct Numbers<'s>(&'s [u32]);

impl<'s> Source<'s> for Input<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        sink.object(Root(self.0));
        Ok(())
    }
}

impl<'s> Source<'s> for Root<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        sink.key("turn");
        sink.number(self.0.turn.into());
        sink.key("items");
        sink.array(Items(&self.0.items));
        Ok(())
    }
}

impl<'s> Source<'s> for Items<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        sink.reserve(self.0.len());
        for record in self.0 {
            sink.object(Rec(record));
        }
        Ok(())
    }
}

const FIELDS: [&str; 20] = [
    "id", "name", "kind", "status", "owner", "group", "active", "priority", "level", "score",
    "ratio", "weight", "count", "limit", "x", "y", "created", "tags", "refs", "note",
];

impl<'s> Rec<'s> {
    /// Writes the field `key`, or nothing when the record has no such field.
    fn write(&self, key: &str, sink: &mut Sink<'_, 's>) -> bool {
        let r = self.0;
        match key {
            "id" => sink.number(r.id.into()),
            "name" => sink.string(&r.name),
            "kind" => sink.string(&r.kind),
            "status" => sink.string(&r.status),
            "owner" => sink.string(&r.owner),
            "group" => sink.string(&r.group),
            "active" => sink.bool(r.active),
            "priority" => sink.number(r.priority.into()),
            "level" => sink.number(r.level.into()),
            "score" => sink.number(r.score),
            "ratio" => sink.number(r.ratio),
            "weight" => sink.number(r.weight),
            "count" => sink.number(r.count.into()),
            "limit" => sink.number(r.limit.into()),
            "x" => sink.number(r.x.into()),
            "y" => sink.number(r.y.into()),
            "created" => sink.number(r.created as f64),
            "tags" => sink.array(Strings(&r.tags)),
            "refs" => sink.array(Numbers(&r.refs)),
            "note" => match &r.note {
                Some(note) => sink.string(note),
                None => sink.null(),
            },
            _ => return false,
        }
        true
    }
}

impl<'s> Source<'s> for Rec<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        sink.reserve(FIELDS.len());
        for key in FIELDS {
            sink.key(key);
            self.write(key, sink);
        }
        Ok(())
    }

    fn read_field(&self, key: &str, sink: &mut Sink<'_, 's>) -> Result<bool, String> {
        self.write(key, sink);
        Ok(true)
    }
}

impl<'s> Source<'s> for Strings<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        for s in self.0 {
            sink.string(s);
        }
        Ok(())
    }
}

impl<'s> Source<'s> for Numbers<'s> {
    fn read(&self, sink: &mut Sink<'_, 's>) -> Result<(), String> {
        for n in self.0 {
            sink.number((*n).into());
        }
        Ok(())
    }
}

fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
}

fn build(c: &mut Criterion) {
    let state = state();
    // The paths must build the same document, or the benchmark compares different work.
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(
        Document::from_serialize(&state).unwrap().to_json(),
        Document::new(&json).to_json()
    );

    let mut group = c.benchmark_group("host build");
    group.bench_function("serde_json::to_value", |b| {
        b.iter(|| serde_json::to_value(black_box(&state)).unwrap())
    });
    group.bench_function("to_value + Document::new", |b| {
        b.iter(|| Document::new(&serde_json::to_value(black_box(&state)).unwrap()))
    });
    group.bench_function("to_string + Document::parse", |b| {
        b.iter(|| Document::parse(&serde_json::to_string(black_box(&state)).unwrap()).unwrap())
    });
    group.bench_function("Document::from_serialize", |b| {
        b.iter(|| Document::from_serialize(black_box(&state)).unwrap())
    });
    group.finish();
}

fn rebuild(c: &mut Criterion) {
    let state = state();
    let exprs: Vec<_> = EXPRESSIONS
        .iter()
        .map(|e| Expression::compile(e).unwrap())
        .collect();
    let bindings = Bindings::new();
    let options = Options::default();

    // Every path gives the same results.
    let json = serde_json::to_value(&state).unwrap();
    let doc = Document::from_serialize(&state).unwrap();
    for expr in &exprs {
        let expected = expr.evaluate(Some(&json), &bindings).unwrap();
        assert!(expected.is_some());
        let source = expr.evaluate_source(&Input(&state), &bindings, &options);
        assert_eq!(source.unwrap(), expected);
        let document = expr.evaluate_document(&doc, &bindings, &options);
        assert_eq!(document.unwrap(), expected);
    }

    for n in [1, 4, 10] {
        let exprs = &exprs[..n];
        let mut group = c.benchmark_group(format!("host rebuild/{n}"));
        group.bench_function("to_value + Document::new", |b| {
            b.iter(|| {
                let doc = Document::new(&serde_json::to_value(black_box(&state)).unwrap());
                for expr in exprs {
                    black_box(expr.evaluate_document(&doc, &bindings, &options).unwrap());
                }
            })
        });
        group.bench_function("Document::from_serialize", |b| {
            b.iter(|| {
                let doc = Document::from_serialize(black_box(&state)).unwrap();
                for expr in exprs {
                    black_box(expr.evaluate_document(&doc, &bindings, &options).unwrap());
                }
            })
        });
        group.bench_function("to_value + evaluate", |b| {
            b.iter(|| {
                let json = serde_json::to_value(black_box(&state)).unwrap();
                for expr in exprs {
                    black_box(
                        expr.evaluate_with(Some(&json), &bindings, &options)
                            .unwrap(),
                    );
                }
            })
        });
        group.bench_function("evaluate_source", |b| {
            b.iter(|| {
                let input = Input(black_box(&state));
                for expr in exprs {
                    black_box(expr.evaluate_source(&input, &bindings, &options).unwrap());
                }
            })
        });
        group.finish();
    }
}

fn binding(c: &mut Criterion) {
    let state = state();
    let doc = Document::from_serialize(&state).unwrap();
    let options = Options::default();
    let record = serde_json::to_value(&state.items[3]).unwrap();
    let bound = Bindings::new().var("self", record);
    let by_variable = Expression::compile("$self.score > 50 and $self.active").unwrap();
    let by_path = Expression::compile("items[3].score > 50 and items[3].active").unwrap();
    let part = doc.pointer("/items/3").unwrap();
    let expected = by_path
        .evaluate_document(&doc, &Bindings::new(), &options)
        .unwrap();
    assert_eq!(
        by_variable
            .evaluate_document(&doc, &bound, &options)
            .unwrap(),
        expected
    );
    assert_eq!(
        by_variable
            .evaluate_document_with(&doc, &[("self", &part)], &Bindings::new(), &options)
            .unwrap(),
        expected
    );

    let mut group = c.benchmark_group("host binding");
    group.bench_function("record through a variable", |b| {
        b.iter(|| {
            by_variable
                .evaluate_document(black_box(&doc), &bound, &options)
                .unwrap()
        })
    });
    let none = Bindings::new();
    group.bench_function("record through a document variable", |b| {
        b.iter(|| {
            by_variable
                .evaluate_document_with(black_box(&doc), &[("self", &part)], &none, &options)
                .unwrap()
        })
    });
    group.bench_function("record through the document", |b| {
        b.iter(|| {
            by_path
                .evaluate_document(black_box(&doc), &none, &options)
                .unwrap()
        })
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = config();
    targets = build, rebuild, binding
}
criterion_main!(benches);
