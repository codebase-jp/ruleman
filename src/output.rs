//! Rendering rule failures. The check engine produces `Diagnostic`s and this
//! module decides how they reach the user, so no other module hard-codes a
//! particular CI vendor's annotation syntax.

use crate::rule::Severity;
use clap::ValueEnum;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[value(rename_all = "lowercase")]
pub(crate) enum OutputFormat {
    /// `github` when running inside GitHub Actions, `text` otherwise.
    #[default]
    Auto,
    /// GitHub Actions workflow commands, surfaced as annotations on the run.
    Github,
    /// One human-readable line per failure.
    Text,
    /// A single JSON document, for editors and other tooling.
    Json,
}

impl OutputFormat {
    /// Resolves `auto`. `GITHUB_ACTIONS` is set to `true` by every GitHub
    /// Actions runner, which is what the annotation syntax is for.
    fn resolve(self) -> OutputFormat {
        match self {
            OutputFormat::Auto => {
                if std::env::var_os("GITHUB_ACTIONS").is_some_and(|value| value == "true") {
                    OutputFormat::Github
                } else {
                    OutputFormat::Text
                }
            }
            resolved => resolved,
        }
    }
}

/// One rule failure, independent of how it is rendered.
pub(crate) struct Diagnostic {
    /// Never `Off`: a rule that is off produces no diagnostics at all.
    pub(crate) severity: Severity,
    /// The rule type that produced this, so JSON consumers can group by it.
    pub(crate) rule: &'static str,
    /// The path the failure is about, when there is one.
    pub(crate) file: Option<String>,
    pub(crate) message: String,
}

impl Diagnostic {
    pub(crate) fn new(
        severity: Severity,
        rule: &'static str,
        file: Option<String>,
        message: String,
    ) -> Self {
        Self {
            severity,
            rule,
            file,
            message,
        }
    }

    /// A problem with the config file itself rather than with a rule, reported
    /// through the same channel so `--format json` still emits one document.
    pub(crate) fn config(message: String) -> Self {
        Self::new(Severity::Error, "config", None, message)
    }

    fn label(&self) -> &'static str {
        match self.severity {
            Severity::Error => "error",
            _ => "warning",
        }
    }
}

/// Prints every diagnostic plus the run's outcome, and returns the exit code:
/// non-zero if any diagnostic is an error.
pub(crate) fn render(format: OutputFormat, diagnostics: &[Diagnostic]) -> i32 {
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = diagnostics.len() - errors;

    match format.resolve() {
        OutputFormat::Github => {
            for diagnostic in diagnostics {
                match &diagnostic.file {
                    Some(file) => eprintln!(
                        "::{} file={}::[ruleman] {}",
                        diagnostic.label(),
                        file,
                        diagnostic.message
                    ),
                    None => eprintln!("::{}::[ruleman] {}", diagnostic.label(), diagnostic.message),
                }
            }
            if errors == 0 {
                println!("[ruleman] All checks passed!");
            }
        }
        OutputFormat::Text => {
            for diagnostic in diagnostics {
                eprintln!("{}: {}", diagnostic.label(), diagnostic.message);
            }
            if diagnostics.is_empty() {
                println!("All checks passed!");
            } else {
                eprintln!("{}", summary(errors, warnings));
            }
        }
        OutputFormat::Json => {
            let document = json!({
                "diagnostics": diagnostics
                    .iter()
                    .map(|d| json!({
                        "severity": d.severity.as_str(),
                        "rule": d.rule,
                        "file": d.file,
                        "message": d.message,
                    }))
                    .collect::<Vec<_>>(),
                "summary": { "errors": errors, "warnings": warnings },
            });
            println!("{}", serde_json::to_string_pretty(&document).unwrap());
        }
        OutputFormat::Auto => unreachable!("resolve() never returns Auto"),
    }

    if errors == 0 { 0 } else { 1 }
}

fn summary(errors: usize, warnings: usize) -> String {
    let plural = |n: usize, word: &str| {
        if n == 1 {
            format!("{} {}", n, word)
        } else {
            format!("{} {}s", n, word)
        }
    };
    match (errors, warnings) {
        (0, w) => plural(w, "warning"),
        (e, 0) => plural(e, "error"),
        (e, w) => format!("{}, {}", plural(e, "error"), plural(w, "warning")),
    }
}

/// Reports a single fatal problem from a subcommand that doesn't run rules.
/// `json` renders like `text` here: `add` and `init` report actions taken, not
/// diagnostics, so there is no document shape for consumers to rely on.
pub(crate) fn emit_error(format: OutputFormat, message: &str) {
    match format.resolve() {
        OutputFormat::Github => eprintln!("::error::[ruleman] {}", message),
        _ => eprintln!("error: {}", message),
    }
}

/// Reports something a subcommand did, e.g. a path it registered.
pub(crate) fn emit_info(format: OutputFormat, message: &str) {
    match format.resolve() {
        OutputFormat::Github => println!("[ruleman] {}", message),
        _ => println!("{}", message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic::new(severity, "file", Some("a.txt".to_string()), "boom".into())
    }

    #[test]
    fn exit_code_is_driven_by_errors_only() {
        assert_eq!(render(OutputFormat::Json, &[]), 0);
        assert_eq!(render(OutputFormat::Json, &[diagnostic(Severity::Warn)]), 0);
        assert_eq!(
            render(OutputFormat::Json, &[diagnostic(Severity::Error)]),
            1
        );
    }

    #[test]
    fn summary_pluralizes() {
        assert_eq!(summary(1, 0), "1 error");
        assert_eq!(summary(2, 0), "2 errors");
        assert_eq!(summary(0, 1), "1 warning");
        assert_eq!(summary(2, 3), "2 errors, 3 warnings");
    }

    #[test]
    fn auto_resolves_to_text_outside_github_actions() {
        // The test process is not a GitHub Actions runner unless CI says so,
        // so only assert the branch that holds either way.
        assert_ne!(OutputFormat::Auto.resolve(), OutputFormat::Auto);
        assert_eq!(OutputFormat::Text.resolve(), OutputFormat::Text);
        assert_eq!(OutputFormat::Github.resolve(), OutputFormat::Github);
    }
}
