use clap::Parser;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Parser, Debug)]
#[command(name = "molda")]
struct Args {
    #[arg(long, default_value = "molda.json")]
    config: String,
}

#[derive(Debug, Deserialize)]
struct Config {
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Rule {
    #[serde(rename = "file-existence")]
    FileExistence { files: Vec<String> },
    #[serde(rename = "json-match")]
    JsonMatch {
        file: String,
        key: String,
        expected: Value,
    },
}

fn load_config(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        return Err(format!(
            "::error::[molda] 設定ファイル '{}' が見つかりません。",
            path.display()
        ));
    }

    let raw = fs::read_to_string(path).map_err(|e| {
        format!(
            "::error::[molda] 設定ファイル '{}' の読み込みに失敗しました: {}",
            path.display(),
            e
        )
    })?;

    serde_json::from_str::<Config>(&raw).map_err(|e| {
        format!(
            "::error::[molda] 設定ファイル '{}' のJSON解析に失敗しました: {}",
            path.display(),
            e
        )
    })
}

fn get_value_by_dotted_key<'a>(root: &'a Value, dotted_key: &str) -> Option<&'a Value> {
    dotted_key
        .split('.')
        .try_fold(root, |current, segment| current.get(segment))
}

fn json_key_matches(root: &Value, key: &str, expected: &Value) -> bool {
    get_value_by_dotted_key(root, key).is_some_and(|actual| actual == expected)
}

fn run(config_path: &str) -> i32 {
    let config = match load_config(Path::new(config_path)) {
        Ok(cfg) => cfg,
        Err(message) => {
            eprintln!("{}", message);
            return 1;
        }
    };

    let mut has_errors = false;

    for rule in config.rules {
        match rule {
            Rule::FileExistence { files } => {
                for file in files {
                    if !Path::new(&file).exists() {
                        eprintln!(
                            "::error::[molda] 必須ファイル '{}' が見つかりません。",
                            file
                        );
                        has_errors = true;
                    }
                }
            }
            Rule::JsonMatch {
                file,
                key,
                expected,
            } => {
                let path = Path::new(&file);
                if !path.exists() {
                    eprintln!(
                        "::error file={}::[molda] ルール不適合: {} の検証に失敗しました。",
                        file, key
                    );
                    has_errors = true;
                    continue;
                }

                let raw = match fs::read_to_string(path) {
                    Ok(content) => content,
                    Err(_) => {
                        eprintln!(
                            "::error file={}::[molda] ルール不適合: {} の検証に失敗しました。",
                            file, key
                        );
                        has_errors = true;
                        continue;
                    }
                };

                let json = match serde_json::from_str::<Value>(&raw) {
                    Ok(value) => value,
                    Err(_) => {
                        eprintln!(
                            "::error file={}::[molda] ルール不適合: {} の検証に失敗しました。",
                            file, key
                        );
                        has_errors = true;
                        continue;
                    }
                };

                if !json_key_matches(&json, &key, &expected) {
                    eprintln!(
                        "::error file={}::[molda] ルール不適合: {} の検証に失敗しました。",
                        file, key
                    );
                    has_errors = true;
                }
            }
        }
    }

    if has_errors {
        1
    } else {
        println!("[molda] すべての標準チェックに合格しました！");
        0
    }
}

fn main() {
    let args = Args::parse();
    std::process::exit(run(&args.config));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dotted_key_can_find_nested_value() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        let found = get_value_by_dotted_key(&value, "compilerOptions.strict");
        assert_eq!(found, Some(&json!(true)));
    }

    #[test]
    fn dotted_key_returns_none_when_missing() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        let found = get_value_by_dotted_key(&value, "compilerOptions.noImplicitAny");
        assert!(found.is_none());
    }

    #[test]
    fn json_key_matches_requires_exact_value() {
        let value = json!({
            "compilerOptions": {
                "strict": true
            }
        });

        assert!(json_key_matches(
            &value,
            "compilerOptions.strict",
            &json!(true)
        ));
        assert!(!json_key_matches(
            &value,
            "compilerOptions.strict",
            &json!(false)
        ));
    }
}
