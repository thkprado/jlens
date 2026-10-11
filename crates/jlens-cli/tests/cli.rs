//! Snapshot tests of the `jlens` binary: exit code, stdout and stderr of each command.

use std::path::Path;

use assert_cmd::Command;

/// Runs `jlens` from the workspace root and renders the outcome for a snapshot.
fn jlens(args: &[&str], stdin: Option<&str>) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cmd = Command::cargo_bin("jlens").unwrap();
    cmd.current_dir(&root).args(args);
    match stdin {
        Some(input) => cmd.write_stdin(input),
        None => cmd.write_stdin(""),
    };
    let out = cmd.output().unwrap();
    // OS error messages differ between platforms.
    let text = |b: &[u8]| {
        String::from_utf8_lossy(b)
            .replace("\r\n", "\n")
            .replace(
                "The system cannot find the file specified.",
                "<file not found>",
            )
            .replace("No such file or directory", "<file not found>")
            // Usage lines name the binary.
            .replace("jlens.exe", "jlens")
            // Glob matches use the platform's separator (escaped in JSON).
            .replace(r"fixtures\\cli\\check\\", "fixtures/cli/check/")
            .replace(r"fixtures\cli\check\", "fixtures/cli/check/")
    };
    format!(
        "exit: {}\n--- stdout\n{}--- stderr\n{}",
        out.status.code().unwrap_or(-1),
        text(&out.stdout),
        text(&out.stderr)
    )
}

macro_rules! snapshot {
    ($name:literal, $args:expr) => {
        insta::assert_snapshot!($name, jlens($args, None))
    };
    ($name:literal, $args:expr, $stdin:expr) => {
        insta::assert_snapshot!($name, jlens($args, Some($stdin)))
    };
}

const ORDER: &str = "fixtures/online-store-order.json";
const RULE: &str = "fixtures/shipping-rule.jsonata";

#[test]
fn eval() {
    snapshot!("eval_rule", &["eval", "-f", RULE, "-i", ORDER]);
    snapshot!(
        "eval_compact_stdin",
        &["eval", "-c", "items.code"],
        r#"{"items":[{"code":"a"},{"code":"b"}]}"#
    );
    snapshot!("eval_raw", &["eval", "-r", "customer.name", "-i", ORDER]);
    snapshot!("eval_undefined", &["eval", "missing", "-i", ORDER]);
    snapshot!(
        "eval_no_input_numbers",
        &[
            "eval",
            "--no-input",
            "-c",
            "[$power(2, 64), 1e21, 0.1 + 0.2, 1/3]"
        ]
    );
    snapshot!(
        "eval_bindings",
        &[
            "eval",
            "--no-input",
            "-b",
            "fixtures/cli/bindings.json",
            "$limit * 2"
        ]
    );
}

#[test]
fn eval_errors() {
    snapshot!(
        "eval_syntax_error",
        &["eval", "items[type = 'x'", "-i", ORDER]
    );
    snapshot!(
        "eval_type_error",
        &["eval", "customer.name + 1", "-i", ORDER]
    );
    // Where a timeout interrupts the evaluation varies from run to run, so the position does too.
    let timeout = jlens(
        &[
            "eval",
            "--no-input",
            "--timeout",
            "20",
            "($f := function($n){ $f($n + 1) }; $f(0))",
        ],
        None,
    );
    let stable = match timeout.find(" (at position") {
        Some(i) => format!(
            "{}{}",
            &timeout[..i],
            &timeout[timeout[i..].find(')').map_or(timeout.len(), |j| i + j + 1)..]
        ),
        None => timeout,
    };
    insta::assert_snapshot!("eval_timeout", stable);
    snapshot!(
        "eval_missing_file",
        &["eval", "x", "-i", "fixtures/does-not-exist.json"]
    );
    snapshot!("eval_bad_json_stdin", &["eval", "x"], "{not json");
}

#[test]
fn eval_json() {
    snapshot!(
        "eval_json_result",
        &["eval", "--json", "customer.name", "-i", ORDER]
    );
    snapshot!(
        "eval_json_undefined",
        &["eval", "--json", "missing", "-i", ORDER]
    );
    snapshot!(
        "eval_json_syntax_error",
        &["eval", "--json", "--no-input", "(\n  $a := 1;\n  $a +\n)"]
    );
    snapshot!(
        "eval_json_type_error",
        &["eval", "--json", "customer.name + 1", "-i", ORDER]
    );
    snapshot!(
        "eval_json_filter",
        &["eval", "--json", "--filter", "a > 1"],
        r#"{"a":2}"#
    );
    snapshot!(
        "eval_json_filter_no_match",
        &["eval", "--json", "--filter", "a > 1"],
        r#"{"a":1}"#
    );
    snapshot!("eval_json_bad_input", &["eval", "--json", "x"], "{not json");
    snapshot!(
        "eval_json_conflict",
        &["eval", "--json", "-r", "x", "--no-input"]
    );
}

#[test]
fn eval_ndjson() {
    let args = ["eval", "--ndjson", "-i", "fixtures/orders.ndjson"];
    let total = "$sum(items.(unitPrice * quantity))";
    snapshot!("ndjson_results", &[&args[..], &[total]].concat());
    let rule = "customer.login in ['alice.walker99', 'bob.harrison42'] and $sum(items.(unitPrice * quantity)) > 50000";
    snapshot!("ndjson_filter", &[&args[..], &["--filter", rule]].concat());
    snapshot!("ndjson_json", &[&args[..], &["--json", total]].concat());
    snapshot!(
        "ndjson_json_filter",
        &[&args[..], &["--json", "--filter", rule]].concat()
    );
    snapshot!(
        "ndjson_json_error",
        &["eval", "--ndjson", "--json", "a + 1"],
        "{\"a\":1}\n{\"a\":\"x\"}\n"
    );
}

#[test]
fn eval_reproducible() {
    snapshot!(
        "eval_now_seed",
        &[
            "eval",
            "--no-input",
            "-c",
            "--now",
            "2026-01-01T00:00:00Z",
            "--seed",
            "42",
            "{'now': $now(), 'millis': $millis(), 'random': $random(), 'shuffle': $shuffle([1, 2, 3, 4, 5])}"
        ]
    );
    snapshot!(
        "eval_now_millis",
        &["eval", "--no-input", "--now", "1000", "$now()"]
    );
    snapshot!(
        "eval_now_invalid",
        &["eval", "--no-input", "--now", "yesterday", "$now()"]
    );
}

#[test]
fn eval_audit() {
    let mut doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(ORDER),
        )
        .unwrap(),
    )
    .unwrap();
    doc["items"][1]["unitPrice"] = 5000.into();
    let input = doc.to_string();
    snapshot!(
        "eval_explain_rejected",
        &["eval", "-f", RULE, "--explain"],
        &input
    );
    snapshot!(
        "eval_explain_simple",
        &[
            "eval",
            "--no-input",
            "--explain",
            "/** @label \"Positive\" */ 3 > 0 and /** @label \"Small\" */ 3 < 2"
        ]
    );
}

#[test]
fn check() {
    snapshot!("check_ok", &["check", "-f", RULE]);
    snapshot!("check_errors", &["check", "Account.Order[0;].Product"]);
    snapshot!("check_json", &["check", "--json", "a +"]);
    snapshot!(
        "check_schema",
        &[
            "check",
            "--schema",
            "fixtures/cli/order.schema.json",
            "items[type = 'x'].unitprice + customer.nick"
        ]
    );
    snapshot!(
        "check_function_warnings",
        &["check", "$summ(price) + $count() + $length(1234)"]
    );
    snapshot!(
        "check_annotation_warning",
        &["check", "/** @label \"x\" @owner \"me\" */ a > 1"]
    );
}

const CHECK_SCHEMA: &str = "fixtures/cli/order.schema.json";

#[test]
fn check_files() {
    snapshot!(
        "check_files_glob",
        &[
            "check",
            "--schema",
            CHECK_SCHEMA,
            "-f",
            "fixtures/cli/check/*.jsonata"
        ]
    );
    snapshot!(
        "check_files_json",
        &[
            "check",
            "--json",
            "--schema",
            CHECK_SCHEMA,
            "-f",
            "fixtures/cli/check/*.jsonata"
        ]
    );
    let warning_only = [
        "check",
        "--schema",
        CHECK_SCHEMA,
        "-f",
        "fixtures/cli/check/ok.jsonata",
        "fixtures/cli/check/unknown-field.jsonata",
    ];
    snapshot!("check_files_warnings", &warning_only);
    snapshot!(
        "check_files_deny_warnings",
        &[&warning_only[..], &["--deny-warnings"]].concat()
    );
    snapshot!(
        "check_files_no_match",
        &["check", "-f", "fixtures/cli/check/*.nope"]
    );
    snapshot!(
        "check_files_missing",
        &[
            "check",
            "--json",
            "-f",
            "fixtures/cli/check/ok.jsonata",
            "missing.jsonata"
        ]
    );
}

#[test]
fn functions() {
    snapshot!("functions_list", &["functions"]);
    snapshot!("functions_one", &["functions", "formatNumber"]);
    snapshot!("functions_unknown", &["functions", "$nope"]);
}

#[test]
fn errors() {
    snapshot!("errors_list", &["errors"]);
    snapshot!("errors_one", &["errors", "s0202"]);
    snapshot!("errors_one_json", &["errors", "U2001", "--json"]);
    snapshot!("errors_unknown", &["errors", "X9999"]);
}

#[test]
fn errors_json_matches_catalog() {
    let out = Command::cargo_bin("jlens")
        .unwrap()
        .args(["errors", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let catalog = jlens_core::error::catalog();
    assert_eq!(entries.len(), catalog.len());
    for (entry, e) in entries.iter().zip(catalog) {
        assert_eq!(entry["code"], e.code);
        assert_eq!(entry["template"], e.template);
        assert!(entry["category"].is_string());
    }
}

#[test]
fn schema() {
    snapshot!(
        "schema_infer",
        &[
            "schema",
            "infer",
            ORDER,
            "--overrides",
            "fixtures/cli/overrides.json"
        ]
    );
    snapshot!(
        "schema_infer_each",
        &["schema", "infer", "--each", "fixtures/cli/samples.json"]
    );
    snapshot!(
        "schema_complete",
        &[
            "schema",
            "complete",
            "--schema",
            "fixtures/cli/order.schema.json",
            "items[type = 'x'].u"
        ]
    );
    snapshot!(
        "schema_complete_cursor",
        &[
            "schema",
            "complete",
            "--schema",
            "fixtures/cli/order.schema.json",
            "--cursor",
            "9",
            "customer.n + 1"
        ]
    );
}

#[test]
fn ast() {
    snapshot!("ast_tokens", &["ast", "--tokens", "a.b[0]"]);
    snapshot!("ast_tree", &["ast", "a[b > 1]"]);
}

/// `jlens lsp` speaks the protocol on standard input and output, and exits when asked to.
#[test]
fn lsp_runs_a_language_server_session() {
    let frame = |message: &str| format!("Content-Length: {}\r\n\r\n{message}", message.len());
    let session = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:a","languageId":"jsonata","version":1,"text":"$sum(1"}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ]
    .map(frame)
    .concat();
    let out = Command::cargo_bin("jlens")
        .unwrap()
        .arg("lsp")
        .write_stdin(session)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(r#""positionEncoding":"utf-16""#),
        "{stdout}"
    );
    assert!(
        stdout.contains(r#""code":"S0203""#),
        "diagnostics: {stdout}"
    );
}
