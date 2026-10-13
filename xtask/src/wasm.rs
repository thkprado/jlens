//! `cargo xtask wasm` and `cargo xtask wasm-test`: the browser package.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs, thread};

use anyhow::{Context, Result, bail};

use crate::{root, site};

/// Upper bound for the gzipped `.wasm` of a release build, in bytes. Raise it deliberately.
const SIZE_BUDGET_GZIP: u64 = 600 * 1024;

const OUT_NAME: &str = "jlens";

pub fn pkg_dir() -> PathBuf {
    root().join("crates/jlens-wasm/pkg")
}

/// Builds `crates/jlens-wasm/pkg`: compiles the crate, generates the ES module and the
/// TypeScript declarations with `wasm-bindgen`, optimizes with `wasm-opt` when it is installed,
/// and writes `package.json`.
pub fn build(dev: bool) -> Result<()> {
    let profile = if dev { "dev" } else { "wasm-release" };
    let status = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args([
            "build",
            "-p",
            "jlens-wasm",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            profile,
        ])
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build failed: {status}");
    }
    let wasm = root().join(format!(
        "target/wasm32-unknown-unknown/{}/jlens_wasm.wasm",
        if dev { "debug" } else { profile }
    ));

    check_wasm_bindgen()?;
    let out = pkg_dir();
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    let status = Command::new("wasm-bindgen")
        .arg(&wasm)
        .args(["--target", "web", "--out-name", OUT_NAME, "--out-dir"])
        .arg(&out)
        .status()
        .context("running wasm-bindgen")?;
    if !status.success() {
        bail!("wasm-bindgen failed: {status}");
    }

    let bg = out.join(format!("{OUT_NAME}_bg.wasm"));
    if !dev {
        optimize(&bg)?;
    }
    write_package(&out)?;

    let raw = fs::metadata(&bg)?.len();
    let gzipped = gzip_len(&fs::read(&bg)?)?;
    println!(
        "wrote crates/jlens-wasm/pkg ({} KiB, {} KiB gzipped)",
        raw / 1024,
        gzipped / 1024
    );
    if !dev && gzipped > SIZE_BUDGET_GZIP {
        bail!(
            "the gzipped .wasm is {gzipped} bytes, over the budget of {SIZE_BUDGET_GZIP} bytes \
             (xtask/src/wasm.rs)"
        );
    }
    Ok(())
}

/// The CLI must match the `wasm-bindgen` version in Cargo.lock exactly.
fn check_wasm_bindgen() -> Result<()> {
    let lock = fs::read_to_string(root().join("Cargo.lock"))?;
    let locked = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"wasm-bindgen\"\n"))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .map(|v| v.trim_end_matches('"').to_owned())
        .context("wasm-bindgen is not in Cargo.lock")?;
    let install = format!("cargo install wasm-bindgen-cli --version {locked} --locked");
    let output = Command::new("wasm-bindgen")
        .arg("--version")
        .output()
        .with_context(|| format!("wasm-bindgen is not installed; run `{install}`"))?;
    let installed = String::from_utf8_lossy(&output.stdout);
    let installed = installed.split_whitespace().nth(1).unwrap_or("?");
    if installed != locked {
        bail!(
            "wasm-bindgen {installed} is installed, but the crate uses {locked}; run `{install}`"
        );
    }
    Ok(())
}

/// The binaryen release used through npm when `wasm-opt` is not installed.
const BINARYEN_NPM: &str = "binaryen@132.0.0";

/// Runs `wasm-opt -Os` in place: the installed binaryen if there is one, otherwise binaryen
/// from npm (through `npx`), otherwise nothing.
fn optimize(wasm: &Path) -> Result<()> {
    let npx = if cfg!(windows) { "npx.cmd" } else { "npx" };
    let mut command = if Command::new("wasm-opt").arg("--version").output().is_ok() {
        Command::new("wasm-opt")
    } else if Command::new(npx).arg("--version").output().is_ok() {
        println!("wasm-opt not found; using {BINARYEN_NPM} from npm");
        let mut command = Command::new(npx);
        command.args(["--yes", "-p", BINARYEN_NPM, "wasm-opt"]);
        command
    } else {
        println!(
            "wasm-opt not found; skipping optimization (install binaryen for a smaller .wasm)"
        );
        return Ok(());
    };
    let status = command
        .arg(wasm)
        .args([
            "-Os",
            // The WebAssembly features rustc enables by default for wasm32-unknown-unknown.
            "--enable-bulk-memory",
            "--enable-nontrapping-float-to-int",
            "--enable-sign-ext",
            "--enable-mutable-globals",
            "--enable-reference-types",
            "--enable-multivalue",
            "-o",
        ])
        .arg(wasm)
        .status()
        .context("running wasm-opt")?;
    if !status.success() {
        bail!("wasm-opt failed: {status}");
    }
    Ok(())
}

fn workspace_version() -> Result<String> {
    let manifest = fs::read_to_string(root().join("Cargo.toml"))?;
    manifest
        .lines()
        .find_map(|l| l.strip_prefix("version = \""))
        .map(|v| v.trim_end_matches('"').to_owned())
        .context("no version in Cargo.toml")
}

fn write_package(out: &Path) -> Result<()> {
    let package = format!(
        r#"{{
  "name": "jlens-wasm",
  "version": "{version}",
  "description": "JSONata in WebAssembly, with evaluation audits and schema-driven autocomplete",
  "license": "MIT",
  "repository": {{
    "type": "git",
    "url": "git+https://github.com/thkprado/jlens.git",
    "directory": "crates/jlens-wasm"
  }},
  "homepage": "https://thkprado.github.io/jlens/",
  "bugs": "https://github.com/thkprado/jlens/issues",
  "type": "module",
  "main": "./{OUT_NAME}.js",
  "types": "./{OUT_NAME}.d.ts",
  "exports": {{
    ".": {{
      "types": "./{OUT_NAME}.d.ts",
      "default": "./{OUT_NAME}.js"
    }},
    "./{OUT_NAME}_bg.wasm": "./{OUT_NAME}_bg.wasm"
  }},
  "files": [
    "{OUT_NAME}.js",
    "{OUT_NAME}.d.ts",
    "{OUT_NAME}_bg.wasm",
    "{OUT_NAME}_bg.wasm.d.ts",
    "snippets/",
    "README.md",
    "LICENSE"
  ],
  "sideEffects": false,
  "keywords": ["jsonata", "json", "query", "transform", "wasm"]
}}
"#,
        version = workspace_version()?
    );
    fs::write(out.join("package.json"), package)?;
    fs::copy(
        root().join("crates/jlens-wasm/README.md"),
        out.join("README.md"),
    )?;
    fs::copy(root().join("LICENSE"), out.join("LICENSE"))?;
    Ok(())
}

fn gzip_len(bytes: &[u8]) -> Result<u64> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes)?;
    Ok(encoder.finish()?.len() as u64)
}

/// Builds the package, then runs the JavaScript test suite in Node.js and in a headless
/// browser (Chrome, Chromium or Edge; set `JLENS_BROWSER` to choose the executable). With
/// `example`, also tests the shared addresses of `examples/web-editor` in Node.js, and checks
/// that the editor loads, evaluates its sample and restores a shared address, both from
/// the repository and as `cargo xtask site` assembles it (this loads CodeMirror from esm.sh, so
/// it needs network access).
pub fn test(require_browser: bool, example: bool) -> Result<()> {
    build(false)?;
    let tests = root().join("crates/jlens-wasm/tests");

    let status = Command::new("node")
        .arg("--test")
        .arg(tests.join("node.test.mjs"))
        .status()
        .context("running node (Node.js 20 or later is required)")?;
    if !status.success() {
        bail!("the Node.js tests failed");
    }

    let Some(browser) = find_browser() else {
        if require_browser || example {
            bail!("no browser found; set JLENS_BROWSER to a Chrome, Chromium or Edge executable");
        }
        println!("no browser found; skipping the browser tests (set JLENS_BROWSER)");
        return Ok(());
    };
    let port = serve(root())?;

    let url = format!("http://127.0.0.1:{port}/crates/jlens-wasm/tests/browser.html");
    let dom = dump_dom(&browser, &url)?;
    let log = dom
        .split_once("<pre id=\"log\">")
        .and_then(|(_, rest)| rest.split_once("</pre>"))
        .map_or("", |(log, _)| log);
    print!("{}", unescape(log));
    if !dom.contains("data-status=\"pass\"") {
        bail!("the browser tests failed in {} ({url})", browser.display());
    }
    println!("browser tests passed ({})", browser.display());

    if example {
        let status = Command::new("node")
            .arg("--test")
            .arg(root().join("examples/web-editor/share.test.mjs"))
            .status()
            .context("running node")?;
        if !status.success() {
            bail!("the tests of the web editor's shared addresses failed");
        }
        fs::write(root().join("target/editor-check.html"), EDITOR_CHECK)?;
        let url = format!(
            "http://127.0.0.1:{port}/target/editor-check.html?editor=/examples/web-editor/"
        );
        check_editor(&browser, &url)?;
        println!("web editor example loaded, evaluated its sample and restored a shared address");

        // The editor as the site serves it, without building the book.
        let site = root().join("target/site-editor-test");
        site::assemble_editor(&site.join("editor"))?;
        fs::write(site.join("editor-check.html"), EDITOR_CHECK)?;
        let port = serve(site)?;
        check_editor(
            &browser,
            &format!("http://127.0.0.1:{port}/editor-check.html?editor=/editor/"),
        )?;
        println!(
            "web editor of the site loaded, evaluated its sample and restored a shared address"
        );
    }
    Ok(())
}

/// A page that loads the web editor named by `?editor=` in a frame and copies its result once
/// the editor is ready, then loads it again from a shared address (`#s=`) and copies that result
/// too. The editor evaluates in a Web Worker, and the headless browser's virtual time does not
/// wait for a worker: it would dump the page before the worker has loaded the package. Each
/// fetch pauses virtual time, so the page fetches until the editor is ready.
const EDITOR_CHECK: &str = r#"<!doctype html>
<meta charset="utf-8">
<iframe id="editor"></iframe>
<iframe id="shared"></iframe>
<pre id="result"></pre>
<pre id="shared-result"></pre>
<script type="module">
  const editor = new URLSearchParams(location.search).get("editor");
  async function resultOf(frame, src) {
    frame.src = src;
    for (let i = 0; i < 20000; i++) {
      const page = frame.contentDocument;
      if (page?.body?.dataset.ready === "true") return page.getElementById("result").textContent;
      await fetch(location.href, { cache: "no-store" });
    }
    return "(not ready)";
  }
  const $ = (id) => document.getElementById(id);
  $("result").textContent = await resultOf($("editor"), editor);
  const { encodeShare } = await import(`${editor}share.js`);
  const fragment = await encodeShare({
    expression: '$tag & ": " & name',
    input: { name: "jlens" },
    bindings: { tag: "shared-ok" },
  });
  $("shared-result").textContent = await resultOf($("shared"), editor + fragment);
  document.body.dataset.ready = "true";
</script>
"#;

fn check_editor(browser: &Path, url: &str) -> Result<()> {
    let dom = dump_dom(browser, url)?;
    if !dom.contains("data-ready=\"true\"") || !dom.contains("FREE_SHIPPING") {
        bail!("the web editor did not load or evaluate its sample ({url})");
    }
    if !dom.contains("shared-ok: jlens") {
        bail!("the web editor did not restore a shared address ({url})");
    }
    Ok(())
}

/// Loads `url` in a headless browser and returns the DOM once the page has settled. Headless
/// browsers occasionally hang, so each attempt is bounded and retried once.
fn dump_dom(browser: &Path, url: &str) -> Result<String> {
    for attempt in 1..=2 {
        match dump_dom_once(browser, url, Duration::from_secs(90))? {
            Some(dom) => return Ok(dom),
            None => println!(
                "{} did not finish loading {url} (attempt {attempt})",
                browser.display()
            ),
        }
    }
    bail!("{} hung loading {url}", browser.display())
}

fn dump_dom_once(browser: &Path, url: &str, timeout: Duration) -> Result<Option<String>> {
    let profile = env::temp_dir().join(format!(
        "jlens-browser-test-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis()
    ));
    let mut command = Command::new(browser);
    // CI runners (GitHub's Ubuntu images, containers) cannot start the browser sandbox.
    if env::var_os("CI").is_some() {
        command.arg("--no-sandbox");
    }
    let mut child = command
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--virtual-time-budget=30000",
            "--dump-dom",
            // Console messages and errors go to stderr, to explain a page that never finishes.
            "--enable-logging=stderr",
            "--v=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {}", browser.display()))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let mut stderr = child.stderr.take().expect("piped stderr");
    let log_reader = thread::spawn(move || {
        let mut log = Vec::new();
        let _ = stderr.read_to_end(&mut log);
        log
    });
    let started = Instant::now();
    let finished = loop {
        if child.try_wait()?.is_some() {
            break true;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let out = reader.join().unwrap_or_default();
    let log = log_reader.join().unwrap_or_default();
    if !finished {
        // Show what the page logged last, to tell which step hung.
        let log = String::from_utf8_lossy(&log);
        let lines: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("CONSOLE") || l.contains("ERROR"))
            .collect();
        println!("last browser log lines:");
        for line in &lines[lines.len().saturating_sub(15)..] {
            println!("  {line}");
        }
    }
    let _ = fs::remove_dir_all(&profile);
    Ok(finished.then(|| String::from_utf8_lossy(&out).into_owned()))
}

fn unescape(html: &str) -> String {
    html.replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn find_browser() -> Option<PathBuf> {
    if let Some(path) = env::var_os("JLENS_BROWSER") {
        return Some(PathBuf::from(path));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for base in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            if let Some(base) = env::var_os(base) {
                let base = PathBuf::from(base);
                candidates.push(base.join("Google/Chrome/Application/chrome.exe"));
                candidates.push(base.join("Microsoft/Edge/Application/msedge.exe"));
            }
        }
    } else if cfg!(target_os = "macos") {
        candidates.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        candidates.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
    } else {
        for name in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
        ] {
            for dir in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
                candidates.push(dir.join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Serves the files under `dir` over HTTP on a free local port, from a background thread.
pub fn serve(dir: PathBuf) -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let dir = dir.clone();
            thread::spawn(move || {
                let _ = respond(stream, &dir);
            });
        }
    });
    Ok(port)
}

fn respond(mut stream: TcpStream, dir: &Path) -> Result<()> {
    let mut request = String::new();
    BufReader::new(&stream).read_line(&mut request)?;
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let relative = path.trim_start_matches('/');
    let mut file = dir.join(relative);
    if file.is_dir() {
        file = file.join("index.html");
    }
    let safe = !relative.split('/').any(|s| s == "..");
    match fs::read(&file) {
        Ok(body) if safe => {
            let content_type = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js" | "mjs") => "text/javascript; charset=utf-8",
                Some("wasm") => "application/wasm",
                Some("json") => "application/json",
                Some("css") => "text/css",
                Some("jsonata" | "txt" | "md") => "text/plain; charset=utf-8",
                _ => "application/octet-stream",
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
                 Cache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        _ => {
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
    }
    Ok(())
}

/// Serves the repository, or with `site` the site built by `cargo xtask site`, and prints the
/// address of the web editor.
pub fn serve_forever(site: bool) -> Result<()> {
    if site {
        let dir = site::dir();
        if !dir.join("index.html").is_file() {
            bail!("{} is not built; run `cargo xtask site`", dir.display());
        }
        let port = serve(dir)?;
        println!("serving the site at http://127.0.0.1:{port}/");
        println!("web editor: http://127.0.0.1:{port}/editor/");
    } else {
        let port = serve(root())?;
        println!("serving the repository at http://127.0.0.1:{port}/");
        println!("web editor: http://127.0.0.1:{port}/examples/web-editor/");
    }
    println!("press Ctrl+C to stop");
    loop {
        thread::park();
    }
}
