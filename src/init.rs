//! The `init` subcommand: scaffolds a starter config file.

use crate::output::{OutputFormat, emit_error, emit_info};
use std::fs;
use std::path::Path;

pub(crate) const INIT_TEMPLATE: &str = r#"{
  "$schema": "https://ruleman.dev/schema.json",
  "rules": [
    {
      "type": "file",
      "severity": "error",
      "state": "present",
      "files": ["README.md", "LICENSE"]
    }
  ]
}
"#;

pub(crate) fn run(force: bool, format: OutputFormat) -> i32 {
    let path = Path::new("ruleman.json");
    if path.exists() && !force {
        emit_error(
            format,
            &format!(
                "'{}' already exists; pass --force to overwrite",
                path.display()
            ),
        );
        return 1;
    }

    match fs::write(path, INIT_TEMPLATE) {
        Ok(()) => {
            emit_info(format, &format!("created '{}'", path.display()));
            0
        }
        Err(e) => {
            emit_error(
                format,
                &format!("cannot create '{}': {}", path.display(), e),
            );
            1
        }
    }
}
