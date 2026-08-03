//! Parsing a structured file into the one shape the rule engine walks, so that
//! `content`'s dotted-key logic is written once regardless of the format.
//!
//! JSON is the shape everything converts to. That makes the mapping from YAML
//! and TOML part of the tool's contract rather than an implementation detail,
//! which is why each conversion below is explicit instead of delegated to a
//! serde bridge: a value's type decides whether `expected` matches it.

use crate::rule::ContentFormat;
use serde_json::{Map, Number, Value};
use yaml_rust2::{Yaml, YamlLoader};

pub(crate) fn parse(format: ContentFormat, raw: &str) -> Result<Value, String> {
    match format {
        ContentFormat::Json => serde_json::from_str(raw).map_err(|e| e.to_string()),
        ContentFormat::Yaml => parse_yaml(raw),
        ContentFormat::Toml => parse_toml(raw),
    }
}

fn parse_yaml(raw: &str) -> Result<Value, String> {
    let mut documents = YamlLoader::load_from_str(raw).map_err(|e| e.to_string())?;
    match documents.len() {
        // An empty file is an empty document, not an error; the rule then
        // reports that its key is not set.
        0 => Ok(Value::Null),
        1 => yaml_to_value(documents.remove(0)),
        // Reading only the first of several would silently check less than the
        // author meant, so say so instead.
        count => Err(format!(
            "the file holds {} YAML documents, and a rule reads one; \
             split them or point the rule at a single-document file",
            count
        )),
    }
}

fn yaml_to_value(yaml: Yaml) -> Result<Value, String> {
    Ok(match yaml {
        Yaml::Null => Value::Null,
        Yaml::Boolean(value) => Value::Bool(value),
        Yaml::Integer(value) => Value::Number(value.into()),
        // YAML floats are kept as text until needed. `.inf` and `.nan` have no
        // JSON equivalent, so they stay strings and can be compared as such.
        Yaml::Real(text) => match text.parse::<f64>().ok().and_then(Number::from_f64) {
            Some(number) => Value::Number(number),
            None => Value::String(text),
        },
        Yaml::String(text) => Value::String(text),
        Yaml::Array(items) => Value::Array(
            items
                .into_iter()
                .map(yaml_to_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Yaml::Hash(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(yaml_key(key)?, yaml_to_value(value)?);
            }
            Value::Object(object)
        }
        // Anchors are resolved while loading, so an alias reaching here means it
        // had no anchor to point at.
        Yaml::Alias(_) | Yaml::BadValue => {
            return Err("the file contains a YAML alias with no matching anchor".to_string());
        }
    })
}

/// Mapping keys become the strings a dotted key is written with. YAML allows
/// non-string scalars as keys, which are spelled the way they were written.
fn yaml_key(key: Yaml) -> Result<String, String> {
    match key {
        Yaml::String(text) | Yaml::Real(text) => Ok(text),
        Yaml::Integer(value) => Ok(value.to_string()),
        Yaml::Boolean(value) => Ok(value.to_string()),
        Yaml::Null => Ok("null".to_string()),
        // A sequence or mapping as a key can't be named by a dotted key at all.
        _ => Err(
            "the file uses a non-scalar YAML mapping key, which a dotted key cannot address"
                .to_string(),
        ),
    }
}

fn parse_toml(raw: &str) -> Result<Value, String> {
    // `str::parse` reads a single TOML *value*, not a document; `from_str` is
    // the one that reads a whole file.
    let parsed: toml::Value = toml::from_str(raw).map_err(|e| e.to_string())?;
    Ok(toml_to_value(parsed))
}

fn toml_to_value(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => Value::Number(number.into()),
        toml::Value::Float(number) => match Number::from_f64(number) {
            Some(number) => Value::Number(number),
            None => Value::String(number.to_string()),
        },
        toml::Value::Boolean(value) => Value::Bool(value),
        // TOML's date-times have no JSON counterpart. Deserializing them
        // generically yields an internal marker object, so they are spelled as
        // the string they were written with instead.
        toml::Value::Datetime(datetime) => Value::String(datetime.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_value).collect()),
        toml::Value::Table(entries) => Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key, toml_to_value(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn yaml_maps_onto_the_same_shape_as_json() {
        let value = parse(
            ContentFormat::Yaml,
            r#"
name: web
strict: true
retries: 3
ratio: 1.5
missing: ~
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
"#,
        )
        .unwrap();

        assert_eq!(value["name"], json!("web"));
        assert_eq!(value["strict"], json!(true));
        assert_eq!(value["retries"], json!(3));
        assert_eq!(value["ratio"], json!(1.5));
        assert_eq!(value["missing"], json!(null));
        assert_eq!(
            value["jobs"]["build"]["steps"][0]["uses"],
            json!("actions/checkout@v7")
        );
    }

    #[test]
    fn yaml_anchors_are_resolved() {
        let value = parse(
            ContentFormat::Yaml,
            "defaults: &d\n  node: 22\nbuild:\n  <<: *d\nrelease: *d\n",
        )
        .unwrap();
        assert_eq!(value["release"]["node"], json!(22));
    }

    #[test]
    fn yaml_non_string_keys_are_addressable_as_strings() {
        let value = parse(ContentFormat::Yaml, "1: one\ntrue: yes-key\n").unwrap();
        assert_eq!(value["1"], json!("one"));
        assert_eq!(value["true"], json!("yes-key"));
    }

    #[test]
    fn multi_document_yaml_is_rejected() {
        let error = parse(ContentFormat::Yaml, "a: 1\n---\nb: 2\n").unwrap_err();
        assert!(error.contains("2 YAML documents"), "{}", error);
        // A single document with a leading marker is still one document.
        assert!(parse(ContentFormat::Yaml, "---\na: 1\n").is_ok());
        // An empty file is an empty document.
        assert_eq!(parse(ContentFormat::Yaml, "").unwrap(), json!(null));
    }

    #[test]
    fn malformed_yaml_is_an_error() {
        assert!(parse(ContentFormat::Yaml, "a:\n- b\n  c: d\n").is_err());
    }

    #[test]
    fn toml_maps_onto_the_same_shape_as_json() {
        let value = parse(
            ContentFormat::Toml,
            r#"
name = "ruleman"
edition = 2024
[dependencies]
serde = { version = "1", features = ["derive"] }
[package.metadata]
released = 1979-05-27T07:32:00Z
"#,
        )
        .unwrap();

        assert_eq!(value["name"], json!("ruleman"));
        assert_eq!(value["edition"], json!(2024));
        assert_eq!(value["dependencies"]["serde"]["version"], json!("1"));
        assert_eq!(
            value["dependencies"]["serde"]["features"][0],
            json!("derive")
        );
        // Spelled as written rather than as an internal marker object.
        assert_eq!(
            value["package"]["metadata"]["released"],
            json!("1979-05-27T07:32:00Z")
        );
    }

    #[test]
    fn malformed_toml_is_an_error() {
        assert!(parse(ContentFormat::Toml, "a = ").is_err());
    }

    #[test]
    fn json_is_unchanged() {
        assert_eq!(
            parse(ContentFormat::Json, r#"{ "a": [1, true, null] }"#).unwrap(),
            json!({ "a": [1, true, null] })
        );
        assert!(parse(ContentFormat::Json, "{").is_err());
    }
}
