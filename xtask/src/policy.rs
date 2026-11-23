// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The project's written rules (CLAUDE.md) that no compiler or linter
//! checks, as pure functions of the repository's text: commit messages,
//! the diff, manifests and CLAUDE.md itself. `main.rs` gathers the inputs
//! from git and the file system.

use std::collections::BTreeSet;

use regex::Regex;

/// One file's change: its path and the lines added or removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FileDiff {
    /// Path relative to the repository root, with `/` separators.
    pub(crate) path: String,
    /// Lines added or removed (without the `+`/`-` marker).
    pub(crate) changed_lines: Vec<String>,
}

/// Commit types that change no behaviour and so need no tests.
const NO_TEST_TYPES: &[&str] = &["docs", "ci", "chore", "style", "build", "revert"];

/// Every type a commit subject may start with.
const TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Longest commit subject (it becomes the PR title and changelog line).
pub(crate) const MAX_SUBJECT: usize = 72;

/// Checks a Conventional Commits subject such as `feat(cli): send`.
/// Returns what is wrong, or `None`.
pub(crate) fn subject_problem(subject: &str) -> Option<String> {
    let pattern = format!(r"^({})(\([a-z0-9-]+\))?!?: \S", TYPES.join("|"));
    let re = Regex::new(&pattern).ok()?;
    if !re.is_match(subject) {
        return Some(format!(
            "{subject:?} is not a Conventional Commit (type(scope): summary, type one of {})",
            TYPES.join(", ")
        ));
    }
    // A squash merge adds ` (#123)` to the PR title, which was checked.
    let own = Regex::new(r" \(#\d+\)$").ok().map_or(subject, |pr| {
        pr.find(subject).map_or(subject, |m| &subject[..m.start()])
    });
    let length = own.chars().count();
    if length > MAX_SUBJECT {
        return Some(format!(
            "{subject:?} is {length} characters; keep subjects at most {MAX_SUBJECT}"
        ));
    }
    None
}

/// The type of a commit subject (`feat` in `feat(cli)!: x`), if it has one.
pub(crate) fn subject_type(subject: &str) -> Option<&str> {
    let end = subject.find(['(', '!', ':'])?;
    Some(&subject[..end])
}

/// Session links and session trailers do not belong in commits or PRs.
pub(crate) fn session_link_problem(message: &str) -> Option<String> {
    let line = message.lines().find(|l| {
        l.trim_start().starts_with("Claude-Session:") || l.contains("claude.ai/code/session")
    })?;
    Some(format!("remove the session link: {:?}", line.trim()))
}

/// The license line every source file starts with.
pub(crate) const SPDX: &str = "SPDX-License-Identifier: GPL-3.0-only";

/// The copyright line after it.
pub(crate) const COPYRIGHT: &str =
    "Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot";

/// Source files that carry the header: code and styles.
pub(crate) fn needs_header(path: &str) -> bool {
    ["rs", "ts", "tsx", "js", "mjs", "css", "py"]
        .iter()
        .any(|ext| has_ext(path, ext))
}

/// What is wrong with the start of `path`, or `None`: its first lines
/// (after a `#!` line) must be [`SPDX`] and [`COPYRIGHT`] as comments.
pub(crate) fn header_problem(path: &str, text: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| l.starts_with("#!"));
    let strip = |l: &str| {
        l.trim_start_matches(['/', '#', '*', ' '])
            .trim_end_matches(['*', '/', ' '])
            .to_owned()
    };
    let first = lines.next().map(strip);
    let second = lines.next().map(strip);
    if first.as_deref() == Some(SPDX) && second.as_deref() == Some(COPYRIGHT) {
        return None;
    }
    Some(format!(
        "{path}: start it with the comments \"{SPDX}\" and \"{COPYRIGHT}\""
    ))
}

/// Whether `path` ends in `.<ext>` (any case).
fn has_ext(path: &str, ext: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether `path` is production code (not a test, not a build script).
pub(crate) fn is_code(path: &str) -> bool {
    if path.contains("/tests/") || path.ends_with("/build.rs") {
        return false;
    }
    let rust = has_ext(path, "rs")
        && (path.starts_with("crates/")
            || path.starts_with("app/src-tauri/src/")
            || path.starts_with("xtask/src/"));
    let web = path.starts_with("app/src/")
        && (has_ext(path, "ts") || has_ext(path, "tsx"))
        && !path.contains(".test.")
        && !path.ends_with("test-setup.ts");
    rust || web
}

/// Whether a change touches tests: a test file, or test code inside a
/// Rust file (`#[test]`, `mod tests`, assertions).
pub(crate) fn touches_tests(diff: &FileDiff) -> bool {
    let p = &diff.path;
    if p.contains("/tests/") || p.contains(".test.ts") {
        return true;
    }
    has_ext(p, "rs")
        && diff.changed_lines.iter().any(|l| {
            let l = l.trim();
            l.starts_with("#[test]")
                || l.starts_with("#[tokio::test")
                || l.contains("mod tests")
                || l.starts_with("assert")
        })
}

/// "Every change ships with tests written in the same change": a change
/// to production code needs a change to tests, unless every commit is of a
/// type that changes no behaviour (docs, ci, chore, style, build, revert).
/// With no commits (uncommitted work) code changes always need tests.
pub(crate) fn tests_problem(subjects: &[String], diffs: &[FileDiff]) -> Option<String> {
    let code: Vec<&str> = diffs
        .iter()
        .filter(|d| is_code(&d.path))
        .map(|d| d.path.as_str())
        .collect();
    if code.is_empty() || diffs.iter().any(touches_tests) {
        return None;
    }
    let exempt = !subjects.is_empty()
        && subjects
            .iter()
            .all(|s| subject_type(s).is_some_and(|t| NO_TEST_TYPES.contains(&t)));
    if exempt {
        return None;
    }
    Some(format!(
        "code changed without any test change: {}",
        code.join(", ")
    ))
}

/// Every member must inherit the workspace lints (they forbid `unsafe`,
/// deny `unwrap`/`expect` outside tests and require docs).
pub(crate) fn lints_problem(member: &str, manifest: &str) -> Option<String> {
    let inherits = manifest.split("\n[").any(|section| {
        section.trim_start_matches('[').starts_with("lints]")
            && section.contains("workspace = true")
    });
    (!inherits).then(|| format!("{member}/Cargo.toml must have `[lints]` with `workspace = true`"))
}

/// The workspace lints and clippy settings the quality bar relies on.
pub(crate) fn lint_config_problems(workspace_manifest: &str, clippy_toml: &str) -> Vec<String> {
    let required = [
        ("unsafe_code", "forbid"),
        ("missing_docs", "warn"),
        ("unwrap_used", "warn"),
        ("expect_used", "warn"),
        ("print_stdout", "warn"),
        ("print_stderr", "warn"),
    ];
    let mut problems: Vec<String> = required
        .iter()
        .filter(|(lint, level)| {
            !workspace_manifest
                .lines()
                .any(|l| l.replace(' ', "") == format!("{lint}=\"{level}\""))
        })
        .map(|(lint, level)| format!("Cargo.toml [workspace.lints] must set {lint} = \"{level}\""))
        .collect();
    for setting in [
        "allow-unwrap-in-tests = true",
        "allow-expect-in-tests = true",
    ] {
        if !clippy_toml.lines().any(|l| l.trim() == setting) {
            problems.push(format!("clippy.toml must set {setting}"));
        }
    }
    problems
}

/// CLAUDE.md's layout must name every workspace member and every crate
/// directory must be a member, so the map of the code stays true.
pub(crate) fn layout_problems(
    members: &[String],
    crate_dirs: &[String],
    claude_md: &str,
) -> Vec<String> {
    let mut problems = Vec::new();
    for member in members {
        let short = member.trim_end_matches("/src-tauri");
        if !claude_md.contains(&format!("`{member}`"))
            && !claude_md.contains(&format!("`{short}/`"))
        {
            problems.push(format!(
                "CLAUDE.md does not describe the workspace member {member}"
            ));
        }
    }
    for dir in crate_dirs {
        if !members.contains(dir) {
            problems.push(format!("{dir} is not a workspace member"));
        }
    }
    problems
}

/// The members listed in the workspace manifest.
pub(crate) fn workspace_members(manifest: &str) -> Vec<String> {
    let Some(start) = manifest.find("members = [") else {
        return Vec::new();
    };
    let rest = &manifest[start + "members = [".len()..];
    let end = rest.find(']').unwrap_or(rest.len());
    rest[..end]
        .split(',')
        .map(|m| m.trim().trim_matches('"').to_owned())
        .filter(|m| !m.is_empty())
        .collect()
}

/// The check ids CLAUDE.md names as `` `check:<id>` ``.
pub(crate) fn documented_checks(claude_md: &str) -> BTreeSet<String> {
    let re = Regex::new(r"`check:([a-z-]+)`").ok();
    re.map(|re| {
        re.captures_iter(claude_md)
            .map(|c| c[1].to_owned())
            .collect()
    })
    .unwrap_or_default()
}

/// CLAUDE.md and `cargo xtask check` must describe the same checks.
pub(crate) fn rules_doc_problems(claude_md: &str, implemented: &[&str]) -> Vec<String> {
    let documented = documented_checks(claude_md);
    let implemented: BTreeSet<String> = implemented.iter().map(|s| (*s).to_owned()).collect();
    let mut problems: Vec<String> = documented
        .difference(&implemented)
        .map(|id| format!("CLAUDE.md names `check:{id}`, which `cargo xtask check` does not run"))
        .collect();
    problems.extend(
        implemented
            .difference(&documented)
            .map(|id| format!("`cargo xtask check` runs {id}, which CLAUDE.md does not explain")),
    );
    problems
}

/// Parses `git diff --unified=0` output into per-file changes.
pub(crate) fn parse_diff(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut old_path = String::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("--- a/") {
            path.clone_into(&mut old_path);
        } else if line == "--- /dev/null" {
            old_path.clear();
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            files.push(FileDiff {
                path: path.to_owned(),
                changed_lines: Vec::new(),
            });
        } else if line == "+++ /dev/null" {
            // A deleted file keeps its old name.
            files.push(FileDiff {
                path: old_path.clone(),
                changed_lines: Vec::new(),
            });
        } else if let Some(file) = files.last_mut()
            && (line.starts_with('+') || line.starts_with('-'))
        {
            file.changed_lines.push(line[1..].to_owned());
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str, lines: &[&str]) -> FileDiff {
        FileDiff {
            path: path.into(),
            changed_lines: lines.iter().map(|l| (*l).to_owned()).collect(),
        }
    }

    #[test]
    fn subjects_follow_conventional_commits() {
        for ok in [
            "feat(cli): send, with expect",
            "fix: keep the capture",
            "refactor(mcp)!: move approvals",
            "ci: add the policy job",
        ] {
            assert_eq!(subject_problem(ok), None, "{ok}");
        }
        assert!(
            subject_problem("Add stuff")
                .unwrap()
                .contains("Conventional")
        );
        assert!(
            subject_problem("feat(CLI): x").is_some(),
            "scopes are lowercase"
        );
        assert!(subject_problem("feat:no space").is_some());
        assert!(
            subject_problem(&format!("feat: {}", "x".repeat(70)))
                .unwrap()
                .contains("at most 72")
        );
        // The ` (#N)` a squash merge adds to a checked PR title is not counted.
        let title = format!("feat: {}", "x".repeat(66));
        assert_eq!(subject_problem(&format!("{title} (#100)")), None);
        assert!(subject_problem(&format!("{title}x (#100)")).is_some());
        assert!(subject_problem(&format!("feat: {} (#1", "x".repeat(66))).is_some());
        assert_eq!(subject_type("feat(cli)!: x"), Some("feat"));
        assert_eq!(subject_type("docs: x"), Some("docs"));
        assert_eq!(subject_type("nothing"), None);
    }

    #[test]
    fn source_files_carry_the_header() {
        assert!(needs_header("app/src/App.tsx"));
        assert!(needs_header("site/build.py"));
        assert!(!needs_header("README.md"));
        let rust = format!("// {SPDX}\n// {COPYRIGHT}\n//! Docs.\n");
        assert_eq!(header_problem("a.rs", &rust), None);
        let css = format!("/* {SPDX}\n * {COPYRIGHT} */\nbody {{}}\n");
        assert_eq!(header_problem("a.css", &css), None);
        let script = format!("#!/usr/bin/env python3\n# {SPDX}\n# {COPYRIGHT}\n");
        assert_eq!(header_problem("a.py", &script), None);
        assert!(
            header_problem("a.rs", "//! Docs.\n")
                .unwrap()
                .contains("a.rs: start it")
        );
        let other = format!("// {SPDX}\n// Copyright (C) 2026 Someone Else\n");
        assert!(header_problem("a.ts", &other).is_some());
        assert!(header_problem("a.ts", "").is_some());
    }

    #[test]
    fn session_links_are_refused() {
        assert!(
            session_link_problem("feat: x\n\nClaude-Session: https://claude.ai/code/session_1")
                .is_some()
        );
        assert!(session_link_problem("see https://claude.ai/code/session_abc").is_some());
        assert_eq!(
            session_link_problem("feat: x\n\nCo-Authored-By: Claude <noreply@anthropic.com>"),
            None
        );
    }

    #[test]
    fn code_and_tests_are_told_apart() {
        assert!(is_code("crates/serialbot-cli/src/args.rs"));
        assert!(is_code("app/src-tauri/src/commands.rs"));
        assert!(is_code("app/src/App.tsx"));
        assert!(is_code("xtask/src/policy.rs"));
        assert!(!is_code("crates/serialbot-cli/tests/e2e.rs"));
        assert!(!is_code("app/src/App.test.tsx"));
        assert!(!is_code("app/src/test-setup.ts"));
        assert!(!is_code("app/src-tauri/build.rs"));
        assert!(!is_code("README.md"));
        assert!(touches_tests(&diff("crates/x/tests/a.rs", &[])));
        assert!(touches_tests(&diff("app/src/a.test.ts", &[])));
        assert!(touches_tests(&diff("crates/x/src/a.rs", &["    #[test]"])));
        assert!(touches_tests(&diff(
            "crates/x/src/a.rs",
            &["        assert_eq!(a, b);"]
        )));
        assert!(touches_tests(&diff("crates/x/src/a.rs", &["mod tests {"])));
        assert!(!touches_tests(&diff("crates/x/src/a.rs", &["fn a() {}"])));
    }

    #[test]
    fn code_changes_need_test_changes() {
        let feat = vec!["feat(cli): x".to_owned()];
        let code = diff("crates/serialbot-cli/src/a.rs", &["fn a() {}"]);
        let test = diff("crates/serialbot-cli/src/a.rs", &["    #[test]"]);
        assert!(
            tests_problem(&feat, std::slice::from_ref(&code))
                .unwrap()
                .contains("a.rs")
        );
        assert_eq!(tests_problem(&feat, &[code.clone(), test]), None);
        assert_eq!(
            tests_problem(
                &["docs: x".into(), "ci: y".into()],
                std::slice::from_ref(&code)
            ),
            None
        );
        assert!(
            tests_problem(
                &["docs: x".into(), "fix: y".into()],
                std::slice::from_ref(&code)
            )
            .is_some()
        );
        assert!(
            tests_problem(&[], &[code]).is_some(),
            "uncommitted code needs tests too"
        );
        assert_eq!(tests_problem(&feat, &[diff("README.md", &["x"])]), None);
    }

    #[test]
    fn members_inherit_the_workspace_lints() {
        assert_eq!(
            lints_problem(
                "a",
                "[package]\nname = \"a\"\n\n[lints]\nworkspace = true\n"
            ),
            None
        );
        assert!(lints_problem("a", "[package]\nname = \"a\"\n").is_some());
        assert!(lints_problem("a", "[package]\n[lints.rust]\nunsafe_code = \"allow\"\n").is_some());
    }

    #[test]
    fn the_lint_config_cannot_be_weakened() {
        let manifest = "unsafe_code = \"forbid\"\nmissing_docs = \"warn\"\nunwrap_used = \"warn\"\nexpect_used = \"warn\"\nprint_stdout = \"warn\"\nprint_stderr = \"warn\"\n";
        let clippy = "allow-unwrap-in-tests = true\nallow-expect-in-tests = true\n";
        assert!(lint_config_problems(manifest, clippy).is_empty());
        let weak = manifest.replace("forbid", "allow");
        assert_eq!(lint_config_problems(&weak, clippy).len(), 1);
        assert_eq!(lint_config_problems(manifest, "").len(), 2);
    }

    #[test]
    fn the_layout_names_every_member() {
        let manifest =
            "members = [\n    \"crates/a\",\n    \"app/src-tauri\",\n    \"xtask\",\n]\n";
        let members = workspace_members(manifest);
        assert_eq!(members, ["crates/a", "app/src-tauri", "xtask"]);
        let md = "- `crates/a` — a\n- `app/` — app\n- `xtask` — checks\n";
        assert!(layout_problems(&members, &["crates/a".into()], md).is_empty());
        let problems = layout_problems(
            &members,
            &["crates/a".into(), "crates/b".into()],
            "- `crates/a`",
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(workspace_members("").is_empty());
    }

    #[test]
    fn claude_md_and_the_checks_agree() {
        let md = "| a | `check:fmt` |\n| b | `check:tests-with-change` |";
        assert!(rules_doc_problems(md, &["fmt", "tests-with-change"]).is_empty());
        let problems = rules_doc_problems(md, &["fmt", "coverage"]);
        assert_eq!(problems.len(), 2, "{problems:?}");
    }

    #[test]
    fn diffs_are_parsed_per_file() {
        let text = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/gone.rs b/gone.rs\n--- a/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n";
        let files = parse_diff(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "x.rs");
        assert_eq!(files[0].changed_lines, ["old", "new"]);
        assert_eq!(files[1].path, "gone.rs", "deleted files keep their name");
        assert_eq!(files[1].changed_lines, ["bye"]);
    }
}
