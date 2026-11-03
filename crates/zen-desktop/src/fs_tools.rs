use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, optional_str, required_str};
use crate::locations::{display_path, Location, Locations};
use crate::names::validate_name;

const MAX_CONTENT_BYTES: usize = 1_000_000;
// Archivos que Windows puede ejecutar o que abren otros programas al hacer doble click.
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "bat", "cmd", "com", "scr", "msi", "dll", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh",
    "hta", "lnk", "reg", "cpl", "jar", "url",
];

struct Target {
    location: Location,
    path: PathBuf,
}

fn resolve_target(locations: &Locations, args: &Value) -> Result<Target, ToolError> {
    let key = required_str(args, "location")?;
    let location = Location::from_key(key)
        .ok_or_else(|| invalid(format!("ubicación desconocida «{key}»; usa desktop, documents o downloads")))?;
    let name = required_str(args, "name")?;
    validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
    let dir = locations
        .resolve_dir(location, optional_str(args, "subfolder")?)
        .map_err(invalid)?;
    Ok(Target { location, path: dir.join(name) })
}

fn parameters(extra: Option<(&str, Value)>, name_description: &str) -> Value {
    let mut properties = json!({
        "location": {
            "type": "string",
            "enum": Location::KEYS,
            "description": "Carpeta del usuario donde actuar."
        },
        "subfolder": {
            "type": "string",
            "description": "Subcarpeta ya existente dentro de la ubicación (opcional), por ejemplo «Proyecto Zen»."
        },
        "name": { "type": "string", "description": name_description }
    });
    if let Some((key, schema)) = extra {
        properties[key] = schema;
    }
    json!({ "type": "object", "properties": properties, "required": ["location", "name"] })
}

fn io_error(path: &Path, what: &str, error: std::io::Error) -> ToolError {
    match error.kind() {
        ErrorKind::AlreadyExists => ToolError::Failed(format!(
            "Ya existe «{}». No lo sobrescribo.",
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        )),
        ErrorKind::PermissionDenied => ToolError::Failed(format!("Windows no permite {what} aquí (permisos).")),
        _ => ToolError::Failed(format!("No se pudo {what}: {error}")),
    }
}

pub struct CreateFolder {
    locations: Arc<Locations>,
}

impl CreateFolder {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }
}

impl Tool for CreateFolder {
    fn name(&self) -> &'static str {
        "create_folder"
    }

    fn description(&self) -> &'static str {
        "Crea una carpeta nueva en el Escritorio, Documentos o Descargas. No sobrescribe nada."
    }

    fn parameters(&self) -> Value {
        parameters(None, "Nombre de la carpeta (un solo nombre, sin rutas).")
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &["location", "subfolder", "name"])?;
        resolve_target(&self.locations, args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match resolve_target(&self.locations, args) {
            Ok(target) => format!("Crear la carpeta {}", display_path(&target.path)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let target = resolve_target(&self.locations, args)?;
        std::fs::create_dir(&target.path).map_err(|error| io_error(&target.path, "crear la carpeta", error))?;
        let path = display_path(&target.path);
        Ok(ToolOutput {
            summary: format!("Carpeta creada: {path}"),
            data: json!({ "path": path, "location": target.location.key() }),
        })
    }
}

pub struct CreateFile {
    locations: Arc<Locations>,
}

impl CreateFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }
}

fn extension_blocked(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| BLOCKED_EXTENSIONS.iter().any(|blocked| blocked.eq_ignore_ascii_case(ext)))
}

impl Tool for CreateFile {
    fn name(&self) -> &'static str {
        "create_file"
    }

    fn description(&self) -> &'static str {
        "Crea un archivo de texto nuevo, con contenido opcional. No sobrescribe archivos existentes \
         ni crea ejecutables ni scripts."
    }

    fn parameters(&self) -> Value {
        parameters(
            Some(("content", json!({ "type": "string", "description": "Contenido de texto del archivo (opcional)." }))),
            "Nombre del archivo con extensión, por ejemplo README.md.",
        )
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &["location", "subfolder", "name", "content"])?;
        resolve_target(&self.locations, args)?;
        let name = required_str(args, "name")?;
        if extension_blocked(name) {
            return Err(ToolError::NotAllowed(format!(
                "no creo archivos ejecutables ni scripts («{name}»)"
            )));
        }
        if optional_str(args, "content")?.is_some_and(|content| content.len() > MAX_CONTENT_BYTES) {
            return Err(invalid(format!("el contenido supera {MAX_CONTENT_BYTES} bytes")));
        }
        Ok(())
    }

    fn preview(&self, args: &Value) -> String {
        match resolve_target(&self.locations, args) {
            Ok(target) => {
                let bytes = optional_str(args, "content").ok().flatten().map_or(0, str::len);
                format!("Crear el archivo {} ({bytes} bytes)", display_path(&target.path))
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let target = resolve_target(&self.locations, args)?;
        let content = optional_str(args, "content")?.unwrap_or("");
        // `create_new` falla si el archivo existe: nunca se pisa nada.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target.path)
            .map_err(|error| io_error(&target.path, "crear el archivo", error))?;
        file.write_all(content.as_bytes())
            .map_err(|error| io_error(&target.path, "escribir el archivo", error))?;
        let path = display_path(&target.path);
        Ok(ToolOutput {
            summary: format!("Archivo creado: {path}"),
            data: json!({ "path": path, "location": target.location.key(), "bytes": content.len() }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn tools(t: &TempLocations) -> (CreateFolder, CreateFile) {
        let locations = Arc::new(t.locations.clone());
        (CreateFolder::new(locations.clone()), CreateFile::new(locations))
    }

    #[test]
    fn crea_una_carpeta_y_no_la_sobrescribe() {
        let t = temp_locations();
        let (folder, _) = tools(&t);
        let args = json!({ "location": "desktop", "name": "Proyecto Zen" });
        assert!(folder.validate(&args).is_ok());
        folder.execute(&args).unwrap();
        assert!(t.locations_root("desktop").join("Proyecto Zen").is_dir());
        assert!(matches!(folder.execute(&args), Err(ToolError::Failed(m)) if m.contains("Ya existe")));
    }

    #[test]
    fn crea_un_archivo_con_contenido_dentro_de_una_subcarpeta() {
        let t = temp_locations();
        let (folder, file) = tools(&t);
        folder.execute(&json!({ "location": "documents", "name": "Proyecto" })).unwrap();
        let args = json!({ "location": "documents", "subfolder": "Proyecto", "name": "README.md", "content": "# Hola" });
        file.validate(&args).unwrap();
        file.execute(&args).unwrap();
        let written = std::fs::read_to_string(t.locations_root("documents").join("Proyecto").join("README.md")).unwrap();
        assert_eq!(written, "# Hola");
    }

    #[test]
    fn un_archivo_existente_no_se_pisa() {
        let t = temp_locations();
        let (_, file) = tools(&t);
        let args = json!({ "location": "desktop", "name": "a.txt", "content": "uno" });
        file.execute(&args).unwrap();
        let again = json!({ "location": "desktop", "name": "a.txt", "content": "dos" });
        assert!(file.execute(&again).is_err());
        assert_eq!(std::fs::read_to_string(t.locations_root("desktop").join("a.txt")).unwrap(), "uno");
    }

    #[test]
    fn rechaza_rutas_fuera_de_las_ubicaciones() {
        let t = temp_locations();
        let (folder, file) = tools(&t);
        for name in ["..", "..\\x", "C:\\Windows\\x", "a/b"] {
            let args = json!({ "location": "desktop", "name": name });
            assert!(matches!(folder.validate(&args), Err(ToolError::InvalidArguments(_))), "{name}");
            assert!(file.validate(&args).is_err(), "{name}");
        }
        let escape = json!({ "location": "desktop", "subfolder": "..", "name": "x" });
        assert!(folder.validate(&escape).is_err());
        assert!(folder.validate(&json!({ "location": "system32", "name": "x" })).is_err());
    }

    #[test]
    fn bloquea_ejecutables_y_scripts() {
        let t = temp_locations();
        let (_, file) = tools(&t);
        for name in ["virus.exe", "a.BAT", "x.ps1", "enlace.lnk", "s.js"] {
            let args = json!({ "location": "desktop", "name": name });
            assert!(matches!(file.validate(&args), Err(ToolError::NotAllowed(_))), "{name}");
        }
        assert!(file.validate(&json!({ "location": "desktop", "name": "notas.txt" })).is_ok());
    }

    #[test]
    fn valida_tipos_argumentos_extra_y_tamano() {
        let t = temp_locations();
        let (folder, file) = tools(&t);
        assert!(folder.validate(&json!({ "location": "desktop" })).is_err());
        assert!(folder.validate(&json!({ "location": "desktop", "name": 5 })).is_err());
        assert!(folder.validate(&json!({ "location": "desktop", "name": "x", "extra": 1 })).is_err());
        assert!(folder.validate(&json!("texto")).is_err());
        let big = "x".repeat(MAX_CONTENT_BYTES + 1);
        assert!(file.validate(&json!({ "location": "desktop", "name": "a.txt", "content": big })).is_err());
    }
}