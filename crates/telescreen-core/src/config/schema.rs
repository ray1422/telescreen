//! Layer 2: JSON Schema. The schema file is the same file that the Python tools use.

use std::sync::OnceLock;

use serde_json::Value;

const SCHEMA: &str = include_str!("../../../../schema/config.schema.json");

fn validator() -> Result<&'static jsonschema::Validator, &'static str> {
    static VALIDATOR: OnceLock<Result<jsonschema::Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| {
            let schema: Value = serde_json::from_str(SCHEMA).map_err(|e| e.to_string())?;
            jsonschema::validator_for(&schema).map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(String::as_str)
}

/// Returns one message for each schema violation. An empty list means valid.
pub fn errors(value: &Value) -> Vec<String> {
    match validator() {
        Ok(v) => v
            .iter_errors(value)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect(),
        Err(e) => vec![format!("the built-in schema is not valid: {e}")],
    }
}
