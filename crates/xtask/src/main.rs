//! Build orchestration tasks. Run with `cargo xtask <task>` (alias in `.cargo/config.toml`).
//! Works the same in PowerShell and POSIX shells.

use anyhow::{Context, Result, bail};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

mod generate;

const HELP: &str = "\
usage: cargo xtask <task>

tasks:
  check               fmt --check, clippy -D warnings, tests, both lints
  lint-placeholders   fail on placeholder markers in shipped paths (--phase0 tolerates the
                      Phase 0 not-yet-implemented marker)
  lint-copy           fail on em or en dashes and banned phrases in human-facing text
  gen-schema          export JSON Schema for every public type into schema/
  gen-fixtures        write dashboard dev fixtures into packages/dashboard/fixtures/
  check-generated     fail if gen-schema output differs from the committed files
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = root();
    std::env::set_current_dir(&root)?;
    match args.first().map(String::as_str) {
        Some("check") => check(&args[1..]),
        Some("lint-placeholders") => lint_placeholders(&root, args.iter().any(|a| a == "--phase0")),
        Some("lint-copy") => lint_copy(&root),
        Some("gen-schema") => generate::schema(&root.join("schema")),
        Some("gen-fixtures") => generate::fixtures(&root.join("packages/dashboard/fixtures")),
        Some("check-generated") => check_generated(&root),
        _ => {
            print!("{HELP}");
            if args.is_empty() {
                Ok(())
            } else {
                bail!("unknown task {:?}", args[0])
            }
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    eprintln!("$ {cmd} {}", args.join(" "));
    let st = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("spawn {cmd}"))?;
    if !st.success() {
        bail!("{cmd} {} failed ({st})", args.join(" "));
    }
    Ok(())
}

fn check(extra: &[String]) -> Result<()> {
    run("cargo", &["fmt", "--all", "--check"])?;
    run(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run("cargo", &["test", "--workspace"])?;
    let root = root();
    lint_placeholders(&root, extra.iter().any(|a| a == "--phase0"))?;
    lint_copy(&root)
}

fn files_under(root: &Path, dirs: &[&str], exts: &[&str]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for d in dirs {
        let p = root.join(d);
        if p.is_file() {
            v.push(p);
            continue;
        }
        for e in WalkDir::new(&p).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            !matches!(
                n.as_ref(),
                "node_modules" | "target" | "dist" | ".git" | "internal" | "fixtures" | "vectors"
            )
        }) {
            let Ok(e) = e else { continue };
            if e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| exts.contains(&x))
            {
                v.push(e.into_path());
            }
        }
    }
    v
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Shipped paths: crate sources (not tests), packages, docs, README.
fn lint_placeholders(root: &Path, phase0: bool) -> Result<()> {
    let mut dirs: Vec<String> = Vec::new();
    for e in std::fs::read_dir(root.join("crates"))? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name != "xtask" && name != "fh-testkit" && name != "fh-fakeagent" {
            dirs.push(format!("crates/{name}/src"));
        }
    }
    dirs.extend(["packages", "docs", "README.md", "spec"].map(String::from));
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    // Built from parts so this file does not match itself.
    let words = [
        ["TO", "DO"].concat(),
        ["FIX", "ME"].concat(),
        ["lo", "rem ipsum"].concat(),
    ];
    let macros = [["todo", "!("].concat(), ["unimplemented", "!("].concat()];
    let nyi = ["NotYet", "Implemented"].concat();
    let allow = std::fs::read_to_string(root.join("crates/xtask/placeholder-allow.txt"))
        .unwrap_or_default();
    let allowed: Vec<&str> = allow
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut bad = Vec::new();
    for f in files_under(
        root,
        &dirs,
        &["rs", "ts", "tsx", "js", "css", "html", "md", "json", "toml"],
    ) {
        let r = rel(root, &f);
        if allowed.iter().any(|a| r.starts_with(a)) || r.contains("/tests/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let in_tests = text.find("#[cfg(test)]").unwrap_or(text.len());
        for (i, line) in text.lines().enumerate() {
            let offset: usize = text.lines().take(i).map(|l| l.len() + 1).sum();
            if f.extension().is_some_and(|x| x == "rs") && offset >= in_tests {
                break;
            }
            let lower = line.to_lowercase();
            let hit = words.iter().any(|w| {
                if w.contains(' ') {
                    lower.contains(w)
                } else {
                    line.contains(w.as_str())
                }
            }) || macros.iter().any(|m| line.contains(m.as_str()))
                || (!phase0 && line.contains(&nyi));
            if hit {
                bad.push(format!("{r}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    if bad.is_empty() {
        eprintln!("lint-placeholders: ok");
        Ok(())
    } else {
        for b in &bad {
            eprintln!("{b}");
        }
        bail!(
            "lint-placeholders: {} placeholder marker(s) in shipped paths",
            bad.len()
        )
    }
}

/// Banned vocabulary from the copy rules. `harness` is banned only as a verb
/// ("harness the power"), because the noun names the agent runtime that emits hooks.
fn banned() -> Vec<(Regex, &'static str)> {
    let words = [
        "delve",
        "intricate",
        "tapestry",
        "pivotal",
        "underscore",
        "landscape",
        "foster",
        "testament",
        "enhance",
        "crucial",
        "journey",
        "passion",
        "seamless",
        "robust",
        "leverage",
        "unlock",
        "empower",
        "cutting-edge",
        "game-changing",
        "revolutionary",
        "blazing",
        "supercharge",
        "elevate",
        "furthermore",
        "additionally",
        "moreover",
    ];
    let mut v: Vec<(Regex, &'static str)> = words
        .iter()
        .map(|w| {
            (
                Regex::new(&format!(r"(?i)\b{}\w*", regex::escape(w))).expect("static regex"),
                *w,
            )
        })
        .collect();
    v.push((
        Regex::new(r"(?i)\bharness(es|ed|ing)?\s+(the|your|its|their|this)\b")
            .expect("static regex"),
        "harness (verb)",
    ));
    v.push((
        Regex::new(r"(?i)\bstreamlin(e|es|ed|ing)\b\s+\w").expect("static regex"),
        "streamline (verb)",
    ));
    v.push((
        Regex::new(r"(?i)in today's fast-paced|in the world of|in conclusion")
            .expect("static regex"),
        "stock phrase",
    ));
    v
}

fn lint_copy(root: &Path) -> Result<()> {
    let dirs = [
        "README.md",
        "CHANGELOG.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        "docs",
        "spec",
        "crates",
        "packages",
        ".github",
        "action",
        "bench",
    ];
    let mut bad = Vec::new();
    let rules = banned();
    for f in files_under(
        root,
        &dirs,
        &[
            "md", "rs", "ts", "tsx", "js", "css", "html", "yml", "yaml", "toml", "json", "txt",
        ],
    ) {
        let r = rel(root, &f);
        if r.starts_with("crates/xtask/")
            || r.ends_with("Cargo.lock")
            || r.contains("package-lock.json")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.contains('\u{2014}') || line.contains('\u{2013}') {
                bad.push(format!("{r}:{}: em or en dash", i + 1));
            }
            for (re, name) in &rules {
                if re.is_match(line) {
                    bad.push(format!("{r}:{}: banned phrase \"{name}\"", i + 1));
                }
            }
        }
    }
    if bad.is_empty() {
        eprintln!("lint-copy: ok");
        Ok(())
    } else {
        for b in &bad {
            eprintln!("{b}");
        }
        bail!("lint-copy: {} problem(s)", bad.len())
    }
}

fn check_generated(root: &Path) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("fh-schema-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    generate::schema(&tmp)?;
    let mut diffs = Vec::new();
    for e in WalkDir::new(&tmp) {
        let e = e?;
        if !e.file_type().is_file() {
            continue;
        }
        let relp = e.path().strip_prefix(&tmp)?;
        let committed = root.join("schema").join(relp);
        let a = std::fs::read(e.path())?;
        if std::fs::read(&committed).ok().as_deref() != Some(&a[..]) {
            diffs.push(relp.display().to_string());
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if diffs.is_empty() {
        eprintln!("check-generated: ok");
        Ok(())
    } else {
        bail!(
            "generated schema out of date (run cargo xtask gen-schema): {}",
            diffs.join(", ")
        )
    }
}
