//! Snapshot tests of the `jlens` binary: exit code, stdout and stderr of each command.

use std::path::Path;

use assert_cmd::Command;

/// Runs `jlens` from the workspace root and renders the outcome for a snapshot.
fn jlens(args: &[&str], stdin: Option<&str>) -> String {
    jlens_in(".", args, stdin)
}

/// Runs `jlens` from `dir`, relative to the workspace root.
fn jlens_in(dir: &str, args: &[&str], stdin: Option<&str>) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
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
            .replace(r"fixtures\workspace\\", "fixtures/workspace/")
            .replace(r"fixtures\workspace\", "fixtures/workspace/")
            .replace(r"rules\\", "rules/")
            .replace(r"rules\", "rules/")
            .replace(r"conditions\", "conditions/")
            .replace(r"lines\", "lines/")
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
    // The limit is the error's value.
    snapshot!(
        "eval_json_max_steps",
        &[
            "eval",
            "--json",
            "--no-input",
            "--max-steps",
            "100",
            "$sum($map([1..1000], function($i) { $i }))"
        ]
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

/// The input is parsed without the fields the expression does not read (ADR 0016).
#[test]
fn eval_cut_input() {
    snapshot!(
        "eval_cut_filter_prints_whole",
        &["eval", "-c", "--filter", "a > 1"],
        r#"{"a":2,"b":[1,{"c":"x"}]}"#
    );
    snapshot!(
        "eval_cut_bad_json_unread",
        &["eval", "a"],
        r#"{"a":2,"b":[1,}"#
    );
    snapshot!(
        "eval_cut_ndjson_filter_prints_whole",
        &["eval", "--ndjson", "--filter", "a > 1"],
        "{\"a\":2,\"b\":{\"c\":[true]}}\n{\"a\":1,\"b\":null}\n"
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
fn eval_size_caps() {
    snapshot!(
        "eval_max_string_length",
        &[
            "eval",
            "--no-input",
            "--max-string-length",
            "5",
            "'abc' & 'def'"
        ]
    );
    snapshot!(
        "eval_max_array_length",
        &["eval", "--no-input", "--max-array-length", "5", "[1..10]"]
    );
    snapshot!(
        "eval_max_steps",
        &[
            "eval",
            "--no-input",
            "--max-steps",
            "100",
            "$sum($map([1..1000], function($i) { $i }))"
        ]
    );
    snapshot!(
        "eval_max_result_size",
        &[
            "eval",
            "--no-input",
            "--max-result-size",
            "40",
            "($o := {'name': 'abcdef'}; {'l': $o, 'r': $o})"
        ]
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

/// `fixtures/workspace` is checked by the language server tests too.
const WORKSPACE_CONFIG: &str = "fixtures/workspace/jlens.json";

#[test]
fn check_config() {
    snapshot!(
        "check_config",
        &[
            "check",
            "--config",
            WORKSPACE_CONFIG,
            "-f",
            "fixtures/workspace/rules/*.jsonata"
        ]
    );
    // The `jlens.json` of the current directory applies by default.
    insta::assert_snapshot!(
        "check_config_default",
        jlens_in(
            "fixtures/workspace",
            &["check", "--json", "-f", "rules/*.jsonata"],
            None
        )
    );
    // `--schema` replaces the schema; the variables and functions stay declared.
    snapshot!(
        "check_config_schema",
        &[
            "check",
            "--config",
            WORKSPACE_CONFIG,
            "--schema",
            CHECK_SCHEMA,
            "-f",
            "fixtures/workspace/rules/discount.jsonata"
        ]
    );
    // An expression of the argument gets the configuration of the path it is given.
    let inline = [
        "check",
        "--config",
        WORKSPACE_CONFIG,
        "$rate('toy') > $minimun",
    ];
    snapshot!("check_config_inline", &inline);
    snapshot!(
        "check_config_path",
        &[
            &inline[..],
            &["--path", "fixtures/workspace/rules/new.jsonata"]
        ]
        .concat()
    );
    snapshot!(
        "check_config_missing",
        &["check", "--config", "missing.json", "a"]
    );
}

#[test]
fn test() {
    snapshot!(
        "test_pass",
        &[
            "test",
            "fixtures/cli/test/shipping.json",
            "fixtures/cli/test/clock.json"
        ]
    );
    snapshot!(
        "test_failures",
        &["test", "fixtures/cli/test/discount.json"]
    );
    snapshot!(
        "test_json",
        &["test", "--json", "fixtures/cli/test/discount.json"]
    );
    snapshot!(
        "test_size_caps",
        &[
            "test",
            "--max-array-length",
            "5",
            "fixtures/cli/test/caps.json"
        ]
    );
    // Each item takes 4 steps, and the rest 4: 12 for two items, 16 for three.
    snapshot!(
        "test_max_steps",
        &["test", "--max-steps", "12", "fixtures/cli/test/steps.json"]
    );
    snapshot!(
        "test_invalid",
        &[
            "test",
            "--json",
            "fixtures/cli/test/shipping.json",
            "fixtures/cli/test/invalid/two-expectations.json"
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
    // Paths the samples lack are declared, with a warning that suggests the path meant.
    snapshot!(
        "schema_infer_missing_paths",
        &[
            "schema",
            "infer",
            ORDER,
            "--overrides",
            "fixtures/cli/overrides-missing.json"
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

#[test]
fn reads() {
    snapshot!(
        "reads_expr",
        &["reads", "$sum(items[type = 'book'].(price * $rate))"]
    );
    let files = [
        "reads",
        "-f",
        "fixtures/cli/check/ok.jsonata",
        "fixtures/cli/check/unknown-field.jsonata",
    ];
    snapshot!("reads_files", &files);
    snapshot!("reads_union", &[&files[..], &["--union"]].concat());
    snapshot!(
        "reads_syntax_error",
        &["reads", "-f", "fixtures/cli/check/*.jsonata"]
    );
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

/// The standard output of a run that must succeed, with nothing on standard error.
fn stdout(dir: &Path, args: &[&str]) -> String {
    let out = Command::cargo_bin("jlens")
        .unwrap()
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The cases every package answers with the same JSON (roadmap 0.33 #7), through a
/// configuration that declares the variables and the host functions.
#[test]
fn schema_uses_and_changes_of_the_shared_cases() {
    use serde_json::{Value, json};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("fixtures/uses/cases.json")).unwrap(),
    )
    .unwrap();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("uses");
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, value: &Value| {
        std::fs::write(dir.join(name), value.to_string()).unwrap();
    };
    for (version, schema, variables) in [
        ("old", "schema", "variables"),
        ("new", "newSchema", "newVariables"),
    ] {
        write(&format!("{version}.schema.json"), &fixture[schema]);
        write(
            &format!("{version}.jlens.json"),
            &json!({"rules/*.jsonata": {
                "schema": format!("{version}.schema.json"),
                "variables": fixture[variables],
                "functions": fixture["functions"],
            }}),
        );
    }
    let mut text = String::new();
    for case in fixture["cases"].as_array().unwrap() {
        let expression = case["expression"].as_str().unwrap();
        // The configuration applies to the files of `rules/`.
        std::fs::create_dir_all(dir.join("rules")).unwrap();
        std::fs::write(dir.join("rules/case.jsonata"), expression).unwrap();
        for version in ["old", "new"] {
            let config = format!("{version}.jlens.json");
            let uses = stdout(
                &dir,
                &[
                    "schema",
                    "uses",
                    "-f",
                    "rules/case.jsonata",
                    "--config",
                    &config,
                ],
            );
            std::fs::write(dir.join(format!("{version}.uses.json")), &uses).unwrap();
            if version == "old" {
                let uses: Value = serde_json::from_str(&uses).unwrap();
                assert_eq!(uses, case["uses"], "{expression}");
            }
        }
        let args = ["schema", "changes", "old.uses.json", "new.uses.json"];
        let changes: Value =
            serde_json::from_str(&stdout(&dir, &[&args[..], &["--json"]].concat())).unwrap();
        assert_eq!(changes, case["changes"], "{expression}");
        text.push_str(&format!("{expression}\n{}\n", stdout(&dir, &args)));
    }
    // The changes as text, for a person.
    insta::assert_snapshot!("schema_changes_text", text);
}

/// The recursive schema that every package answers the same about (roadmap 0.33 #12).
#[test]
fn schema_uses_and_changes_of_the_shared_recursive_schema() {
    use serde_json::Value;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("fixtures/uses/cases.json")).unwrap(),
    )
    .unwrap();
    let fixture = &fixture["recursive"];
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("recursive");
    std::fs::create_dir_all(&dir).unwrap();
    for (version, schema) in [("old", "schema"), ("new", "newSchema")] {
        std::fs::write(
            dir.join(format!("{version}.schema.json")),
            fixture[schema].to_string(),
        )
        .unwrap();
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let expression = case["expression"].as_str().unwrap();
        for version in ["old", "new"] {
            let schema = format!("{version}.schema.json");
            let uses = stdout(&dir, &["schema", "uses", "--schema", &schema, expression]);
            std::fs::write(dir.join(format!("{version}.uses.json")), &uses).unwrap();
            if version == "old" {
                let uses: Value = serde_json::from_str(&uses).unwrap();
                assert_eq!(uses, case["uses"], "{expression}");
            }
        }
        let args = [
            "schema",
            "changes",
            "old.uses.json",
            "new.uses.json",
            "--json",
        ];
        let changes: Value = serde_json::from_str(&stdout(&dir, &args)).unwrap();
        assert_eq!(changes, case["changes"], "{expression}");
    }
}

/// The expected result type and the context, from `jlens.json` and from the flags.
#[test]
fn result_type_and_context() {
    let workspace = |name: &str, args: &[&str]| {
        insta::assert_snapshot!(name, jlens_in("fixtures/workspace", args, None));
    };
    workspace(
        "check_result_config",
        &["check", "-f", "conditions/*.jsonata", "lines/*.jsonata"],
    );
    // The flags replace the configuration's `result` and `context`.
    workspace(
        "check_result_flags",
        &[
            "check",
            "--json",
            "--path",
            "lines/new.jsonata",
            "--result",
            "string,null",
            "--context",
            "customer",
            "login",
        ],
    );
    workspace(
        "schema_result_config",
        &["schema", "result", "-f", "conditions/vip.jsonata"],
    );
    workspace(
        "schema_result_flags",
        &[
            "schema",
            "result",
            "--schema",
            "order.schema.json",
            "--result",
            "number",
            "items.quantity",
        ],
    );
    snapshot!("schema_result_no_schema", &["schema", "result", "a.b"]);
    snapshot!("schema_result_syntax_error", &["schema", "result", "a +"]);
    snapshot!(
        "check_result_unknown_type",
        &["check", "--result", "bool", "a"]
    );
}

#[test]
fn schema_uses() {
    // The schema, the variables and the host functions of the configuration.
    insta::assert_snapshot!(
        "schema_uses_config",
        jlens_in(
            "fixtures/workspace",
            &["schema", "uses", "-f", "rules/discount.jsonata"],
            None
        )
    );
    // `--schema` alone, and `--path` for an expression given as an argument.
    snapshot!(
        "schema_uses_schema",
        &[
            "schema",
            "uses",
            "--schema",
            "fixtures/cli/order.schema.json",
            "customer.login & nope"
        ]
    );
    insta::assert_snapshot!(
        "schema_uses_path",
        jlens_in(
            "fixtures/workspace",
            &["schema", "uses", "--path", "rules/new.jsonata", "$minimum"],
            None
        )
    );
    snapshot!("schema_uses_no_schema", &["schema", "uses", "a.b"]);
    snapshot!("schema_uses_syntax_error", &["schema", "uses", "a +"]);
    // A file that is not an answer of `schema uses`.
    snapshot!(
        "schema_changes_not_an_answer",
        &[
            "schema",
            "changes",
            "fixtures/cli/bindings.json",
            "fixtures/cli/bindings.json"
        ]
    );
    snapshot!(
        "schema_changes_missing_file",
        &["schema", "changes", "nope.json", "nope.json"]
    );
}

/// The schema of two documents that every package loads (roadmap 0.33 #10): the other document
/// is the file beside the schema's, and the `$ref`s that are not followed are warned about.
#[test]
fn a_schema_follows_the_files_beside_it() {
    use serde_json::{Value, json};

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("fixtures/uses/cases.json")).unwrap(),
    )
    .unwrap();
    let fixture = &fixture["references"];
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("references");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("schemas")).unwrap();
    std::fs::create_dir_all(dir.join("alone")).unwrap();
    let write = |name: &str, value: &Value| {
        std::fs::write(dir.join(name), value.to_string()).unwrap();
    };
    // The schema's `$id` is in another folder of another host: the file is still beside it.
    write("schemas/order.json", &fixture["schema"]);
    write("alone/order.json", &fixture["schema"]);
    for (uri, document) in fixture["documents"].as_object().unwrap() {
        let name = uri.rsplit('/').next().unwrap();
        write(&format!("schemas/{name}"), document);
    }
    let run = |args: &[&str]| {
        let out = Command::cargo_bin("jlens")
            .unwrap()
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    };
    for case in fixture["cases"].as_array().unwrap() {
        let expression = case["expression"].as_str().unwrap();
        for folder in ["schemas", "alone"] {
            let schema = format!("{folder}/order.json");
            let (code, uses, warned) = run(&["schema", "uses", "--schema", &schema, expression]);
            assert_eq!(code, Some(0), "{warned}");
            // One line for each document that was not found, one for the place that the
            // document found does not have, and one for the `type` of `code` in that document,
            // which names no JSON type (roadmap 0.34 #10).
            let missing: &[&str] = match folder {
                "schemas" => &[
                    "address.json",
                    "carrier.json",
                    "common.json#/$defs/discount",
                    "common.json#/$defs/person/properties/code",
                ],
                _ => &["common.json", "carrier.json"],
            };
            assert_eq!(warned.lines().count(), missing.len(), "{warned}");
            for (line, name) in warned.lines().zip(missing) {
                assert!(line.starts_with("warning: schema "), "{line}");
                assert!(
                    line.contains(&format!("https://example.com/schemas/{name}")),
                    "{name} in {line}"
                );
            }
            std::fs::write(dir.join(format!("{folder}.uses.json")), &uses).unwrap();
            if folder == "schemas" {
                let uses: Value = serde_json::from_str(&uses).unwrap();
                assert_eq!(uses, case["uses"], "{expression}");
            }
        }
        let (_, changes, _) = run(&[
            "schema",
            "changes",
            "alone.uses.json",
            "schemas.uses.json",
            "--json",
        ]);
        let changes: Value = serde_json::from_str(&changes).unwrap();
        assert_eq!(changes, case["changes"], "{expression}");
    }

    // `check` reads them the same way, through a configuration too, whose variables' schemas
    // refer to files from its folder; the warnings of a schema do not fail the check.
    write(
        "jlens.json",
        &json!({"rules/*.jsonata": {
            "schema": "schemas/order.json",
            "variables": {
                "buyer": {"$ref": "schemas/common.json#/$defs/person"},
                "seller": {"$ref": "../outside.json"}
            }
        }}),
    );
    std::fs::create_dir_all(dir.join("rules")).unwrap();
    std::fs::write(
        dir.join("rules/a.jsonata"),
        "customer.nme & $buyer.nme & $seller.x & carrier.x",
    )
    .unwrap();
    let (code, _, warned) = run(&[
        "check",
        "--deny-warnings",
        "--json",
        "-f",
        "rules/a.jsonata",
    ]);
    assert_eq!(code, Some(1), "two unknown fields: {warned}");
    // Each of the schema and `$buyer` reads the `code` of `person`, whose `type` is not read.
    assert_eq!(warned.lines().count(), 7, "{warned}");
    assert_eq!(warned.matches("names what is not a JSON type").count(), 2);
    assert!(warned.contains("warning: variable $seller: "), "{warned}");
    assert!(warned.contains("warning: variable $buyer: "), "{warned}");
    let (code, found, _) = run(&["check", "--json", "-f", "rules/a.jsonata"]);
    assert_eq!(code, Some(0));
    let found: Value = serde_json::from_str(&found).unwrap();
    let codes: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["W0201", "W0201"]);
    let (code, _, warned) = run(&["check", "--schema", "schemas/order.json", "customer.name"]);
    assert_eq!(code, Some(0), "{warned}");
    assert_eq!(
        warned.lines().count(),
        5,
        "four warnings and `ok`: {warned}"
    );

    // A file beside the schema that is not JSON fails the command.
    std::fs::write(dir.join("schemas/carrier.json"), "{").unwrap();
    let (code, _, failed) = run(&["check", "--schema", "schemas/order.json", "customer.name"]);
    assert_eq!(code, Some(2), "{failed}");
    assert!(failed.contains("carrier.json"), "{failed}");
}
