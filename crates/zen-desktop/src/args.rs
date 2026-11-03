use serde_json::Value;
use zen_core::tools::ToolError;

use crate::locations::Location;

pub fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::InvalidArguments(message.into())
}

/// Rechaza argumentos que el esquema no declara.
pub fn check_keys(args: &Value, allowed: &[&str]) -> Result<(), ToolError> {
    let object = args.as_object().ok_or_else(|| invalid("los argumentos deben ser un objeto"))?;
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(invalid(format!("argumento desconocido «{key}»"))),
        None => Ok(()),
    }
}

pub fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(invalid(format!("«{key}» debe ser texto"))),
        None => Err(invalid(format!("falta «{key}»"))),
    }
}

pub fn optional_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(invalid(format!("«{key}» debe ser texto"))),
    }
}
/// Entero opcional dentro de un rango; `None` si no se indicó.
pub fn optional_u64(args: &Value, key: &str, min: u64, max: u64) -> Result<Option<u64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => match value.as_u64() {
            Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
            _ => Err(invalid(format!("«{key}» debe ser un entero entre {min} y {max}"))),
        },
    }
}

pub fn parse_location(key: &str) -> Result<Location, ToolError> {
    Location::from_key(key)
        .ok_or_else(|| invalid(format!("ubicación desconocida «{key}»; usa desktop, documents o downloads")))
}

/// Propiedades JSON Schema comunes: ubicación y subcarpeta opcional.
pub fn location_properties(prefix: &str, what: &str) -> serde_json::Value {
    serde_json::json!({
        format!("{prefix}location"): { "type": "string", "enum": Location::KEYS, "description": format!("Carpeta del usuario: {what}.") },
        format!("{prefix}subfolder"): { "type": "string", "description": format!("Subcarpeta ya existente dentro de la ubicación (opcional): {what}.") },
    })
}