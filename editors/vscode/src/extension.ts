// The jlens extension: starts `jlens lsp` for JSONata files (ADR 0013).

import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { LanguageClient } from "vscode-languageclient/node";

const INSTALL_URL = "https://thkprado.github.io/jlens/getting-started/installation.html";
const EXECUTABLE = process.platform === "win32" ? "jlens.exe" : "jlens";

/** A server program, and where it was found. */
interface Server {
  path: string;
  origin: "jlens.server.path" | "bundled" | "PATH";
}

let client: LanguageClient | undefined;
let output: vscode.LogOutputChannel;
// Starts and stops one after the other, so that restarts requested together do not overlap.
let queue: Promise<void> = Promise.resolve();

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  output = vscode.window.createOutputChannel("jlens", { log: true });
  context.subscriptions.push(
    output,
    vscode.commands.registerCommand("jlens.restartServer", () => restart(context)),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("jlens.server.path")) {
        void restart(context);
      }
    }),
  );
  await restart(context);
}

export function deactivate(): Promise<void> {
  return enqueue(stop);
}

function restart(context: vscode.ExtensionContext): Promise<void> {
  return enqueue(async () => {
    await stop();
    await start(context);
  });
}

function enqueue(task: () => Promise<void>): Promise<void> {
  queue = queue.then(task, task);
  return queue;
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  const server = findServer(context);
  if (typeof server === "string") {
    // Highlighting keeps working without a server. The message is not awaited: it stays open
    // until the user closes it, and would hold up the next restart.
    output.warn(server);
    void vscode.window.showWarningMessage(server, "Installation").then((choice) => {
      if (choice) {
        void vscode.env.openExternal(vscode.Uri.parse(INSTALL_URL));
      }
    });
    return;
  }
  const started = new LanguageClient(
    "jlens",
    "jlens",
    { command: server.path, args: ["lsp"] },
    {
      documentSelector: [
        { scheme: "file", language: "jsonata" },
        { scheme: "untitled", language: "jsonata" },
      ],
      outputChannel: output,
    },
  );
  try {
    await started.start();
  } catch (e) {
    output.error(`jlens: ${server.path} did not start as a language server: ${e}`);
    await started.dispose().catch(() => {});
    return;
  }
  client = started;
  const version = started.initializeResult?.serverInfo?.version ?? "of an unknown version";
  output.info(`jlens ${version}: ${server.path} (${server.origin})`);
}

async function stop(): Promise<void> {
  const stopping = client;
  client = undefined;
  await stopping?.dispose().catch((e) => output.error(`jlens: stopping the server: ${e}`));
}

/**
 * The server program: the setting `jlens.server.path`, then the binary bundled with the
 * extension, then `jlens` on the `PATH`. When there is none, the message that says so.
 */
function findServer(context: vscode.ExtensionContext): Server | string {
  // The setting has the `machine` scope and is restricted in untrusted workspaces: only the
  // user's settings choose which program runs.
  const configured = vscode.workspace.getConfiguration("jlens").get<string>("server.path")?.trim();
  if (configured) {
    return isFile(configured)
      ? { path: configured, origin: "jlens.server.path" }
      : `jlens: the setting jlens.server.path names ${configured}, which is not a file.`;
  }
  // The platform packages hold the release binary in server/ (`cargo xtask vscode`).
  const bundled = context.asAbsolutePath(path.join("server", EXECUTABLE));
  if (isFile(bundled)) {
    return { path: bundled, origin: "bundled" };
  }
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    const candidate = dir && path.join(dir, EXECUTABLE);
    if (candidate && isFile(candidate)) {
      return { path: candidate, origin: "PATH" };
    }
  }
  return "jlens: no language server found. Install the jlens CLI, or set jlens.server.path; highlighting works without it.";
}

function isFile(file: string): boolean {
  try {
    return fs.statSync(file).isFile();
  } catch {
    return false;
  }
}
