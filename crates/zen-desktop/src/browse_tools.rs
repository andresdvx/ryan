use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, location_properties, optional_str, parse_location, required_str};
use crate::locations::{display_path, Locations};

const MAX_ENTRIES: usize = 200;
const SUMMARY_LINES: usize = 40;

pub struct ListFiles {
    locations: Arc<Locations>,
}

impl ListFiles {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn directory(&self, args: &Value) -> Result<std::path::PathBuf, ToolError> {
        check_keys(args, &["location", "subfolder"])?;
        let location = parse_location(required_str(args, "location")?)?;
        self.locations
            .resolve_dir(location, optional_str(args, "subfolder")?)
            .map_err(crate::args::invalid)
    }
}

struct Entry {
    name: String,
    is_dir: bool,
    size: Option<u64>,
}

impl Tool for ListFiles {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
        "Lista los archivos y carpetas de una ubicación (Escritorio, Documentos o Descargas) o de una de \
         sus subcarpetas. Solo lee: no modifica nada. Devuelve como máximo 200 elementos."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": location_properties("", "qué carpeta listar"), "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.directory(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.directory(args) {
            Ok(dir) => format!("Listar el contenido de {}", display_path(&dir)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let dir = self.directory(args)?;
        let read = std::fs::read_dir(&dir).map_err(|error| ToolError::Failed(format!("No se pudo leer la carpeta: {error}")))?;
        let mut entries: Vec<Entry> = read
            .flatten()
            .map(|entry| {
                let metadata = entry.metadata().ok();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                Entry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_dir,
                    size: metadata.filter(|_| !is_dir).map(|m| m.len()),
                }
            })
            .collect();
        // Carpetas primero, después por nombre sin distinguir mayúsculas.
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));

        let total = entries.len();
        let folders = entries.iter().filter(|e| e.is_dir).count();
        let truncated = total > MAX_ENTRIES;
        entries.truncate(MAX_ENTRIES);

        let mut summary = format!(
            "{} tiene {total} elementos ({folders} carpetas, {} archivos):",
            display_path(&dir),
            total - folders
        );
        for entry in entries.iter().take(SUMMARY_LINES) {
            summary.push_str(&format!("\n{} {}", if entry.is_dir { "📁" } else { "📄" }, entry.name));
        }
        if total > SUMMARY_LINES {
            summary.push_str(&format!("\n… y {} más", total - SUMMARY_LINES));
        }

        let listed: Vec<Value> = entries
            .iter()
            .map(|e| json!({ "name": e.name, "kind": if e.is_dir { "folder" } else { "file" }, "size_bytes": e.size }))
            .collect();
        Ok(ToolOutput {
            summary,
            data: json!({ "path": display_path(&dir), "total": total, "truncated": truncated, "entries": listed }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_locations;

    fn tool(t: &crate::testutil::TempLocations) -> ListFiles {
        ListFiles::new(Arc::new(t.locations.clone()))
    }

    #[test]
    fn lista_carpetas_primero_y_ordena_sin_distinguir_mayusculas() {
        let t = temp_locations();
        let root = t.locations_root("desktop");
        std::fs::create_dir(root.join("zeta")).unwrap();
        std::fs::create_dir(root.join("Alfa")).unwrap();
        std::fs::write(root.join("b.txt"), b"hola").unwrap();
        std::fs::write(root.join("A.txt"), b"").unwrap();
        let output = tool(&t).execute(&json!({ "location": "desktop" })).unwrap();
        let names: Vec<&str> = output.data["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Alfa", "zeta", "A.txt", "b.txt"]);
        assert_eq!(output.data["entries"][3]["size_bytes"], 4);
        assert_eq!(output.data["entries"][0]["size_bytes"], Value::Null);
        assert!(output.summary.contains("4 elementos (2 carpetas, 2 archivos)"));
        assert!(output.summary.contains("📁 Alfa") && output.summary.contains("📄 b.txt"));
    }

    #[test]
    fn lista_una_subcarpeta_y_rechaza_salirse() {
        let t = temp_locations();
        std::fs::create_dir(t.locations_root("documents").join("Proyecto")).unwrap();
        std::fs::write(t.locations_root("documents").join("Proyecto").join("a.md"), b"").unwrap();
        let tool = tool(&t);
        let output = tool.execute(&json!({ "location": "documents", "subfolder": "Proyecto" })).unwrap();
        assert_eq!(output.data["total"], 1);
        for bad in ["..", "../desktop", "C:\\Windows", "no_existe"] {
            assert!(tool.validate(&json!({ "location": "documents", "subfolder": bad })).is_err(), "{bad}");
        }
        assert!(tool.validate(&json!({ "location": "system" })).is_err());
        assert!(tool.validate(&json!({ "location": "desktop", "extra": 1 })).is_err());
    }

    #[test]
    fn corta_en_el_maximo_pero_informa_del_total() {
        let t = temp_locations();
        for i in 0..MAX_ENTRIES + 25 {
            std::fs::write(t.locations_root("downloads").join(format!("f{i:03}.txt")), b"").unwrap();
        }
        let output = tool(&t).execute(&json!({ "location": "downloads" })).unwrap();
        assert_eq!(output.data["total"], MAX_ENTRIES + 25);
        assert_eq!(output.data["truncated"], true);
        assert_eq!(output.data["entries"].as_array().unwrap().len(), MAX_ENTRIES);
        assert!(output.summary.contains(&format!("… y {} más", MAX_ENTRIES + 25 - SUMMARY_LINES)));
    }

    #[test]
    fn es_de_solo_lectura_y_segura() {
        let t = temp_locations();
        assert_eq!(tool(&t).risk(&json!({})), RiskLevel::Safe);
    }
}