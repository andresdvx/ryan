//! Las API keys viven en el Administrador de credenciales de Windows, nunca en un archivo.
//! Hay una por proveedor; la variable de entorno del proveedor sirve de alternativa en desarrollo.

use keyring::Entry;
use zen_ai::ProviderKind;

const SERVICE: &str = "zen-pet";

fn entry(kind: ProviderKind) -> keyring::Result<Entry> {
    Entry::new(SERVICE, &format!("{}-api-key", kind.id()))
}

fn clean(key: String) -> Option<String> {
    let key = key.trim().to_string();
    (!key.is_empty()).then_some(key)
}

fn stored_key(kind: ProviderKind) -> Option<String> {
    entry(kind).ok().and_then(|entry| entry.get_password().ok()).and_then(clean)
}

fn env_key(kind: ProviderKind) -> Option<String> {
    std::env::var(kind.env_var()).ok().and_then(clean)
}

pub fn api_key(kind: ProviderKind) -> Option<String> {
    stored_key(kind).or_else(|| env_key(kind))
}

/// De dónde sale la clave en uso, sin revelarla: "stored", "env" o "none".
pub fn key_source(kind: ProviderKind) -> &'static str {
    if stored_key(kind).is_some() {
        "stored"
    } else if env_key(kind).is_some() {
        "env"
    } else {
        "none"
    }
}

/// Comprueba el formato antes de guardar (no que la clave sea válida para el proveedor).
pub fn validate_key(key: &str) -> Result<&str, String> {
    let key = key.trim();
    if key.is_empty() {
        Err("La clave está vacía.".into())
    } else if key.len() > 512 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        Err("La clave tiene un formato inválido: no puede contener espacios ni saltos de línea.".into())
    } else {
        Ok(key)
    }
}

pub fn save_key(kind: ProviderKind, key: &str) -> Result<(), String> {
    let key = validate_key(key)?;
    entry(kind)
        .and_then(|entry| entry.set_password(key))
        .map_err(|error| format!("No se pudo guardar en el Administrador de credenciales: {error}"))
}

pub fn forget_key(kind: ProviderKind) -> Result<(), String> {
    match entry(kind).and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sin el backend nativo, `keyring` usa un almacén falso en memoria y la clave se perdería
    /// al cerrar la app: se comprueba que lo escrito por una `Entry` lo lee otra distinta.
    #[test]
    fn el_administrador_de_credenciales_de_windows_persiste_la_clave() {
        let writer = Entry::new("zen-pet-test", "roundtrip").unwrap();
        writer.set_password("valor-de-prueba").unwrap();
        let reader = Entry::new("zen-pet-test", "roundtrip").unwrap();
        assert_eq!(reader.get_password().unwrap(), "valor-de-prueba");
        writer.delete_credential().unwrap();
        assert!(matches!(reader.get_password(), Err(keyring::Error::NoEntry)));
    }

    #[test]
    fn valida_el_formato_de_la_clave() {
        assert_eq!(validate_key("  AIzaSyEjemplo123  "), Ok("AIzaSyEjemplo123"));
        for bad in ["", "   ", "dos palabras", "con\nsalto", "con\ttab"] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
        assert!(validate_key(&"x".repeat(513)).is_err());
    }

    #[test]
    fn cada_proveedor_usa_su_propia_entrada() {
        let names: Vec<String> = ProviderKind::ALL.iter().map(|k| format!("{}-api-key", k.id())).collect();
        assert_eq!(names, ["gemini-api-key", "claude-api-key", "openai-api-key", "ollama-api-key"]);
    }
}