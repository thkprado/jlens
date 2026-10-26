# Contributing to jlens

Everyone who takes part in the project follows the [code of conduct](CODE_OF_CONDUCT.md).

## Setting up

You need Rust 1.88 or later (stable). Some tasks need more:

| Task | Needs |
|---|---|
| The WebAssembly package | the `wasm32-unknown-unknown` target; `wasm-bindgen-cli` at the version in `Cargo.lock`; Node.js 20+ |
| Browser tests | Chrome, Chromium or Edge (`JLENS_BROWSER` to choose) |
| The book | `mdbook` |
| Comparisons with jsonata-js | Node.js and `npm install` in `benches/compare` |
| Comparisons with the JVM implementations | a JDK 25 or later (`cargo xtask java-compare`, `benches/compare/README.md`) |

## Checks

CI runs the following; run them before sending a change:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets         # warnings are errors in CI
cargo test --workspace                         # includes the conformance baseline
cargo xtask gen-docs --check                   # generated pages are up to date
cargo xtask doc-check                          # the book's examples run, its links resolve
cargo xtask wasm-test                          # the browser package, in Node.js and a browser
cargo semver-checks -p jlens-core -p jlens-schema -p jlens-lsp  # the API against the last release on crates.io
cargo +1.88 check --workspace --all-targets    # the minimum Rust version
cargo deny check                               # advisories, licenses and sources of dependencies
actionlint                                     # the workflows, with shellcheck on their scripts
```

The last check fails on a breaking change while `Cargo.toml` still carries the version of the
last release. A breaking change is fine before 1.0, but it is deliberate: the change that
introduces the first one after a release also raises the minor version in `Cargo.toml` (step 1
of [Releasing](#releasing)), and records the break in the `Changed` section of `CHANGELOG.md`.

Other useful tasks:

```sh
cargo xtask conformance                        # full suite run, writes tests/conformance/REPORT.md
cargo bench -p jlens-core                      # criterion benchmarks
cargo xtask ab main .                          # compare the speed of main and the working tree
PROPTEST_CASES=100000 cargo test -p jlens-core --test robustness
cargo xtask serve                              # serve the repository, for examples/web-editor
cargo xtask site && cargo xtask serve --site   # build and preview the site (book + web editor)
(cd benches/compare && npm install && node compare.mjs && node doc-examples.mjs)
cargo xtask differential                       # generated expressions, jlens against jsonata-js
cargo +nightly fuzz run evaluate --sanitizer none --debug-assertions  # fuzzing, Linux only
cargo xtask fuzz-replay evaluate <file or directory>  # replay saved fuzz inputs, any platform
```

## Guidelines

- **Conformance first.** jlens follows jsonata-js 2.2.2. A difference must be deliberate,
  explained in `docs/src/language/compatibility.md`, and, if it affects the suite, recorded in
  `tests/conformance/overrides.json` with a reason. The baseline in
  `tests/conformance/baseline.txt` may only grow.
- **Compatibility is weighed against precision, correctness and performance.** Copying a
  jsonata-js behaviour is right when its cost stays in unusual input or error paths. When it
  would lose precision, give a result that is wrong by the JSONata documentation or exposes a
  JavaScript internal, or slow down a common path (measure with `benches/`), jlens may differ
  on purpose. Either way the compatibility page records the decision and its reason, including
  the jsonata-js bugs jlens reproduces.
- **Errors have codes.** New errors reuse the JSONata codes, or `U1001` with a message for
  limits; warnings use `W` codes (`crates/jlens-core/src/error.rs`).
- **Documentation is part of the change.** Examples in the book use `jsonata-example` blocks,
  which `cargo xtask doc-check` runs. Rust snippets in the crate guide come from
  `crates/jlens-core/tests/guide.rs`. Significant design decisions get a record in `docs/adr/`.
- **Commits** describe why, not only what.
- **Vulnerabilities** are reported privately, as [SECURITY.md](SECURITY.md) describes, not in
  public issues.

## Releasing

1. Update the version in `[workspace.package]` and in the `jlens-core`, `jlens-schema`,
   `jlens-lsp` and `regress` (`jlens-regress`) entries of `[workspace.dependencies]`
   (`Cargo.toml`), and the project's `<version>` in `crates/jlens-java/java/pom.xml`
   (`cargo xtask java` fails while they differ). Also the version of the Maven and Gradle snippets of `docs/src/getting-started/installation.md` and
   `docs/src/guides/java.md`.
2. Move the `Unreleased` entries of `CHANGELOG.md` under the new version, and open the section
   with a sentence that sums up the release. The section becomes the GitHub release notes;
   preview them with `cargo xtask release-notes X.Y.Z`, which points relative links
   (`](docs/...)`) at the files of the tag.
3. Measure the tables of the [performance page](docs/src/architecture/performance.md) again
   and set the new version in the sentence above them (`for jlens X.Y.Z`): the native table
   with `cargo bench -p jlens-core`, the WebAssembly one with `node compare.mjs` in
   `benches/compare` after `cargo xtask wasm`, the Python one with `python.py speed` (its
   docstring says how), and the Java ones with `cargo xtask java-compare`
   (`benches/compare/README.md`), together with the text that quotes their numbers. Compare the new
   numbers with the old table, not only one revision with another: a change in the benchmark
   scripts shows only there.
4. Check that everything passes, including the packages:

   ```sh
   cargo package --workspace --exclude jlens-wasm --exclude xtask
   cargo xtask wasm-test --require-browser --example
   (cd crates/jlens-wasm/pkg && npm pack --dry-run)
   ```

5. Commit, push the commit to `main`, tag it `vX.Y.Z`, and push the tag. The `Release` workflow
   first builds the `jlens` binaries for Linux, macOS and Windows, the Python wheels and
   source distribution, and the Java package's JAR with the libraries of its seven targets,
   each tested on its platform (CI builds all three on each push to `main` too), and waits for
   the CI run of the tagged commit on `main`: nothing is published unless it passed, and a tag
   on a commit not pushed to `main` publishes nothing. When that CI run failed for a reason
   outside the code and passes when run again, "Re-run failed jobs" on the `Release` run
   resumes it. Then the workflow publishes `jlens-regress`, `jlens-core`, `jlens-schema`,
   `jlens-lsp` and `jlens-cli` to crates.io, in that order, and to npm with provenance
   `jlens-wasm` and the packages of the CLI (ADR 0012: the five `jlens-cli-<os>-<cpu>` packages
   built by `cargo xtask npm-cli` from the archives, then `jlens-cli`; only when the repository
   variable `PUBLISH_NPM_CLI` is `true`), and to PyPI the Python package `jlens` (ADR 0014:
   fourteen wheels and the source distribution, with attestations), and to the Central Portal
   the Java package `io.github.thkprado:jlens` (ADR 0021: the JAR, its sources and Javadoc and
   the POM, signed with GPG), which waits there for step 6, and to ghcr.io the container image
   `ghcr.io/thkprado/jlens` (`crates/jlens-cli/Dockerfile`, the two static Linux binaries; tags
   `X.Y.Z`, `X.Y` and `latest`), and to the Visual Studio Marketplace and Open VSX the VS Code
   extension `thkprado.jlens` (ADR 0013: the seven VSIX files of the targets and the one that
   uses the `PATH`, built by `cargo xtask vscode` from the archives), then attests the binary
   archives and the image (build provenance, through `actions/attest`) and drafts the GitHub
   release with the notes of the version, the archives and their `SHA256SUMS`. It needs the
   repository secret `CARGO_REGISTRY_TOKEN`. The npm packages are published from the
   environment `npm`, the Python package from the environment `pypi`, the Java package from the
   environment `maven-central`, and the extension from the environment `vscode`, set up as
   [Publishing to npm](#publishing-to-npm),
   [Publishing the Python package](#publishing-the-python-package),
   [Publishing the Java package](#publishing-the-java-package) and
   [Publishing the VS Code extension](#publishing-the-vs-code-extension) say. The image is
   pushed with the run's `GITHUB_TOKEN`. The provenance and the attestations are signed with
   the run's OIDC token, so they need no secret. If the CLI's npm job, the PyPI job or the
   extension's job fails halfway, re-run it: it skips the files whose version is already
   published. The Maven Central job uploads a new deployment each time it runs: drop the
   Portal's extra ones before publishing one.

   To publish by hand instead: `cargo publish -p jlens-regress`, then `-p jlens-core`, then
   `-p jlens-schema`, then `-p jlens-lsp`, then `-p jlens-cli`; then
   `npm publish --access public` in `crates/jlens-wasm/pkg` after `cargo xtask wasm` (without
   provenance and attestations, which only a workflow run can sign); then the Python package
   as [Publishing the Python package](#publishing-the-python-package) says; then
   `cargo xtask release-notes X.Y.Z > notes.md` and
   `gh release create vX.Y.Z --draft --verify-tag --title "jlens X.Y.Z" --notes-file notes.md`.

   If a binary fails, nothing has been published: fix the cause on `main`, then move the tag to
   the fixed commit (`git tag -f vX.Y.Z`, `git push -f origin vX.Y.Z`); re-run the failed jobs
   instead when only a runner failed. If the draft release fails after the packages are
   published, run the workflow by hand on the tag: `gh workflow run release.yml --ref vX.Y.Z`.
   That run publishes nothing; it builds the binaries of the tag, attests them and drafts the
   release. It runs the workflow as it was at the tag, never one given by a branch, so that no
   run checks out other code in the cache scope of `main`. When that workflow itself is broken,
   draft the release by hand as above (without attestations) and fix the workflow on `main` for
   the next release.
6. Review the draft on the Releases page, edit it if needed, and publish it. `cargo binstall`
   finds the binaries once the release is published. Publishing it starts the `Homebrew`
   workflow, which writes the formula of the release (`cargo xtask homebrew`, from its
   `SHA256SUMS`) to the tap `thkprado/homebrew-jlens`. It needs the repository secret
   `HOMEBREW_TAP_TOKEN`: a fine-grained personal access token whose only repository is
   `thkprado/homebrew-jlens`, with the permission Contents: read and write and nothing else.
   Such a token expires: renew it before then, or the workflow fails. To write the formula
   again (after renewing the token, say), run the workflow by hand on the tag:
   `gh workflow run homebrew.yml --ref vX.Y.Z`. It also starts the `Scoop` workflow, which
   writes the manifest of the Windows binary (`cargo xtask scoop`) to the bucket
   `thkprado/scoop-jlens`, with the secret `SCOOP_BUCKET_TOKEN`: a token like the one of the
   tap, whose only repository is `thkprado/scoop-jlens`. It is run again the same way:
   `gh workflow run scoop.yml --ref vX.Y.Z`.

   Then publish the Java package: on <https://central.sonatype.com/publishing/deployments>, the
   deployment `jlens X.Y.Z` that the `Maven Central` job uploaded is *Validated*. Check that it
   holds the JAR, the sources, the Javadoc and the POM, each with its `.asc` and checksums, and
   press **Publish**. A version published there can never be removed or replaced: when in doubt,
   **Drop** the deployment instead, fix the cause and upload again (re-run the job, or publish
   by hand as [Publishing the Java package](#publishing-the-java-package) says). It reaches
   `repo1.maven.org` within about half an hour, and the search of central.sonatype.com later.
7. Check that the packages prove where they were built:

   ```sh
   gh release download vX.Y.Z --dir dist
   cd dist
   for f in jlens-*; do gh attestation verify "$f" --repo thkprado/jlens; done
   npm view jlens-wasm@X.Y.Z dist.attestations   # lists the provenance (slsa.dev) statement
   npm view jlens-cli@X.Y.Z dist.attestations    # and of jlens-cli-<os>-<cpu> likewise
   npx -y jlens-cli@X.Y.Z --version              # the binary of this platform, from npm
   uv run --no-project --with jlens==X.Y.Z python -c "import jlens; print(jlens.__version__)"   # the wheel of this platform, from PyPI
   curl -s https://pypi.org/integrity/jlens/X.Y.Z/jlens-X.Y.Z.tar.gz/provenance | jq '.attestation_bundles[].publisher'
   (cd ../crates/jlens-java/java && ./mvnw -q dependency:get -Dartifact=io.github.thkprado:jlens:X.Y.Z)   # resolved from Maven Central
   m=https://repo1.maven.org/maven2/io/github/thkprado/jlens/X.Y.Z
   for f in jlens-X.Y.Z.jar jlens-X.Y.Z-sources.jar jlens-X.Y.Z-javadoc.jar jlens-X.Y.Z.pom; do curl -sfO "$m/$f" && curl -sfO "$m/$f.asc"; done
   gpg --keyserver keyserver.ubuntu.com --recv-keys D007C3464271F80C1C1DF11BF1B365D7F599090B   # the release key
   for f in jlens-X.Y.Z*.jar jlens-X.Y.Z.pom; do gpg --verify "$f.asc" "$f"; done   # Good signature, from that fingerprint
   echo 'System.out.println(io.github.thkprado.jlens.Expression.compile("$sum([1, 2, 3])").evaluateJson("null"))' | jshell --class-path jlens-X.Y.Z.jar -R--enable-native-access=ALL-UNNAMED -q -   # 6, the library of this platform
   brew upgrade thkprado/jlens/jlens && jlens --version   # macOS and Linux, after the Homebrew run
   scoop update jlens; jlens --version                    # Windows, after the Scoop run
   docker run --rm ghcr.io/thkprado/jlens:X.Y.Z --version
   gh attestation verify oci://ghcr.io/thkprado/jlens:X.Y.Z --repo thkprado/jlens
   code --install-extension thkprado.jlens --force   # from the Marketplace, then open a .jsonata file
   node editors/vscode/node_modules/@vscode/vsce/vsce show thkprado.jlens   # the Marketplace's version
   curl -s "https://open-vsx.org/api/-/query?extensionId=thkprado.jlens&extensionVersion=X.Y.Z" | jq '[.extensions[].targetPlatform]'
   ```

   Open VSX lists eight targets: the seven platforms and `universal`. The Marketplace serves
   each package only after checking it, a few minutes after `vsce` publishes it; until then,
   `code --install-extension` may install the package without a binary (its folder has no
   `-win32-x64`, say, suffix). The extension's **jlens** output channel shows the version of
   the server it started.

   The first push of the image creates the package `jlens` on GitHub, private: make it public
   once, in its Package settings (Change visibility), so that `docker pull` needs no login.

   The package page on npmjs.com also shows a Provenance section linking to the workflow run,
   and each file on pypi.org/project/jlens a Verified details section naming the workflow.
8. Check the site. The tag also runs the `Pages` workflow, which builds the book and the web
   editor of the tag (`cargo xtask site`) and deploys them to <https://thkprado.github.io/jlens/>
   (the `github-pages` environment accepts `v*` tags). Check that the run passed
   (`gh run list --workflow pages.yml --limit 1`), that the book's introduction names the new
   version, and that the editor evaluates an example. If only the deployment failed, re-run it
   on the tag: `gh workflow run pages.yml --ref vX.Y.Z`.

### Publishing to npm

The `npm` and `npm (CLI)` jobs of the `Release` workflow run in the GitHub environment `npm`,
whose deployment rules accept only `v*` tags. It holds no secret: npm trusts the run through
trusted publishing, which needs npm 11.5.1 or later (the jobs use Node.js 24). The seven
packages, `jlens-wasm`, `jlens-cli` and the five `jlens-cli-<os>-<cpu>`, belong to the
maintainer's account, and each registers as its trusted publisher (on npmjs.com, in the
package's **Settings**) GitHub Actions with the owner `thkprado`, the repository `jlens`, the
workflow `release.yml`, the environment `npm` and the action "Allow npm publish". A new
connection expires unless a publish validates it within two days, so register it shortly
before a release tag; the same applies when the workflow file, the environment or the
repository is renamed. A new package cannot have a trusted publisher until it exists: publish
its first version by hand or with a short-lived granular token, then register it. Once a
package has been published through trusted publishing, its **Publishing access** is set to
"Require two-factor authentication and disallow bypass 2fa tokens", so that no token can
publish it.

### Publishing the Python package

The `pypi` job of the `Release` workflow runs in the GitHub environment `pypi`, whose deployment
rules accept only `v*` tags. It holds no secret: PyPI trusts the run through trusted publishing.
The project `jlens` on PyPI belongs to the maintainer's account, and registers as its trusted
publisher the owner `thkprado`, the repository `jlens`, the workflow `release.yml` and the
environment `pypi` (on pypi.org, under the project's **Publishing** settings). Until the first
release the project does not exist: the publisher is registered as a *pending* publisher (under
the account's **Publishing**), which creates the project on its first upload. If the
workflow file, the environment or the repository is renamed, register the publisher again.
`pypa/gh-action-pypi-publish` exchanges the run's OIDC token for an upload token valid for a few
minutes, and signs a PEP 740 attestation for each file with the same token.

To publish a release by hand (without attestations), download the wheels and the source
distribution of a `Release` run that built them,
`gh run download <run> --pattern 'python-*' --dir dist`, which puts each artifact in a folder of
its own, then run `uvx twine upload --skip-existing dist/*/*` with an API token of the project.

### Publishing the Java package

The `Maven Central` job of the `Release` workflow runs in the GitHub environment
`maven-central`, whose deployment rules accept only `v*` tags. The Central Portal has no trusted
publishing: the environment holds four secrets, `MAVEN_CENTRAL_USERNAME` and
`MAVEN_CENTRAL_PASSWORD` (a user token of the Portal) and `MAVEN_GPG_KEY` and
`MAVEN_GPG_PASSPHRASE` (the release key, which signs every file, as Maven Central requires). The
job runs `mvnw -P release deploy` (the profile `release` of `pom.xml`): the sources and Javadoc
JARs, the signatures, made by the GPG plugin in Java (no `gpg` program), and the upload of one
deployment, which the Portal validates and then holds until the maintainer publishes it
(`autoPublish` false). Before the upload, the job checks that the tag is the version of the
workspace and of `pom.xml` (CI's Java job fails on each push while those two differ), and that
the JAR it built is byte for byte the one the JAR workflow tested on the seven targets: the JAR's
entries have a fixed time (`project.build.outputTimestamp`).

To set it up, once:

1. Sign in to <https://central.sonatype.com> with the GitHub account `thkprado`. The Portal then
   verifies the namespace `io.github.thkprado` by itself; it is listed under **Namespaces**.
2. Under **View Account**, **Generate User Token**. Set its two parts as the environment's
   secrets `MAVEN_CENTRAL_USERNAME` and `MAVEN_CENTRAL_PASSWORD`. Generating a new token revokes
   the old one: set the secrets again then.
3. Make the release key, an Ed25519 signing key, with the GitHub noreply address, never a
   personal one: `gpg --quick-gen-key "jlens releases <2347749+thkprado@users.noreply.github.com>"
   ed25519 sign never`. Keep the revocation certificate that gpg writes, and the key, out of the
   repository. Send the public key to a key server the Portal reads, one that keeps its user ID
   without a confirmation mail, which a noreply address cannot receive:
   `gpg --keyserver keyserver.ubuntu.com --send-keys <fingerprint>`. Set
   `gpg --armor --export-secret-keys <fingerprint>` as the secret `MAVEN_GPG_KEY` and its
   passphrase as `MAVEN_GPG_PASSPHRASE`.

The release key's fingerprint is `D007C3464271F80C1C1DF11BF1B365D7F599090B` (`jlens releases`,
Ed25519, made 2026-10-26).

If the key is lost or leaks, revoke it with the certificate, send the revocation to the key
server, and make a new one: the versions already published stay signed by the old key.

To publish a release by hand, download the libraries of a `Release` (or CI) run,
`gh run download <run> --pattern 'java-native-*' --dir dl`, which puts each artifact in a folder
of its own, copy them with `cp -r dl/*/* crates/jlens-java/java/native/`, then, in
`crates/jlens-java/java`, with the Portal's token in a server `central` of
`~/.m2/settings.xml` and the key in the two environment variables, run
`./mvnw -B -P release deploy -DskipTests`, and publish the deployment on the Portal.
`-DskipUpload=true` builds the bundle without sending it
(`target/central-publishing/central-bundle.zip`).

### Publishing the VS Code extension

The `vscode` job of the `Release` workflow runs in the GitHub environment `vscode`, which holds
what both registries need. Its deployment rules accept only `v*` tags. The publisher `thkprado`
on the Marketplace and the namespace `thkprado` on Open VSX belong to the maintainer.

The Marketplace is reached as a Microsoft Entra ID application, `jlens-release`, without a
secret: global Azure DevOps personal access tokens stop working on 2026-12-01. It lives in the
tenant `thkpradooutlook.onmicrosoft.com`, which the Azure sign-up of the maintainer's Microsoft
account created (a personal account alone cannot register applications). An application needs
no subscription and no role, so it keeps working whatever happens to the subscription. To set
it up:

1. In the Azure portal, under **App registrations**, register an application for this tenant
   only, without a redirect URI. Under **Certificates & secrets**, **Federated credentials**,
   add one for the scenario *GitHub Actions deploying Azure resources*: organization `thkprado`
   (ID 2347749), repository `jlens` (ID 1413211223), entity *Environment*, name `vscode`. The
   repository's OIDC tokens carry immutable subjects
   (`gh api repos/thkprado/jlens/actions/oidc/customization/sub`), so the subject is
   `repo:thkprado@2347749/jlens@1413211223:environment:vscode`.
2. In the repository's environment `vscode`, set the variables `AZURE_CLIENT_ID` (the
   application's client ID) and `AZURE_TENANT_ID`.
3. Only a workflow run can sign in as the application, so its Marketplace ID is read by one: let
   the environment accept the branch `main` too, run `gh workflow run marketplace-identity.yml`,
   and read the ID in the log of its `az rest` step (`gh run view --log`). Remove `main` from
   the environment's rules after step 4.
4. On <https://marketplace.visualstudio.com/manage/publishers/thkprado>, add that ID as a member
   with the role **Contributor**. The member list shows it as `<tenant ID>\<object ID>`. Run
   the workflow again: its last step (`vsce verify-pat --azure-credential thkprado`) now passes.

Nothing expires. If the environment, the repository or the publisher is renamed or recreated,
change the federated credential, run step 3 again and replace the member.

Open VSX is reached through trusted publishing: the namespace `thkprado`, which the account
`thkprado` owns, registers a trusted publisher for the repository `thkprado/jlens`, the workflow
`release.yml` and the environment `vscode` (in the open-vsx.org settings, under the namespace).
With the secret `OVSX_PAT` unset, `ovsx` exchanges the run's OIDC token for a token valid for a
few minutes. If the workflow file or the environment is renamed, register the publisher again.
Open VSX publishes the first version of a new extension only with an access token: create one
under **Access Tokens**, set it as the environment's secret `OVSX_PAT` for that release, then
delete the secret and the token.

To publish a release by hand, build its VSIX files on Linux or macOS (Windows loses the
executable bit): `gh release download vX.Y.Z --dir dist`, `npm ci` in `editors/vscode`, then
`cargo xtask vscode dist`, which writes them to `target/vscode`. Upload them on the publisher's
page of the Marketplace, and publish them to Open VSX with
`npx ovsx publish --skip-duplicate --packagePath target/vscode/*.vsix -p <token>`.

## License

By contributing, you agree that your contributions are licensed under the MIT license.
