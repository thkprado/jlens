//! The server end to end: a client talks to it over an in-memory connection.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use serde_json::{Value, json};

struct Client {
    connection: Connection,
    server: Option<JoinHandle<Result<(), jlens_lsp::Error>>>,
    next_id: i32,
    /// Notifications received while waiting for responses.
    notifications: Vec<Notification>,
}

/// A workspace folder in the temporary directory, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let path = std::env::temp_dir().join(format!("jlens-lsp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn uri(&self, relative: &str) -> String {
        uri(&self.0.join(relative))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uri(path: &Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
}

fn order_schema() -> String {
    let schema = jlens_schema::Schema::infer(&[json!({
        "customer": {"login": "alice", "name": "Acme"},
        "items": [{"code": "1", "type": "apparel", "unitPrice": 10.5, "quantity": 2}]
    })]);
    schema.to_json_schema().to_string()
}

impl Client {
    /// Starts a server and initializes it with these client capabilities and options.
    fn start(folder: Option<&Folder>, capabilities: Value, options: Value) -> (Client, Value) {
        let (server, connection) = Connection::memory();
        let handle = std::thread::spawn(move || jlens_lsp::run(&server));
        let mut client = Client {
            connection,
            server: Some(handle),
            next_id: 0,
            notifications: Vec::new(),
        };
        let folders = match folder {
            Some(f) => json!([{"uri": uri(&f.0), "name": "work"}]),
            None => Value::Null,
        };
        let result = client.request(
            "initialize",
            json!({
                "capabilities": capabilities,
                "workspaceFolders": folders,
                "initializationOptions": options,
            }),
        );
        client.notify("initialized", json!({}));
        (client, result.expect("initialize succeeds"))
    }

    fn notify(&self, method: &str, params: Value) {
        let notification = Notification::new(method.into(), params);
        self.connection.sender.send(notification.into()).unwrap();
    }

    fn recv(&self) -> Message {
        self.connection
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("a message from the server")
    }

    /// Sends a request and waits for its response, keeping the notifications that arrive.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, (i32, String)> {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        let request = Request::new(id.clone(), method.into(), params);
        self.connection.sender.send(request.into()).unwrap();
        loop {
            match self.recv() {
                Message::Response(Response {
                    id: rid,
                    response_result,
                }) if rid == id => return response_result.map_err(|e| (e.code, e.message)),
                Message::Response(_) => {}
                Message::Notification(n) => self.notifications.push(n),
                // Requests from the server, such as `client/registerCapability`, succeed.
                Message::Request(r) => {
                    let response = Response::new_ok(r.id, Value::Null);
                    self.connection.sender.send(response.into()).unwrap();
                }
            }
        }
    }

    /// Waits for the next diagnostics published for `uri`.
    fn diagnostics(&mut self, uri: &str) -> Vec<(String, String, Value)> {
        let matches = |n: &Notification| {
            n.method == "textDocument/publishDiagnostics" && n.params["uri"] == uri
        };
        let notification = match self.notifications.iter().position(matches) {
            Some(i) => self.notifications.remove(i),
            None => loop {
                match self.recv() {
                    Message::Notification(n) if matches(&n) => break n,
                    Message::Notification(n) => self.notifications.push(n),
                    Message::Request(r) => {
                        let response = Response::new_ok(r.id, Value::Null);
                        self.connection.sender.send(response.into()).unwrap();
                    }
                    Message::Response(_) => {}
                }
            },
        };
        notification.params["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["code"].as_str().unwrap().to_owned(),
                    d["message"].as_str().unwrap().to_owned(),
                    d["range"].clone(),
                )
            })
            .collect()
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": "jsonata", "version": 1, "text": text}}),
        );
    }

    fn at(&mut self, method: &str, uri: &str, line: u32, character: u32) -> Value {
        self.request(
            method,
            json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
        )
        .unwrap()
    }

    fn shutdown(mut self) {
        self.request("shutdown", Value::Null).unwrap();
        self.notify("exit", Value::Null);
        let result = self.server.take().unwrap().join().unwrap();
        assert!(result.is_ok(), "{result:?}");
    }
}

fn range(start: (u32, u32), end: (u32, u32)) -> Value {
    json!({"start": {"line": start.0, "character": start.1}, "end": {"line": end.0, "character": end.1}})
}

fn labels(completion: &Value) -> Vec<&str> {
    completion
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect()
}

#[test]
fn negotiates_the_position_encoding() {
    let (client, result) = Client::start(None, json!({}), Value::Null);
    let capabilities = &result["capabilities"];
    assert_eq!(capabilities["positionEncoding"], "utf-16");
    assert_eq!(capabilities["textDocumentSync"], 2);
    assert_eq!(result["serverInfo"]["name"], "jlens");
    client.shutdown();

    let (client, result) = Client::start(
        None,
        json!({"general": {"positionEncodings": ["utf-8", "utf-16"]}}),
        Value::Null,
    );
    assert_eq!(result["capabilities"]["positionEncoding"], "utf-8");
    client.shutdown();
}

#[test]
fn diagnostics_follow_the_document() {
    let folder = Folder::new("diagnostics");
    folder.write("schemas/order.json", &order_schema());
    folder.write(
        "jlens.json",
        r#"{"rules/**/*.jsonata": {"schema": "schemas/order.json"}}"#,
    );
    let (mut client, _) = Client::start(Some(&folder), json!({}), Value::Null);

    let rule = folder.uri("rules/a.jsonata");
    client.open(&rule, "'é' & customer.Login");
    let diagnostics = client.diagnostics(&rule);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let (code, message, at) = &diagnostics[0];
    assert_eq!(code, "W0201");
    assert!(message.contains("did you mean `login`"), "{message}");
    assert_eq!(*at, range((0, 15), (0, 20)), "UTF-16 range");

    // An incremental change fixes the field.
    client.notify(
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": rule, "version": 2},
            "contentChanges": [{"range": range((0, 15), (0, 16)), "text": "l"}]
        }),
    );
    assert!(client.diagnostics(&rule).is_empty());

    // Files that no pattern matches are checked without a schema.
    let other = folder.uri("other.jsonata");
    client.open(&other, "customer.Login +");
    let codes: Vec<String> = client
        .diagnostics(&other)
        .into_iter()
        .map(|d| d.0)
        .collect();
    assert_eq!(codes, ["S0207"]);

    client.notify(
        "textDocument/didClose",
        json!({"textDocument": {"uri": rule}}),
    );
    assert!(client.diagnostics(&rule).is_empty());
    client.shutdown();
}

#[test]
fn completion_hover_and_signature_help() {
    let folder = Folder::new("completion");
    folder.write("order.json", &order_schema());
    folder.write(
        "jlens.json",
        r#"{"*.jsonata": {
            "schema": "order.json",
            "variables": {"minimum": {"type": "number"}},
            "functions": [{"name": "shipping", "signature": "<ns:n>", "params": ["weight", "region"], "summary": "Shipping cost"}]
        }}"#,
    );
    let (mut client, _) = Client::start(Some(&folder), json!({}), Value::Null);
    let rule = folder.uri("rule.jsonata");
    client.open(&rule, "(\n  '𝄞' & items[type = 'x'].unit\n)");
    client.diagnostics(&rule);

    let completion = client.at("textDocument/completion", &rule, 1, 31);
    assert_eq!(labels(&completion), ["unitPrice"]);
    let item = &completion[0];
    assert_eq!(item["kind"], 5, "a field");
    assert_eq!(
        item["textEdit"]["range"],
        range((1, 27), (1, 31)),
        "UTF-16 range"
    );
    assert_eq!(item["textEdit"]["newText"], "unitPrice");
    assert_eq!(item["detail"], "number");

    // Variables and declared functions; the service's order is kept.
    client.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": rule, "version": 2}, "contentChanges": [{"text": "$s"}]}),
    );
    let completion = client.at("textDocument/completion", &rule, 0, 2);
    let found = labels(&completion);
    assert_eq!(found[0], "$shipping", "{found:?}");
    let sort: Vec<&str> = completion
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["sortText"].as_str().unwrap())
        .collect();
    assert!(sort.is_sorted(), "{sort:?}");

    client.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": rule, "version": 3}, "contentChanges": [{"text": "$shipping(2, customer.login)"}]}),
    );
    let hover = client.at("textDocument/hover", &rule, 0, 3);
    assert_eq!(hover["contents"]["kind"], "markdown");
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Shipping cost"),
        "{hover}"
    );
    assert_eq!(hover["range"], range((0, 0), (0, 9)));
    let hover = client.at("textDocument/hover", &rule, 0, 24);
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .starts_with("`login`: string"),
        "{hover}"
    );

    let help = client.at("textDocument/signatureHelp", &rule, 0, 13);
    assert_eq!(
        help["signatures"][0]["label"],
        "$shipping($weight, $region)"
    );
    assert_eq!(help["activeParameter"], 1);
    assert!(
        help["signatures"][0]["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("<ns:n>")
    );
    assert_eq!(
        client.at("textDocument/signatureHelp", &rule, 0, 0),
        Value::Null
    );

    // Declared functions are known, and checked against their signature.
    let calls = folder.uri("calls.jsonata");
    client.open(
        &calls,
        "$shipping(2, 'eu') + $shiping(2, 'eu') + $shipping(2)",
    );
    let codes: Vec<String> = client
        .diagnostics(&calls)
        .into_iter()
        .map(|d| d.0)
        .collect();
    assert_eq!(codes, ["W0202", "W0203"]);
    client.shutdown();
}

#[test]
fn utf8_positions_when_negotiated() {
    let (mut client, _) = Client::start(
        None,
        json!({"general": {"positionEncodings": ["utf-8"]}}),
        Value::Null,
    );
    let doc = "untitled:rule";
    client.open(doc, "'é' & $upp");
    client.diagnostics(doc);
    let completion = client.at("textDocument/completion", doc, 0, 11);
    assert_eq!(labels(&completion), ["$uppercase"]);
    assert_eq!(completion[0]["textEdit"]["range"], range((0, 7), (0, 11)));
    client.shutdown();
}

#[test]
fn the_configuration_is_reloaded_when_it_changes() {
    let folder = Folder::new("reload");
    folder.write("order.json", &order_schema());
    let (mut client, _) = Client::start(
        Some(&folder),
        json!({"workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}}),
        Value::Null,
    );
    let rule = folder.uri("rule.jsonata");
    client.open(&rule, "customer.Login");
    assert!(client.diagnostics(&rule).is_empty(), "no schema yet");

    folder.write("jlens.json", r#"{"*.jsonata": {"schema": "order.json"}}"#);
    client.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes": [{"uri": folder.uri("jlens.json"), "type": 1}]}),
    );
    let codes: Vec<String> = client.diagnostics(&rule).into_iter().map(|d| d.0).collect();
    assert_eq!(codes, ["W0201"]);

    // A broken configuration is reported, and leaves the files without settings.
    folder.write("jlens.json", "{oops");
    client.notify(
        "workspace/didChangeWatchedFiles",
        json!({"changes": [{"uri": folder.uri("jlens.json"), "type": 2}]}),
    );
    assert!(client.diagnostics(&rule).is_empty());
    let warning = client
        .notifications
        .iter()
        .find(|n| n.method == "window/showMessage")
        .expect("a warning");
    assert!(
        warning.params["message"]
            .as_str()
            .unwrap()
            .contains("jlens.json"),
        "{:?}",
        warning.params
    );
    client.shutdown();
}

#[test]
fn initialization_options_configure_files_too() {
    let folder = Folder::new("options");
    folder.write("order.json", &order_schema());
    folder.write("jlens.json", r#"{"*.jsonata": {}}"#);
    let options =
        json!({"special/*.jsonata": {"schema": "order.json", "variables": {"$limit": null}}});
    let (mut client, _) = Client::start(Some(&folder), json!({}), options);
    let rule = folder.uri("special/rule.jsonata");
    client.open(&rule, "customer.Login > $limit");
    let codes: Vec<String> = client.diagnostics(&rule).into_iter().map(|d| d.0).collect();
    assert_eq!(codes, ["W0201"], "the options come before jlens.json");
    let completion = client.at("textDocument/completion", &rule, 0, 23);
    assert!(labels(&completion).contains(&"$limit"));
    client.shutdown();
}

#[test]
fn unknown_and_invalid_requests_fail_cleanly() {
    let (mut client, _) = Client::start(None, json!({}), Value::Null);
    let (code, _) = client
        .request("textDocument/definition", json!({}))
        .unwrap_err();
    assert_eq!(code, -32601, "method not found");
    let (code, _) = client
        .request("textDocument/completion", json!({"oops": 1}))
        .unwrap_err();
    assert_eq!(code, -32602, "invalid params");
    // Documents that are not open have nothing to offer.
    assert_eq!(
        client.at("textDocument/hover", "file:///nowhere.jsonata", 0, 0),
        Value::Null
    );
    // Changes to documents that are not open, and unknown notifications, are ignored.
    client.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": "file:///nowhere.jsonata", "version": 2}, "contentChanges": [{"text": "x"}]}),
    );
    client.notify("$/unknown", json!({}));
    // Malformed notifications are reported, and do not stop the server.
    client.notify("textDocument/didOpen", json!({"textDocument": 1}));
    // Positions past the end of the text are clamped.
    client.open("untitled:a", "items");
    client.diagnostics("untitled:a");
    assert!(client.request(
        "textDocument/completion",
        json!({"textDocument": {"uri": "untitled:a"}, "position": {"line": 99, "character": 99}}),
    )
    .is_ok());
    client.shutdown();
}
