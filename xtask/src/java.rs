//! `cargo xtask java`: the Java package (ADR 0021), built by Maven over `jlens-capi`;
//! `cargo xtask java-compare`: the comparison with the other JVM implementations, and with
//! `java-compare ab`, of two builds of the package.

use std::collections::BTreeMap;
use std::env::consts::{ARCH, DLL_PREFIX, DLL_SUFFIX, OS};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::bench_table::median;
use crate::differential::flag;
use crate::root;

/// The difference `java-compare ab` marks, when the runs agree on it.
const AB_MARK: f64 = 0.02;

/// The Maven project of the Java package.
fn project() -> PathBuf {
    root().join("crates/jlens-java/java")
}

/// Builds the C API for the host, puts it where the JAR takes it from (`native/<platform>/`),
/// and runs the Maven Wrapper with `args`, `verify` when none are given.
pub fn test(args: &[String]) -> Result<()> {
    check_version()?;
    build_native()?;
    match args.is_empty() {
        true => maven(&["verify".to_owned()]),
        false => maven(args),
    }
}

/// Builds the package's JAR (tests skipped), then the harness in `benches/compare/java`, and
/// runs it with `args`; `ab` as the first argument runs [`ab`] on the rest.
pub fn compare(args: &[String]) -> Result<()> {
    if args.first().is_some_and(|a| a == "ab") {
        return ab(&args[1..]);
    }
    let harness = build_harness()?;
    let status = harness_command(&harness, None)?
        .args(args)
        .status()
        .context("running the harness")?;
    if !status.success() {
        bail!("the comparison failed");
    }
    Ok(())
}

/// Builds the C API, the package's JAR and the harness; returns the harness's directory.
fn build_harness() -> Result<PathBuf> {
    let version = check_version()?;
    build_native()?;
    maven(&["-q".into(), "package".into(), "-DskipTests".into()])?;
    let harness = root().join("benches/compare/java");
    // The JARs copied by an earlier build stay, the package's of another version among them,
    // and the classpath's `deps/*` would load whichever comes first.
    let deps = harness.join("target/deps");
    if deps.exists() {
        std::fs::remove_dir_all(&deps).with_context(|| format!("removing {}", deps.display()))?;
    }
    maven(&[
        "-q".into(),
        "-f".into(),
        harness.join("pom.xml").display().to_string(),
        format!("-Djlens.version={version}"),
        "package".into(),
    ])?;
    Ok(harness)
}

/// The JVM that runs the harness, with `jar` first on the classpath when one is given.
fn harness_command(harness: &Path, jar: Option<&Path>) -> Result<Command> {
    let java = std::env::var_os("JAVA_HOME").map_or_else(
        || PathBuf::from("java"),
        |home| Path::new(&home).join("bin/java"),
    );
    let classpath = std::env::join_paths(jar.map(Path::to_path_buf).into_iter().chain([
        harness.join("target/classes"),
        harness.join("target/deps/*"),
    ]))?;
    let mut command = Command::new(&java);
    command
        .current_dir(harness)
        .args([
            "-Xmx4g",
            "--enable-native-access=ALL-UNNAMED",
            "-Dstdout.encoding=UTF-8",
        ])
        .arg("-cp")
        .arg(classpath)
        .arg("compare.Compare");
    Ok(command)
}

/// `java-compare ab <A.jar> <B.jar> [--rounds N] [<harness argument>...]`: the harness on two
/// JARs of the package, in alternation as `cargo xtask ab` runs two programs, A then B, then B
/// then A, three rounds by default; a JVM per run. Prints each number's median, the difference
/// and each JAR's spread, the slowest run against the fastest. `*` marks a difference above
/// [`AB_MARK`] that the runs agree on: no run of one JAR reaches the other's. The harness's
/// parts are its own default, its engines `jlens,jlens-objects`.
fn ab(args: &[String]) -> Result<()> {
    let [a, b, rest @ ..] = args else {
        bail!("java-compare ab takes two JARs: <A.jar> <B.jar>");
    };
    let jars = [std::path::absolute(a)?, std::path::absolute(b)?];
    for jar in &jars {
        if !jar.is_file() {
            bail!("no JAR at {}", jar.display());
        }
    }
    let rounds = flag(rest, "--rounds")?.unwrap_or(3);
    if rounds == 0 {
        bail!("--rounds needs at least one round");
    }
    let mut harness_args = Vec::new();
    let mut given = rest.iter();
    while let Some(arg) = given.next() {
        if arg == "--rounds" {
            given.next();
        } else {
            harness_args.push(arg.clone());
        }
    }
    if harness_args.iter().any(|a| a == "limits") {
        bail!("java-compare ab measures times, and `limits` prints none");
    }
    if !harness_args.iter().any(|a| a == "--engines") {
        harness_args.extend(["--engines".to_owned(), "jlens,jlens-objects".to_owned()]);
    }
    harness_args.push("--exact".to_owned());

    let harness = build_harness()?;
    // Only the two JARs may answer for the package: the one the build copied here would hide a
    // class missing from an older JAR.
    let copied = format!("jlens-{}.jar", check_version()?);
    fs::remove_file(harness.join("target/deps").join(&copied))
        .with_context(|| format!("removing {copied} from the harness's dependencies"))?;

    let mut order = Vec::new();
    let mut times: BTreeMap<Place, [Vec<f64>; 2]> = BTreeMap::new();
    for round in 0..rounds {
        eprintln!("round {} of {rounds}", round + 1);
        let sides = if round % 2 == 0 { [0, 1] } else { [1, 0] };
        for side in sides {
            let output = harness_command(&harness, Some(&jars[side]))?
                .args(&harness_args)
                .stderr(Stdio::inherit())
                .output()
                .context("running the harness")?;
            if !output.status.success() {
                bail!("the harness failed on {}", jars[side].display());
            }
            for (place, time) in parse_tables(&String::from_utf8(output.stdout)?)? {
                if !times.contains_key(&place) {
                    order.push(place.clone());
                }
                times.entry(place).or_default()[side].push(time);
            }
        }
    }
    print_ab(&order, &times);
    println!(
        "\nA is {}, B is {}\n* a difference above {:.0}%, and no run of one JAR reaches the other's",
        jars[0].display(),
        jars[1].display(),
        AB_MARK * 100.0
    );
    Ok(())
}

/// Where a number stands in the harness's tables.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    /// The line above the table.
    table: String,
    /// The last row without numbers, which heads the rows under it.
    group: String,
    row: String,
    column: String,
}

/// The numbers of the harness's Markdown tables; NaN for a cell without one (`error`, `differs`).
fn parse_tables(output: &str) -> Result<Vec<(Place, f64)>> {
    let mut found = Vec::new();
    let (mut table, mut group) = (String::new(), String::new());
    let mut columns: Vec<String> = Vec::new();
    for line in output.lines().map(str::trim) {
        let Some(line) = line.strip_prefix('|') else {
            if !line.is_empty() {
                table = line.to_owned();
                group.clear();
                columns.clear();
            }
            continue;
        };
        let line = line.strip_suffix('|').unwrap_or(line);
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let (first, rest) = cells.split_first().context("an empty table row")?;
        if columns.is_empty() {
            columns = rest.iter().map(|c| (*c).to_owned()).collect();
        } else if first.starts_with("---") {
            continue;
        } else if rest.iter().all(|c| c.is_empty()) {
            group = (*first).to_owned();
        } else {
            if rest.len() != columns.len() {
                bail!(
                    "{} cells under {} columns: {line}",
                    rest.len(),
                    columns.len()
                );
            }
            for (column, cell) in columns.iter().zip(rest) {
                let place = Place {
                    table: table.clone(),
                    group: group.clone(),
                    row: (*first).to_owned(),
                    column: column.clone(),
                };
                found.push((place, cell.parse().unwrap_or(f64::NAN)));
            }
        }
    }
    Ok(found)
}

fn print_ab(order: &[Place], times: &BTreeMap<Place, [Vec<f64>; 2]>) {
    let label = |place: &Place| format!("{}, {}", place.row, place.column);
    let width = order.iter().map(|p| label(p).len()).max().unwrap_or(0);
    let (mut table, mut group) = ("", "");
    for place in order {
        if place.table != table {
            table = &place.table;
            group = "";
            println!(
                "\n{table}\n\n{:width$}  {:>12}  {:>12}  {:>7}  {:>8}  {:>8}",
                "", "A median", "B median", "B/A", "A spread", "B spread"
            );
        }
        if place.group != group {
            group = &place.group;
            println!("{group}");
        }
        let [a, b] = &times[place];
        println!("{:width$}  {}", label(place), ab_cells(a, b));
    }
}

/// The columns of `java-compare ab` for one number, from each JAR's runs.
fn ab_cells(a: &[f64], b: &[f64]) -> String {
    if a.iter().chain(b).any(|t| t.is_nan()) || a.len() != b.len() {
        return "not measured in every run".to_owned();
    }
    let [
        (median_a, fastest_a, slowest_a),
        (median_b, fastest_b, slowest_b),
    ] = [a, b].map(|times| {
        let mut sorted = times.to_vec();
        sorted.sort_by(f64::total_cmp);
        (median(&sorted), sorted[0], sorted[sorted.len() - 1])
    });
    let change = median_b / median_a - 1.0;
    let apart = fastest_a > slowest_b || fastest_b > slowest_a;
    format!(
        "{:>12}  {:>12}  {:>+6.1}%  {:>7.1}%  {:>7.1}%{}",
        number(median_a),
        number(median_b),
        change * 100.0,
        (slowest_a / fastest_a - 1.0) * 100.0,
        (slowest_b / fastest_b - 1.0) * 100.0,
        if apart && change.abs() > AB_MARK {
            "  *"
        } else {
            ""
        },
    )
}

/// A time to the nanosecond, a rate to the unit.
fn number(value: f64) -> String {
    match value < 100_000.0 {
        true => format!("{value:.3}"),
        false => format!("{value:.0}"),
    }
}

/// The directory of `native/` that holds the host's library, as the package's loader names it:
/// `<os>-<arch>`, and `<os>-<arch>-musl` on musl.
fn platform() -> String {
    let libc = if cfg!(target_env = "musl") {
        "-musl"
    } else {
        ""
    };
    format!("{OS}-{ARCH}{libc}")
}

/// Builds `jlens-capi` in release mode and copies it into `native/<platform>/`.
fn build_native() -> Result<()> {
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root())
        .args(["build", "--release", "-p", "jlens-capi"])
        .status()
        .context("running cargo build")?;
    if !status.success() {
        bail!("cargo build -p jlens-capi failed");
    }
    let name = format!("{DLL_PREFIX}jlens_capi{DLL_SUFFIX}");
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root().join("target"), PathBuf::from);
    let built = target.join("release").join(&name);
    let native = project().join("native").join(platform());
    fs::create_dir_all(&native)?;
    fs::copy(&built, native.join(&name)).with_context(|| format!("copying {}", built.display()))?;
    Ok(())
}

/// Runs the Java package's Maven Wrapper, in batch mode, with `args`.
fn maven(args: &[String]) -> Result<()> {
    let wrapper = match cfg!(windows) {
        true => project().join("mvnw.cmd"),
        false => project().join("mvnw"),
    };
    let status = Command::new(&wrapper)
        .current_dir(project())
        .arg("-B")
        .args(args)
        .status()
        .with_context(|| {
            format!(
                "running {} (a JDK 25 or later is required)",
                wrapper.display()
            )
        })?;
    if !status.success() {
        bail!("mvnw failed");
    }
    Ok(())
}

/// Fails unless `pom.xml` has the workspace's version, which it returns.
fn check_version() -> Result<String> {
    let version = crate::wasm::workspace_version()?;
    let pom = fs::read_to_string(project().join("pom.xml"))?;
    // The project's own version is the first one after its artifactId.
    let ours = pom
        .split_once("<artifactId>jlens</artifactId>")
        .and_then(|(_, rest)| rest.split_once("<version>"))
        .and_then(|(_, rest)| rest.split_once("</version>"))
        .map(|(ours, _)| ours);
    if ours != Some(version.as_str()) {
        bail!("crates/jlens-java/java/pom.xml must have the workspace's version, {version}");
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: &str = "\
OpenJDK 64-Bit Server VM, 25, 16 hardware threads

Microseconds per evaluation on the spike's documents:

| Expression | jlens, converted | jlens, JSON text |
|---|---:|---:|
| 1,000 items (120 KB of JSON) | | |
| `$` | 0.3400 | 70.1250 |
| 100,000 items (12,000 KB of JSON) | | |
| `$` | 0.3500 | error |

Evaluations per second of `$sum(items.price)` on 12,000 bytes of JSON text:

| Engine | 1 thread | 2 threads |
|---|---:|---:|
| jlens | 90000 | 180000 |
";

    #[test]
    fn the_tables_are_read_by_place() {
        let found = parse_tables(TABLES).unwrap();
        assert_eq!(found.len(), 6);
        let (place, time) = &found[1];
        assert_eq!(place.group, "1,000 items (120 KB of JSON)");
        assert_eq!(
            (place.row.as_str(), place.column.as_str()),
            ("`$`", "jlens, JSON text")
        );
        assert_eq!(*time, 70.125);
        assert_eq!(found[2].0.group, "100,000 items (12,000 KB of JSON)");
        assert!(found[3].1.is_nan());
        assert_eq!(found[5].0.table.split(' ').next(), Some("Evaluations"));
        assert_eq!(found[5].1, 180000.0);
    }

    #[test]
    fn a_row_that_does_not_fit_its_table_fails() {
        assert!(parse_tables("t\n| a | b |\n|---|---:|\n| x | 1 | 2 |\n").is_err());
    }

    #[test]
    fn only_a_difference_the_runs_agree_on_is_marked() {
        assert!(ab_cells(&[1.0, 1.01, 1.02], &[0.9, 0.91, 0.92]).ends_with('*'));
        // Above 2%, but one run of A is among B's.
        assert!(!ab_cells(&[1.0, 1.0, 0.9], &[0.9, 0.91, 0.92]).ends_with('*'));
        // The runs are apart, under 2%.
        assert!(!ab_cells(&[1.0, 1.0, 1.0], &[0.99, 0.99, 0.99]).ends_with('*'));
        assert_eq!(
            ab_cells(&[1.0, f64::NAN], &[1.0, 1.0]),
            "not measured in every run"
        );
    }
}
