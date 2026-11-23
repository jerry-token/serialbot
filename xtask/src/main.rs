//! `cargo xtask`: the project's quality gates as one command.
//!
//! - `cargo xtask check` runs every check CLAUDE.md lists, in CI's order,
//!   and prints a summary. `--fast` skips the slow ones (docs, coverage,
//!   frontend, licenses) for the pre-commit hook.
//! - `cargo xtask policy` runs only the rules no linter knows (commit
//!   messages, tests with every change, the documented layout).
//! - `cargo xtask commit-msg <file>` checks one commit message (the
//!   `commit-msg` hook).
//! - `cargo xtask hooks` installs both git hooks.
//!
//! `--base <ref>` (default `origin/main`) is what the branch is compared
//! with; `--title <text>` also checks a PR title (it becomes the squashed
//! commit's subject).
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool that reports to the terminal"
)]

mod policy;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use policy::FileDiff;

/// Every check, in the order they run, with what CLAUDE.md calls it.
const CHECKS: &[(&str, &str)] = &[
    (
        "conventional-commits",
        "commit subjects are Conventional Commits, at most 72 characters",
    ),
    (
        "no-session-links",
        "no session links or trailers in commit messages",
    ),
    (
        "tests-with-change",
        "every code change comes with a test change",
    ),
    ("crate-lints", "every crate inherits the workspace lints"),
    (
        "lint-config",
        "the lints behind the quality bar stay switched on",
    ),
    ("layout-doc", "CLAUDE.md's layout names every crate"),
    (
        "rules-doc",
        "CLAUDE.md and `cargo xtask check` list the same checks",
    ),
    ("fmt", "cargo fmt --all --check"),
    (
        "clippy",
        "cargo clippy --workspace --all-targets -- -D warnings",
    ),
    ("test", "cargo test --workspace"),
    (
        "docs",
        "cargo doc --workspace --no-deps with warnings as errors",
    ),
    (
        "coverage",
        "Rust line coverage at least 85% (cargo llvm-cov)",
    ),
    (
        "frontend",
        "npm run typecheck, coverage (at least 85% lines) and build",
    ),
    ("deny", "licenses and security advisories (cargo deny)"),
];

/// The first checks are the repository rules (`cargo xtask policy`).
const POLICY_CHECKS: usize = 7;

/// Checks the pre-commit hook skips.
const SLOW: &[&str] = &["docs", "coverage", "frontend", "deny"];

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail(Vec<String>),
    Skip(String),
}

struct Options {
    fast: bool,
    only_policy: bool,
    base: String,
    title: Option<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    match args.first().map(String::as_str) {
        Some("check" | "policy") => {
            let options = match parse_options(&args) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };
            run_checks(&root, &options)
        }
        Some("commit-msg") => match args.get(1) {
            Some(file) => commit_msg(Path::new(file)),
            None => usage(),
        },
        Some("hooks") => install_hooks(&root),
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: cargo xtask check [--fast] [--base REF] [--title TEXT]\n       cargo xtask policy [--base REF] [--title TEXT]\n       cargo xtask commit-msg FILE\n       cargo xtask hooks"
    );
    ExitCode::from(2)
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        fast: false,
        only_policy: args.first().is_some_and(|a| a == "policy"),
        base: "origin/main".into(),
        title: None,
    };
    let mut rest = args.iter().skip(1);
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--fast" => options.fast = true,
            "--base" => options
                .base
                .clone_from(rest.next().ok_or("--base needs a ref")?),
            "--title" => options.title = Some(rest.next().ok_or("--title needs text")?.clone()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn run_checks(root: &Path, options: &Options) -> ExitCode {
    let changes = Changes::collect(root, &options.base);
    let mut failed = false;
    for (i, (id, what)) in CHECKS.iter().enumerate() {
        let is_policy = i < POLICY_CHECKS;
        let outcome = if options.only_policy && !is_policy {
            continue;
        } else if options.fast && SLOW.contains(id) {
            Outcome::Skip("--fast".into())
        } else {
            println!("── {id}: {what}");
            run_one(id, root, options, &changes)
        };
        match &outcome {
            Outcome::Pass => println!("✓ {id}"),
            Outcome::Skip(why) => println!("- {id} skipped ({why})"),
            Outcome::Fail(problems) => {
                failed = true;
                println!("✗ {id}");
                for p in problems {
                    println!("    {p}");
                }
            }
        }
    }
    if failed {
        println!("\nSome checks failed. CLAUDE.md explains each one.");
        ExitCode::FAILURE
    } else {
        println!("\nAll checks passed.");
        ExitCode::SUCCESS
    }
}

/// What the branch changes, compared with the merge base.
struct Changes {
    /// Subjects of the branch's commits (and the PR title, if given).
    subjects: Vec<String>,
    /// Full commit messages.
    messages: Vec<String>,
    /// Changed files, including uncommitted work.
    diffs: Vec<FileDiff>,
    /// Why nothing could be compared, if so.
    unavailable: Option<String>,
}

impl Changes {
    fn collect(root: &Path, base: &str) -> Self {
        let Some(merge_base) = git(root, &["merge-base", base, "HEAD"]) else {
            return Self {
                subjects: Vec::new(),
                messages: Vec::new(),
                diffs: Vec::new(),
                unavailable: Some(format!("no merge base with {base}")),
            };
        };
        let merge_base = merge_base.trim().to_owned();
        let log = git(
            root,
            &["log", "--format=%B%x1e", &format!("{merge_base}..HEAD")],
        )
        .unwrap_or_default();
        let messages: Vec<String> = log
            .split('\u{1e}')
            .map(|m| m.trim().to_owned())
            .filter(|m| !m.is_empty())
            .collect();
        let subjects = messages
            .iter()
            .filter_map(|m| m.lines().next().map(str::to_owned))
            .collect();
        let diff = git(root, &["diff", "--unified=0", &merge_base]).unwrap_or_default();
        Self {
            subjects,
            messages,
            diffs: policy::parse_diff(&diff),
            unavailable: None,
        }
    }
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn read(root: &Path, path: &str) -> String {
    std::fs::read_to_string(root.join(path)).unwrap_or_default()
}

fn outcome(problems: Vec<String>) -> Outcome {
    if problems.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(problems)
    }
}

fn run_one(id: &str, root: &Path, options: &Options, changes: &Changes) -> Outcome {
    let needs_history = matches!(
        id,
        "conventional-commits" | "no-session-links" | "tests-with-change"
    );
    if needs_history && let Some(why) = &changes.unavailable {
        return Outcome::Skip(why.clone());
    }
    let mut subjects = changes.subjects.clone();
    subjects.extend(options.title.clone());
    match id {
        "conventional-commits" => outcome(
            subjects
                .iter()
                .filter(|s| !s.starts_with("Merge ") && !s.starts_with("fixup! "))
                .filter_map(|s| policy::subject_problem(s))
                .collect(),
        ),
        "no-session-links" => outcome(
            changes
                .messages
                .iter()
                .chain(options.title.iter())
                .filter_map(|m| policy::session_link_problem(m))
                .collect(),
        ),
        "tests-with-change" => outcome(
            policy::tests_problem(&subjects, &changes.diffs)
                .into_iter()
                .collect(),
        ),
        "crate-lints" => {
            let members = policy::workspace_members(&read(root, "Cargo.toml"));
            outcome(
                members
                    .iter()
                    .filter_map(|m| {
                        policy::lints_problem(m, &read(root, &format!("{m}/Cargo.toml")))
                    })
                    .collect(),
            )
        }
        "lint-config" => outcome(policy::lint_config_problems(
            &read(root, "Cargo.toml"),
            &read(root, "clippy.toml"),
        )),
        "layout-doc" => {
            let members = policy::workspace_members(&read(root, "Cargo.toml"));
            let crate_dirs: Vec<String> = std::fs::read_dir(root.join("crates"))
                .map(|dir| {
                    dir.filter_map(Result::ok)
                        .filter(|e| e.path().join("Cargo.toml").exists())
                        .map(|e| format!("crates/{}", e.file_name().to_string_lossy()))
                        .collect()
                })
                .unwrap_or_default();
            outcome(policy::layout_problems(
                &members,
                &crate_dirs,
                &read(root, "CLAUDE.md"),
            ))
        }
        "rules-doc" => {
            let ids: Vec<&str> = CHECKS.iter().map(|(id, _)| *id).collect();
            outcome(policy::rules_doc_problems(&read(root, "CLAUDE.md"), &ids))
        }
        _ => run_tool(id, root),
    }
}

/// The checks that run a tool.
fn run_tool(id: &str, root: &Path) -> Outcome {
    match id {
        "fmt" => cargo(root, &["fmt", "--all", "--check"]),
        "clippy" => cargo(
            root,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        "test" => cargo(root, &["test", "--workspace"]),
        "docs" => cargo_with_env(
            root,
            &["doc", "--workspace", "--no-deps"],
            ("RUSTDOCFLAGS", "-D warnings"),
        ),
        "coverage" => {
            if !tool_present(root, &["llvm-cov", "--version"]) {
                return Outcome::Skip("cargo-llvm-cov is not installed; CI runs it".into());
            }
            cargo(
                root,
                &[
                    "llvm-cov",
                    "--workspace",
                    "--exclude",
                    "xtask",
                    "--fail-under-lines",
                    "85",
                    "--summary-only",
                ],
            )
        }
        "frontend" => frontend(root),
        "deny" => {
            if !tool_present(root, &["deny", "--version"]) {
                return Outcome::Skip("cargo-deny is not installed; CI runs it".into());
            }
            cargo(root, &["deny", "check"])
        }
        _ => Outcome::Fail(vec![format!("no such check {id}")]),
    }
}

fn tool_present(root: &Path, args: &[&str]) -> bool {
    Command::new(env!("CARGO"))
        .args(args)
        .current_dir(root)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn cargo(root: &Path, args: &[&str]) -> Outcome {
    status(
        Command::new(env!("CARGO")).args(args).current_dir(root),
        args,
    )
}

fn cargo_with_env(root: &Path, args: &[&str], env: (&str, &str)) -> Outcome {
    status(
        Command::new(env!("CARGO"))
            .args(args)
            .env(env.0, env.1)
            .current_dir(root),
        args,
    )
}

fn status(command: &mut Command, args: &[&str]) -> Outcome {
    match command.status() {
        Ok(s) if s.success() => Outcome::Pass,
        Ok(_) => Outcome::Fail(vec![format!("`{}` failed (output above)", args.join(" "))]),
        Err(e) => Outcome::Fail(vec![format!("cannot run `{}`: {e}", args.join(" "))]),
    }
}

fn frontend(root: &Path) -> Outcome {
    let app = root.join("app");
    if !app.join("node_modules").exists() {
        return Outcome::Skip("run `npm ci` in app/ first; CI runs it".into());
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    for script in ["typecheck", "coverage", "build"] {
        let result = status(
            Command::new(npm).args(["run", script]).current_dir(&app),
            &["npm", "run", script],
        );
        if result != Outcome::Pass {
            return result;
        }
    }
    Outcome::Pass
}

fn commit_msg(file: &Path) -> ExitCode {
    let Ok(text) = std::fs::read_to_string(file) else {
        eprintln!("cannot read {}", file.display());
        return ExitCode::from(2);
    };
    let message: String = text
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    let subject = message.lines().next().unwrap_or_default();
    let problems: Vec<String> = if subject.starts_with("Merge ") || subject.starts_with("fixup! ") {
        Vec::new()
    } else {
        policy::subject_problem(subject)
            .into_iter()
            .chain(policy::session_link_problem(&message))
            .collect()
    };
    if problems.is_empty() {
        return ExitCode::SUCCESS;
    }
    for p in problems {
        eprintln!("commit message: {p}");
    }
    ExitCode::FAILURE
}

fn install_hooks(root: &Path) -> ExitCode {
    let Some(dir) = git(root, &["rev-parse", "--git-path", "hooks"]) else {
        eprintln!("not a git repository");
        return ExitCode::FAILURE;
    };
    let dir = root.join(dir.trim());
    let hooks = [
        (
            "pre-commit",
            "#!/bin/sh\n# Installed by `cargo xtask hooks`.\nexec cargo xtask check --fast\n",
        ),
        (
            "commit-msg",
            "#!/bin/sh\n# Installed by `cargo xtask hooks`.\nexec cargo xtask commit-msg \"$1\"\n",
        ),
    ];
    for (name, script) in hooks {
        let path = dir.join(name);
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, script)) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }
        println!("installed {}", path.display());
    }
    ExitCode::SUCCESS
}
