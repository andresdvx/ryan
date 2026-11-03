use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, location_properties, optional_str, parse_location, required_str};
use crate::locations::{display_path, Locations};
use crate::names::validate_name;

struct MovePlan {
    from: PathBuf,
    to: PathBuf,
}

pub struct MoveFile {
    locations: Arc<Locations>,
}

impl MoveFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    /// Comprueba todo lo comprobable sin tocar nada. El destino nunca se sobrescribe.
    fn plan(&self, args: &Value) -> Result<MovePlan, ToolError> {
        check_keys(args, &["location", "subfolder", "name", "to_location", "to_subfolder", "new_name"])?;
        let source_location = parse_location(required_str(args, "location")?)?;
        let name = required_str(args, "name")?;
        validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
        let source_dir = self
            .locations
            .resolve_dir(source_location, optional_str(args, "subfolder")?)
            .map_err(invalid)?;

        // `canonicalize` comprueba que existe y resuelve enlaces: un enlace simbólico que apunte
        // fuera de la carpeta no se puede mover como si estuviera dentro.
        let from = source_dir
            .join(name)
            .canonicalize()
            .map_err(|_| ToolError::Failed(format!("No existe «{name}» en {}.", display_path(&source_dir))))?;
        if from.parent() != Some(source_dir.as_path()) {
            return Err(ToolError::NotAllowed(format!("«{name}» es un enlace que apunta fuera de la carpeta.")));
        }

        // Sin destino explícito el elemento se queda en la misma carpeta (renombrar). Con `to_subfolder`
        // sin `to_location`, la subcarpeta es de la misma ubicación; con `to_location` solo, su raíz.
        let target_location = optional_str(args, "to_location")?;
        let target_subfolder = optional_str(args, "to_subfolder")?;
        let target_dir = if target_location.is_none() && target_subfolder.is_none() {
            source_dir.clone()
        } else {
            let location = match target_location {
                Some(key) => parse_location(key)?,
                None => source_location,
            };
            self.locations.resolve_dir(location, target_subfolder).map_err(invalid)?
        };
        let new_name = match optional_str(args, "new_name")? {
            Some(new_name) => {
                validate_name(new_name).map_err(|reason| invalid(format!("nuevo nombre inválido: {reason}")))?;
                new_name.to_string()
            }
            None => from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        let to = target_dir.join(&new_name);

        if from.is_dir() && target_dir.starts_with(&from) {
            return Err(ToolError::NotAllowed("No puedo mover una carpeta dentro de sí misma.".into()));
        }
        if from == to {
            return Err(ToolError::Failed("El origen y el destino son iguales.".into()));
        }
        // Windows no distingue mayúsculas: cambiar solo la capitalización es un renombrado válido.
        let same_file = from.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy());
        if !same_file && to.symlink_metadata().is_ok() {
            return Err(ToolError::Failed(format!(
                "Ya existe «{new_name}» en {}. No lo sobrescribo.",
                display_path(&target_dir)
            )));
        }
        Ok(MovePlan { from, to })
    }
}

impl Tool for MoveFile {
    fn name(&self) -> &'static str {
        "move_file"
    }

    fn description(&self) -> &'static str {
        "Mueve o renombra un archivo o carpeta dentro del Escritorio, Documentos y Descargas. Indica el \
         origen (location, subfolder, name). Para renombrar indica solo new_name: se queda en su misma \
         carpeta. Para mover indica to_location y/o to_subfolder (con to_location solo, va a la raíz de \
         esa ubicación). Pide confirmación al usuario y nunca sobrescribe. No sirve para borrar: para \
         eso usa delete_file."
    }

    fn parameters(&self) -> Value {
        let mut properties = location_properties("", "dónde está el elemento").as_object().cloned().unwrap_or_default();
        properties.extend(
            location_properties("to_", "destino; si no se indica, la misma ubicación")
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        properties.insert(
            "name".into(),
            json!({ "type": "string", "description": "Nombre actual del archivo o carpeta (un solo nombre, sin rutas)." }),
        );
        properties.insert(
            "new_name".into(),
            json!({ "type": "string", "description": "Nuevo nombre (opcional, para renombrar)." }),
        );
        json!({ "type": "object", "properties": properties, "required": ["location", "name"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.plan(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.plan(args) {
            Ok(plan) => format!("Mover:\n{}\n→\n{}", display_path(&plan.from), display_path(&plan.to)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let plan = self.plan(args)?;
        std::fs::rename(&plan.from, &plan.to).map_err(|error| match error.kind() {
            ErrorKind::CrossesDevices => ToolError::Failed("No puedo mover entre unidades distintas.".into()),
            ErrorKind::PermissionDenied => {
                ToolError::Failed("Windows no permite moverlo (¿está abierto en otro programa?).".into())
            }
            _ => ToolError::Failed(format!("No se pudo mover: {error}")),
        })?;
        let (from, to) = (display_path(&plan.from), display_path(&plan.to));
        Ok(ToolOutput { summary: format!("Moví {from}\n→ {to}"), data: json!({ "from": from, "to": to }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn tool(t: &TempLocations) -> MoveFile {
        MoveFile::new(Arc::new(t.locations.clone()))
    }

    fn write(t: &TempLocations, place: &str, name: &str, text: &str) {
        std::fs::write(t.locations_root(place).join(name), text).unwrap();
    }

    #[test]
    fn mueve_un_archivo_a_otra_ubicacion_y_a_una_subcarpeta() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", "contenido");
        std::fs::create_dir(t.locations_root("documents").join("Proyecto")).unwrap();
        let args = json!({ "location": "desktop", "name": "a.txt", "to_location": "documents", "to_subfolder": "Proyecto" });
        tool(&t).validate(&args).unwrap();
        tool(&t).execute(&args).unwrap();
        assert!(!t.locations_root("desktop").join("a.txt").exists());
        let moved = t.locations_root("documents").join("Proyecto").join("a.txt");
        assert_eq!(std::fs::read_to_string(moved).unwrap(), "contenido");
    }

    #[test]
    fn renombra_y_tambien_solo_cambia_mayusculas() {
        let t = temp_locations();
        write(&t, "desktop", "viejo.txt", "x");
        tool(&t).execute(&json!({ "location": "desktop", "name": "viejo.txt", "new_name": "nuevo.txt" })).unwrap();
        assert!(t.locations_root("desktop").join("nuevo.txt").exists());
        tool(&t).execute(&json!({ "location": "desktop", "name": "nuevo.txt", "new_name": "NUEVO.txt" })).unwrap();
        let names: Vec<String> = std::fs::read_dir(t.locations_root("desktop"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["NUEVO.txt"]);
    }

    #[test]
    fn renombrar_dentro_de_una_subcarpeta_no_saca_el_elemento_de_ella() {
        // Fallo real: con solo `new_name`, el elemento se iba a la raíz de la ubicación.
        let t = temp_locations();
        let sub = t.locations_root("desktop").join("Sub");
        std::fs::create_dir_all(sub.join("Carpeta")).unwrap();
        std::fs::write(sub.join("a.txt"), "x").unwrap();
        let tool = tool(&t);
        tool.execute(&json!({ "location": "desktop", "subfolder": "Sub", "name": "Carpeta", "new_name": "Nueva" })).unwrap();
        tool.execute(&json!({ "location": "desktop", "subfolder": "Sub", "name": "a.txt", "new_name": "b.txt" })).unwrap();
        let mut inside: Vec<String> =
            std::fs::read_dir(&sub).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        inside.sort();
        assert_eq!(inside, ["Nueva", "b.txt"]);
        assert_eq!(std::fs::read_dir(t.locations_root("desktop")).unwrap().count(), 1, "en la raíz solo queda Sub");
    }

    #[test]
    fn el_destino_se_interpreta_por_partes() {
        let t = temp_locations();
        std::fs::create_dir_all(t.locations_root("desktop").join("Sub").join("Hondo")).unwrap();
        std::fs::create_dir(t.locations_root("desktop").join("Destino")).unwrap();
        write(&t, "desktop", "Sub\\a.txt", "");
        let tool = tool(&t);
        // Solo to_subfolder: subcarpeta de la misma ubicación (desde una subcarpeta de origen).
        tool.execute(&json!({ "location": "desktop", "subfolder": "Sub", "name": "a.txt", "to_subfolder": "Destino" })).unwrap();
        assert!(t.locations_root("desktop").join("Destino").join("a.txt").exists());
        // Solo to_location: la raíz de esa ubicación.
        tool.execute(&json!({ "location": "desktop", "subfolder": "Destino", "name": "a.txt", "to_location": "downloads" })).unwrap();
        assert!(t.locations_root("downloads").join("a.txt").exists());
    }

    #[test]
    fn mueve_carpetas_con_su_contenido() {
        let t = temp_locations();
        std::fs::create_dir(t.locations_root("desktop").join("Fotos")).unwrap();
        std::fs::write(t.locations_root("desktop").join("Fotos").join("1.jpg"), "img").unwrap();
        tool(&t).execute(&json!({ "location": "desktop", "name": "Fotos", "to_location": "downloads" })).unwrap();
        assert!(t.locations_root("downloads").join("Fotos").join("1.jpg").exists());
    }

    #[test]
    fn nunca_sobrescribe_nada() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", "origen");
        write(&t, "documents", "a.txt", "destino");
        let args = json!({ "location": "desktop", "name": "a.txt", "to_location": "documents" });
        assert!(matches!(tool(&t).validate(&args), Err(ToolError::Failed(m)) if m.contains("No lo sobrescribo")));
        assert!(tool(&t).execute(&args).is_err());
        assert_eq!(std::fs::read_to_string(t.locations_root("documents").join("a.txt")).unwrap(), "destino");
        assert_eq!(std::fs::read_to_string(t.locations_root("desktop").join("a.txt")).unwrap(), "origen");
    }

    #[test]
    fn rechaza_salirse_de_las_ubicaciones_o_usar_rutas() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", "x");
        let tool = tool(&t);
        let attempts = [
            json!({ "location": "desktop", "name": "..\\..\\x" }),
            json!({ "location": "desktop", "name": "a.txt", "to_subfolder": ".." }),
            json!({ "location": "desktop", "name": "a.txt", "new_name": "..\\evil.txt" }),
            json!({ "location": "desktop", "name": "a.txt", "new_name": "C:\\Windows\\a.txt" }),
            json!({ "location": "desktop", "name": "a.txt", "to_location": "system" }),
            json!({ "location": "desktop", "subfolder": "../documents", "name": "a.txt" }),
            json!({ "location": "desktop", "name": "no_existe.txt" }),
            json!({ "location": "desktop", "name": "a.txt", "extra": 1 }),
        ];
        for args in attempts {
            assert!(tool.validate(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn no_mueve_una_carpeta_dentro_de_si_misma_ni_a_su_mismo_sitio() {
        let t = temp_locations();
        std::fs::create_dir_all(t.locations_root("desktop").join("A").join("B")).unwrap();
        write(&t, "desktop", "x.txt", "");
        let tool = tool(&t);
        let inside = json!({ "location": "desktop", "name": "A", "to_location": "desktop", "to_subfolder": "A/B" });
        assert!(matches!(tool.validate(&inside), Err(ToolError::NotAllowed(_))));
        assert!(tool.validate(&json!({ "location": "desktop", "name": "x.txt" })).is_err(), "mismo sitio");
    }

    #[test]
    fn pide_confirmacion_y_la_vista_previa_dice_de_donde_a_donde() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", "");
        let tool = tool(&t);
        assert_eq!(tool.risk(&json!({})), RiskLevel::Confirm);
        let preview = tool.preview(&json!({ "location": "desktop", "name": "a.txt", "to_location": "downloads" }));
        assert!(preview.contains("a.txt") && preview.contains('→') && preview.contains("downloads"), "{preview}");
    }
}
