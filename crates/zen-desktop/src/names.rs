const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];
pub const MAX_NAME_LEN: usize = 100;

/// Valida un único nombre de archivo o carpeta (nunca una ruta).
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("está vacío".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("supera {MAX_NAME_LEN} caracteres"));
    }
    if name.trim() != name {
        return Err("no puede empezar ni terminar con espacios".into());
    }
    if name.ends_with('.') {
        return Err("no puede terminar en punto".into());
    }
    if name.chars().any(|c| c.is_control() || FORBIDDEN_CHARS.contains(&c)) {
        return Err("contiene caracteres no permitidos (< > : \" / \\ | ? *)".into());
    }
    let stem = name.split('.').next().unwrap_or("").trim_end();
    if RESERVED.iter().any(|reserved| reserved.eq_ignore_ascii_case(stem)) {
        return Err("es un nombre reservado de Windows".into());
    }
    Ok(())
}

const SENSITIVE_EXACT: &[&str] = &[
    "id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", ".npmrc", ".netrc", ".pgpass", "known_hosts", "authorized_keys",
    "wallet.dat", "credentials", "credentials.json",
];
const SENSITIVE_EXTENSIONS: &[&str] = &["pem", "key", "pfx", "p12", "ppk", "kdbx", "kdb", "keystore", "jks", "gpg"];
const SENSITIVE_WORDS: &[&str] = &[
    "password", "passwd", "contraseña", "contrasena", "secret", "secreto", "credential", "credencial", "apikey",
    "api_key", "api-key", "private_key", "privatekey", "clave_privada",
];

/// ¿Parece, por su nombre, un archivo con contraseñas o claves? Se prefiere equivocarse por exceso:
/// estos archivos nunca se leen ni se buscan por dentro, así que su contenido no puede llegar a la IA.
pub fn is_sensitive_file_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    let extension = std::path::Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("");
    lower.starts_with(".env")
        || SENSITIVE_EXACT.contains(&lower.as_str())
        || SENSITIVE_EXTENSIONS.contains(&extension)
        || SENSITIVE_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_archivos_con_pinta_de_secretos() {
        for name in [
            ".env", ".env.local", ".ENV.production", "id_rsa", "id_ed25519", "clave.pem", "cert.PFX", "vault.kdbx", "mi.key",
            "passwords.txt", "Contraseñas.docx", "mis_contrasenas.xlsx", "secrets.json", "client_secret_123.json", "credentials.json",
            "api_key.txt", "private_key.txt", ".npmrc", "known_hosts",
        ] {
            assert!(is_sensitive_file_name(name), "{name}");
        }
        for name in ["notas.txt", "README.md", "informe 2026.docx", "main.rs", "environment.md", "keyboard.txt", "turkey.txt", "lista.csv"] {
            assert!(!is_sensitive_file_name(name), "{name} no debía marcarse");
        }
    }

    #[test]
    fn acepta_nombres_normales() {
        for name in ["Proyecto Zen", "README.md", ".gitignore", "informe 2026.txt", "ñandú"] {
            assert_eq!(validate_name(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn rechaza_rutas_y_trucos() {
        for name in [
            "", ".", "..", "a/b", "a\\b", "..\\x", "C:", "C:\\Windows", "a:b", "x?", "con", "NUL.txt",
            "com1", "LPT9.log", "nombre.", " espacio", "espacio ", "con .txt", "a\u{0}b", "a\nb",
        ] {
            assert!(validate_name(name).is_err(), "debía rechazar {name:?}");
        }
        assert!(validate_name(&"x".repeat(MAX_NAME_LEN + 1)).is_err());
    }
}