//! The `init` subcommand: scaffolds a starter config file.

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

pub(crate) fn run(force: bool) -> i32 {
    let path = Path::new("ruleman.json");
    if path.exists() && !force {
        eprintln!(
            "::error::[ruleman] '{}' は既に存在します。上書きするには --force を指定してください。",
            path.display()
        );
        return 1;
    }

    match fs::write(path, INIT_TEMPLATE) {
        Ok(()) => {
            println!("[ruleman] '{}' を作成しました。", path.display());
            0
        }
        Err(e) => {
            eprintln!(
                "::error::[ruleman] '{}' の作成に失敗しました: {}",
                path.display(),
                e
            );
            1
        }
    }
}
