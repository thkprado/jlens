// Declarations of jlens.global.js, the package as one classic script with its WebAssembly module
// embedded. `cargo xtask wasm` copies this file to `pkg/jlens.global.d.ts`.
//
// In a classic script, reference it with `/// <reference types="jlens-wasm/global" />`; in a
// module, `import "jlens-wasm/global"` loads the script and declares the global.

import type * as Package from "./jlens.js";

declare global {
  /**
   * The jlens package, set by `jlens.global.js`: the exports of `jlens.js`, with `init` for its
   * default export. With no argument, `init()` and `initSync()` instantiate the embedded module.
   */
  var jlens: Readonly<
    Omit<typeof Package, "default" | "initSync"> & {
      init: typeof Package.default;
      initSync(module?: { module: Package.SyncInitInput } | Package.SyncInitInput): Package.InitOutput;
    }
  >;
}

export {};
