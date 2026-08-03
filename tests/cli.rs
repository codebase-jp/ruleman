//! Tests that run the real binary.
//!
//! The unit tests cover the engine; these cover the surface a user actually
//! touches — argument parsing, exit codes, what lands on stdout vs stderr, and
//! the files a subcommand writes. Those were only ever verified by hand before.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test, removed and recreated so runs don't leak
/// into each other. Named after the test, since tests run in parallel.
fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ruleman_cli_{}", name));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ruleman(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ruleman"))
        .args(args)
        .current_dir(dir)
        // `auto` would switch to workflow commands when these tests run inside
        // GitHub Actions, so pin the format everywhere it matters.
        .env_remove("GITHUB_ACTIONS")
        .output()
        .expect("failed to run the ruleman binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("terminated by a signal")
}

fn write(dir: &Path, path: &str, contents: &str) {
    let full = dir.join(path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, contents).unwrap();
}

#[test]
fn version_is_reported() {
    let dir = sandbox("version");
    let output = ruleman(&dir, &["--version"]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).starts_with("ruleman "), "{:?}", output);
}

#[test]
fn passing_checks_exit_zero_and_say_so_on_stdout() {
    let dir = sandbox("pass");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
    );
    write(&dir, "README.md", "# hi\n");

    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("All checks passed"),
        "{:?}",
        output
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn failures_go_to_stderr_with_a_count_and_a_nonzero_exit() {
    let dir = sandbox("fail");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [
             { "type": "file", "files": ["README.md"] },
             { "type": "file", "severity": "warn", "files": ["CHANGELOG.md"] }
           ] }"#,
    );

    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 1);
    let errors = stderr(&output);
    assert!(
        errors.contains("error: required file 'README.md'"),
        "{}",
        errors
    );
    assert!(
        errors.contains("warning: required file 'CHANGELOG.md'"),
        "{}",
        errors
    );
    assert!(errors.contains("1 error, 1 warning"), "{}", errors);
}

#[test]
fn warnings_alone_still_exit_zero() {
    let dir = sandbox("warn_only");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "severity": "warn", "files": ["nope.md"] } ] }"#,
    );

    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 0);
    assert!(stderr(&output).contains("warning:"), "{:?}", output);
}

#[test]
fn a_rule_that_is_off_produces_nothing() {
    let dir = sandbox("off");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "severity": "off", "files": ["nope.md"] } ] }"#,
    );

    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "");
}

#[test]
fn json_format_emits_one_document_on_stdout() {
    let dir = sandbox("json");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
    );

    let output = ruleman(&dir, &["--format", "json"]);
    assert_eq!(code(&output), 1);
    let document: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");
    assert_eq!(document["summary"]["errors"], 1);
    assert_eq!(document["summary"]["warnings"], 0);
    assert_eq!(document["diagnostics"][0]["severity"], "error");
    assert_eq!(document["diagnostics"][0]["rule"], "file");
    assert_eq!(document["diagnostics"][0]["file"], "README.md");

    // A config-level failure is still one parseable document, not a bare line.
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "nope" } ] }"#,
    );
    let output = ruleman(&dir, &["--format", "json"]);
    assert_eq!(code(&output), 1);
    let document: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");
    assert_eq!(document["diagnostics"][0]["rule"], "config");
    assert_eq!(document["diagnostics"][0]["file"], serde_json::Value::Null);
}

#[test]
fn github_format_emits_workflow_commands() {
    let dir = sandbox("github");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
    );

    let output = ruleman(&dir, &["--format", "github"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).starts_with("::error file=README.md::[ruleman] "),
        "{:?}",
        stderr(&output)
    );
}

#[test]
fn auto_format_follows_the_github_actions_environment() {
    let dir = sandbox("auto");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
    );

    let in_actions = Command::new(env!("CARGO_BIN_EXE_ruleman"))
        .current_dir(&dir)
        .env("GITHUB_ACTIONS", "true")
        .output()
        .unwrap();
    assert!(
        stderr(&in_actions).starts_with("::error"),
        "{:?}",
        in_actions
    );

    let outside = ruleman(&dir, &[]);
    assert!(stderr(&outside).starts_with("error: "), "{:?}", outside);
}

#[test]
fn a_missing_config_points_at_init() {
    let dir = sandbox("no_config");
    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("no config file found")
            && stderr(&output).contains("ruleman init"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_unknown_attribute_names_the_rule_index() {
    let dir = sandbox("unknown_attr");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [
             { "type": "file", "files": ["a"] },
             { "type": "file", "files": ["b"], "stat": "absent" }
           ] }"#,
    );

    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 1);
    let errors = stderr(&output);
    assert!(errors.contains("rules[1]"), "{}", errors);
    assert!(errors.contains("unknown field `stat`"), "{}", errors);
}

#[test]
fn init_scaffolds_a_config_and_refuses_to_clobber_it() {
    let dir = sandbox("init");

    let output = ruleman(&dir, &["init"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("created 'ruleman.json'"),
        "{:?}",
        output
    );
    let scaffold = fs::read_to_string(dir.join("ruleman.json")).unwrap();
    assert!(scaffold.contains("https://ruleman.dev/schema.json"));

    // The scaffold has to be valid input to the checker itself.
    write(&dir, "README.md", "");
    write(&dir, "LICENSE", "");
    assert_eq!(code(&ruleman(&dir, &[])), 0);

    fs::write(dir.join("ruleman.json"), "{}").unwrap();
    let output = ruleman(&dir, &["init"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("--force"), "{}", stderr(&output));
    assert_eq!(fs::read_to_string(dir.join("ruleman.json")).unwrap(), "{}");

    assert_eq!(code(&ruleman(&dir, &["init", "--force"])), 0);
    assert!(
        fs::read_to_string(dir.join("ruleman.json"))
            .unwrap()
            .contains("rules")
    );
}

#[test]
fn add_registers_a_path_and_preserves_the_config_formatting() {
    let dir = sandbox("add");
    write(
        &dir,
        "ruleman.json",
        "{\n  // keep me\n  \"rules\": [\n    { \"type\": \"file\", \"files\": [\"README.md\"] }\n  ]\n}\n",
    );
    write(&dir, "README.md", "");
    write(&dir, "LICENSE", "");
    fs::create_dir_all(dir.join("src")).unwrap();

    let output = ruleman(&dir, &["add", "LICENSE", "src"]);
    assert_eq!(code(&output), 0);
    let said = stdout(&output);
    assert!(said.contains("added file 'LICENSE'"), "{}", said);
    assert!(said.contains("added directory 'src'"), "{}", said);

    let config = fs::read_to_string(dir.join("ruleman.json")).unwrap();
    assert!(config.contains("// keep me"), "{}", config);
    assert!(config.contains(r#"["README.md", "LICENSE"]"#), "{}", config);
    assert!(config.contains(r#""type": "directory""#), "{}", config);

    // The result is a config the checker accepts.
    assert_eq!(code(&ruleman(&dir, &[])), 0);

    // Re-adding reports rather than duplicating.
    let output = ruleman(&dir, &["add", "LICENSE"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("already registered"),
        "{:?}",
        output
    );

    let output = ruleman(&dir, &["add", "nope.txt"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("not found"), "{}", stderr(&output));
}

#[test]
fn add_checksum_records_refreshes_and_detects_a_change() {
    let dir = sandbox("add_checksum");
    write(&dir, "ruleman.json", r#"{ "rules": [] }"#);
    write(&dir, "vendor/lib.js", "console.log(1)\n");

    let output = ruleman(&dir, &["add", "--checksum", "vendor/lib.js"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("recorded the sha256 checksum"),
        "{:?}",
        output
    );
    assert_eq!(code(&ruleman(&dir, &[])), 0);

    // Editing the file breaks the pin...
    write(&dir, "vendor/lib.js", "console.log(2)\n");
    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("does not match the recorded digest"),
        "{}",
        stderr(&output)
    );

    // ...and re-recording it is how the pin is refreshed.
    let output = ruleman(&dir, &["add", "--checksum", "vendor/lib.js"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("updated the sha256 checksum"),
        "{:?}",
        output
    );
    assert_eq!(code(&ruleman(&dir, &[])), 0);

    // Recording an unchanged file writes nothing.
    let before = fs::read_to_string(dir.join("ruleman.json")).unwrap();
    let output = ruleman(&dir, &["add", "--checksum", "vendor/lib.js"]);
    assert!(stdout(&output).contains("already matches"), "{:?}", output);
    assert_eq!(
        fs::read_to_string(dir.join("ruleman.json")).unwrap(),
        before
    );

    let output = ruleman(&dir, &["add", "--checksum", "vendor"]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("--checksum only applies to files"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn config_is_discovered_upward_and_overridable() {
    let dir = sandbox("discovery");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [ { "type": "file", "files": ["README.md"] } ] }"#,
    );
    write(&dir, "README.md", "");
    fs::create_dir_all(dir.join("deep/nested")).unwrap();
    write(
        &dir,
        "other.json",
        r#"{ "rules": [ { "type": "file", "files": ["absent.md"] } ] }"#,
    );

    // Found by walking up, and its paths still resolve against its own location.
    let output = ruleman(&dir.join("deep/nested"), &[]);
    assert_eq!(code(&output), 0, "{:?}", output);

    // --config wins over discovery.
    let output = ruleman(&dir, &["--config", "other.json"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("absent.md"), "{}", stderr(&output));
}

#[test]
fn every_content_format_is_readable() {
    let dir = sandbox("formats");
    write(&dir, "package.json", r#"{ "name": "@acme/web" }"#);
    write(
        &dir,
        "ci.yml",
        "jobs:\n  test:\n    runs-on: ubuntu-latest\n",
    );
    write(&dir, "Cargo.toml", "[package]\nedition = \"2024\"\n");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [
             { "type": "content", "file": "package.json", "key": "name", "expected": "@acme/web" },
             { "type": "content", "format": "yaml", "file": "ci.yml",
               "key": "jobs.test.runs-on", "expected": "ubuntu-latest" },
             { "type": "content", "format": "toml", "file": "Cargo.toml",
               "key": "package.edition", "expected": "2024" }
           ] }"#,
    );

    assert_eq!(code(&ruleman(&dir, &[])), 0);

    // A parse failure names the format it tried.
    write(&dir, "ci.yml", "jobs:\n- a\n  b: c\n");
    let output = ruleman(&dir, &[]);
    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("cannot parse 'ci.yml' as yaml"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn patterns_check_every_match() {
    let dir = sandbox("patterns");
    write(&dir, "packages/a/package.json", r#"{ "license": "MIT" }"#);
    write(
        &dir,
        "packages/b/package.json",
        r#"{ "license": "UNLICENSED" }"#,
    );
    write(&dir, "debug.log", "");
    write(&dir, ".gitignore", "ignored/\n");
    write(&dir, "ignored/skipped.log", "");
    write(
        &dir,
        "ruleman.json",
        r#"{ "rules": [
             { "type": "content", "file": "packages/*/package.json",
               "key": "license", "expected": "MIT" },
             { "type": "file", "state": "absent", "files": ["**/*.log"] }
           ] }"#,
    );

    let output = ruleman(&dir, &["--format", "json"]);
    assert_eq!(code(&output), 1);
    let document: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    let files: Vec<String> = document["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["file"].as_str().unwrap().to_string())
        .collect();
    // Only the offending package, and only the log that isn't gitignored.
    assert_eq!(files, vec!["packages/b/package.json", "debug.log"]);
}
