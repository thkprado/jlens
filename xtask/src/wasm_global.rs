//! `jlens.global.js`: the package as one classic script, with the `.wasm` embedded.
//!
//! `wasm-bindgen --target web` writes an ES module that imports its JavaScript snippets and finds
//! the `.wasm` from `import.meta.url`, which a page opened from disk can do neither of. This file
//! rewrites that module into a classic script that sets `globalThis.jlens`, with each snippet in
//! a function of its own and the `.wasm` in base64. It changes only what it recognises in the
//! output of the `wasm-bindgen` version in `Cargo.lock`, and fails on anything else, so that a
//! new version that writes another shape stops the build instead of writing a broken script.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// The default of the asynchronous initialization in the generated module.
const DEFAULT_URL: &str = "new URL('jlens_bg.wasm', import.meta.url)";

/// The start of the synchronous initialization in the generated module.
const INIT_SYNC: &str = "function initSync(module) {\n    if (wasm !== undefined) return wasm;\n";

/// Writes `<out>/<name>.global.js` from `<out>/<name>.js`, its snippets and `<out>/<name>_bg.wasm`,
/// and returns its size in bytes.
pub fn write(out: &Path, name: &str, version: &str) -> Result<u64> {
    let module = fs::read_to_string(out.join(format!("{name}.js")))?;
    let wasm = fs::read(out.join(format!("{name}_bg.wasm")))?;
    let script = rewrite(&module, &wasm, version, |path| {
        fs::read_to_string(out.join(path)).with_context(|| format!("reading {path}"))
    })?;
    let path = out.join(format!("{name}.global.js"));
    fs::write(&path, &script)?;
    Ok(script.len() as u64)
}

/// The classic script for `module`, the ES module that `wasm-bindgen` generated; `snippet` reads
/// one of the files that it imports, by its path relative to the module.
fn rewrite(
    module: &str,
    wasm: &[u8],
    version: &str,
    mut snippet: impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    // Each snippet once, in the order of the first import of it.
    let mut snippets: BTreeMap<String, usize> = BTreeMap::new();
    let mut snippet_code = String::new();
    let mut body = String::new();
    let mut exports: Vec<String> = Vec::new();

    for line in module.split_inclusive('\n') {
        let text = line.trim_end();
        if text.starts_with("/* @ts-self-types=") {
            continue;
        }
        if let Some(rest) = text.strip_prefix("import ") {
            let (what, path) = parse_import(rest)
                .with_context(|| format!("an import the script cannot rewrite: {text}"))?;
            let index = match snippets.get(&path) {
                Some(&index) => index,
                None => {
                    let index = snippets.len();
                    let code = snippet(path.trim_start_matches("./"))?;
                    snippet_code.push_str(&wrap_snippet(&code, index, &path)?);
                    snippets.insert(path.clone(), index);
                    index
                }
            };
            body.push_str(&format!("const {what} = __jlens_snippet{index};\n"));
            continue;
        }
        if let Some(declaration) = text.strip_prefix("export ") {
            if let Some(names) = declaration
                .strip_prefix('{')
                .and_then(|r| r.strip_suffix("};"))
            {
                for item in names.split(',') {
                    let item = item.trim();
                    match item.split_once(" as ") {
                        // The initialization, which the global names `init`.
                        Some(("__wbg_init", "default")) => exports.push("init: __wbg_init".into()),
                        None if is_identifier(item) => exports.push(item.into()),
                        _ => bail!("an export the script cannot rewrite: {text}"),
                    }
                }
                continue;
            }
            let name = declared_name(declaration)
                .with_context(|| format!("an export the script cannot rewrite: {text}"))?;
            exports.push(name);
            body.push_str(&line.replacen("export ", "", 1));
            continue;
        }
        body.push_str(line);
    }

    if body.matches(DEFAULT_URL).count() != 1 {
        bail!("the generated module does not find its .wasm as expected ({DEFAULT_URL})");
    }
    let body = body.replace(DEFAULT_URL, "__jlens_wasm_bytes()");
    if body.matches(INIT_SYNC).count() != 1 {
        bail!("the generated module's initSync does not start as expected");
    }
    let body = body.replace(
        INIT_SYNC,
        &format!("{INIT_SYNC}    if (module === undefined) module = {{ module: __jlens_wasm_bytes() }};\n"),
    );
    if body.contains("import.meta") || body.contains("import(") {
        bail!("the generated module uses import.meta or import() where the script cannot");
    }
    if !exports.iter().any(|e| e == "init: __wbg_init") || !exports.iter().any(|e| e == "initSync")
    {
        bail!("the generated module does not export its initialization as expected");
    }

    let mut script = format!(
        "/* jlens-wasm {version}: the package as a classic script, with its WebAssembly module \
         embedded. It sets\n   `globalThis.jlens`, with the exports of jlens.js and `init` for its \
         default export:\n   `await jlens.init()` or `jlens.initSync()`, with no argument. */\n\
         (function () {{\n\"use strict\";\n\n"
    );
    script.push_str(&snippet_code);
    script.push_str(&body);
    script.push_str(
        "\nfunction __jlens_wasm_bytes() {\n    \
         if (typeof Uint8Array.fromBase64 === 'function') return Uint8Array.fromBase64(__jlens_wasm);\n    \
         const text = atob(__jlens_wasm);\n    \
         const bytes = new Uint8Array(text.length);\n    \
         for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i);\n    \
         return bytes;\n}\n\n",
    );
    script.push_str(&format!(
        "globalThis.jlens = Object.freeze({{ {} }});\n\n",
        exports.join(", ")
    ));
    script.push_str("// jlens_bg.wasm, in base64.\nconst __jlens_wasm = \"");
    script.push_str(&base64(wasm));
    script.push_str("\";\n})();\n");
    Ok(script)
}

/// `{ a, b } from './x.js';` or `* as name from "./x.js"`, as the binding to declare and the
/// path, which must be one of the snippets.
fn parse_import(rest: &str) -> Option<(String, String)> {
    let (what, from) = rest.split_once(" from ")?;
    let from = from.trim_end_matches(';');
    let quote = from.chars().next().filter(|q| *q == '\'' || *q == '"')?;
    let path = from.strip_prefix(quote)?.strip_suffix(quote)?;
    if !path.starts_with("./snippets/") || path.contains("..") || !path.ends_with(".js") {
        return None;
    }
    let what = if let Some(names) = what.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
        let names: Vec<&str> = names.split(',').map(str::trim).collect();
        if !names.iter().all(|n| is_identifier(n)) {
            return None;
        }
        format!("{{ {} }}", names.join(", "))
    } else {
        let name = what.strip_prefix("* as ")?;
        if !is_identifier(name) {
            return None;
        }
        name.to_owned()
    };
    Some((what, path.to_owned()))
}

/// A snippet as an expression that evaluates to the object of its exports.
fn wrap_snippet(code: &str, index: usize, path: &str) -> Result<String> {
    let mut body = String::new();
    let mut names = Vec::new();
    for line in code.split_inclusive('\n') {
        let text = line.trim_end();
        if text.starts_with("import ") || text.contains("import.meta") || text.contains("import(") {
            bail!("{path} imports, which the script cannot rewrite: {text}");
        }
        if let Some(rest) = text.strip_prefix("export ") {
            let name = declared_name(rest)
                .with_context(|| format!("{path}: an export the script cannot rewrite: {text}"))?;
            names.push(name);
            body.push_str(&line.replacen("export ", "", 1));
        } else {
            body.push_str(line);
        }
    }
    if names.is_empty() {
        bail!("{path} exports nothing");
    }
    Ok(format!(
        "// {path}\nconst __jlens_snippet{index} = (() => {{\n{body}\nreturn {{ {} }};\n}})();\n\n",
        names.join(", ")
    ))
}

/// The name of `function f(`, `async function f(`, `class C {`, `const x =` and `let x =`.
fn declared_name(declaration: &str) -> Option<String> {
    for keyword in ["function ", "async function ", "class ", "const ", "let "] {
        if let Some(rest) = declaration.strip_prefix(keyword) {
            let rest = rest.strip_prefix('*').unwrap_or(rest).trim_start();
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            return is_identifier(name).then(|| name.to_owned());
        }
    }
    None
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = "/* @ts-self-types=\"./jlens.d.ts\" */
import { a, b } from './snippets/x/inline0.js';
import * as import1 from \"./snippets/x/inline0.js\"

export class Expression {
}
export function version() {
    return a();
}
function initSync(module) {
    if (wasm !== undefined) return wasm;
    return module;
}
async function __wbg_init_once(module_or_path) {
    if (module_or_path === undefined) {
        module_or_path = new URL('jlens_bg.wasm', import.meta.url);
    }
}
export { initSync, __wbg_init as default };
";

    fn snippets(path: &str) -> Result<String> {
        assert_eq!(path, "snippets/x/inline0.js");
        Ok("export function a() {}\nexport const b = 1;\nfunction hidden() {}\n".into())
    }

    #[test]
    fn rewrites_the_module_into_a_script() {
        let script = rewrite(MODULE, b"\0asm", "1.2.3", snippets).unwrap();
        assert!(script.contains("const __jlens_snippet0 = (() => {"));
        assert_eq!(script.matches("// ./snippets/x/inline0.js").count(), 1);
        assert!(script.contains("return { a, b };"));
        assert!(script.contains("const { a, b } = __jlens_snippet0;"));
        assert!(script.contains("const import1 = __jlens_snippet0;"));
        assert!(script.contains("\nclass Expression {"));
        assert!(script.contains("\nfunction version() {"));
        assert!(script.contains("module_or_path = __jlens_wasm_bytes();"));
        assert!(
            script.contains("if (module === undefined) module = { module: __jlens_wasm_bytes() };")
        );
        assert!(script.contains(
            "globalThis.jlens = Object.freeze({ Expression, version, initSync, init: __wbg_init });"
        ));
        assert!(script.contains("const __jlens_wasm = \"AGFzbQ==\";"));
        assert!(!script.contains("import.meta"));
        assert!(!script.contains("@ts-self-types"));
        assert!(
            !script
                .lines()
                .any(|l| l.starts_with("export ") || l.starts_with("import "))
        );
    }

    #[test]
    fn fails_on_what_it_does_not_recognise() {
        let other = |from: &str, to: &str| {
            assert!(MODULE.contains(from));
            rewrite(&MODULE.replace(from, to), b"", "0", snippets).unwrap_err()
        };
        other("'./snippets/x/inline0.js'", "'https://example.com/x.js'");
        other("'./snippets/x/inline0.js'", "'./snippets/../x.js'");
        other("export function version", "export default function version");
        other("export class", "export * from './y.js';\nexport class");
        other("__wbg_init as default", "__wbg_init as default, b as c");
        other(
            "new URL('jlens_bg.wasm', import.meta.url)",
            "new URL('x.wasm', import.meta.url)",
        );
        other(
            "    if (wasm !== undefined) return wasm;\n    return module;",
            "    return module;",
        );
        other(
            "export { initSync, __wbg_init as default };",
            "export { initSync };",
        );
        other("return a();", "return import('./y.js');");

        let importing =
            |_: &str| Ok("import { c } from './c.js';\nexport function a() {}\n".into());
        rewrite(MODULE, b"", "0", importing).unwrap_err();
        let exporting = |_: &str| Ok("export { a };\n".into());
        rewrite(MODULE, b"", "0", exporting).unwrap_err();
    }

    #[test]
    fn base64_as_atob_reads_it() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd, 0x00]), "//79AA==");
    }
}
