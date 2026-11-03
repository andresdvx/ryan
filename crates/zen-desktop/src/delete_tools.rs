//! Borrar archivos y carpetas enviándolos a la Papelera de Windows, nunca de forma permanente.
//! Se usa la API del Shell (no una carpeta llamada «Papelera»), así que funciona igual con Windows en cualquier idioma.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, location_properties, optional_str, parse_location, required_str};
use crate::locations::{display_path, Locations};
use crate::names::validate_name;

/// Para la vista previa basta saber que «hay mucho»: no se recorre una carpeta enorme entera.
const COUNT_LIMIT: usize = 5000;

pub struct DeleteFile {
    locations: Arc<Locations>,
}

impl DeleteFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    /// Comprueba todo lo comprobable sin tocar nada. Solo se puede borrar algo que esté DENTRO de
    /// una ubicación permitida: las ubicaciones en sí (Escritorio, Documentos, Descargas) nunca.
    fn plan(&self, args: &Value) -> Result<PathBuf, ToolError> {
        check_keys(args, &["location", "subfolder", "name"])?;
        let location = parse_location(required_str(args, "location")?)?;
        let name = required_str(args, "name")?;
        validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let target = dir
            .join(name)
            .canonicalize()
            .map_err(|_| ToolError::Failed(format!("No existe «{name}» en {}.", display_path(&dir))))?;
        // Un enlace que apunte fuera de la carpeta no se borra como si estuviera dentro.
        if target.parent() != Some(dir.as_path()) {
            return Err(ToolError::NotAllowed(format!("«{name}» es un enlace que apunta fuera de la carpeta.")));
        }
        Ok(target)
    }
}

/// Elementos que contiene una carpeta (recursivo, sin seguir enlaces), hasta `COUNT_LIMIT`.
fn count_items(dir: &Path, counted: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *counted >= COUNT_LIMIT {
            return;
        }
        *counted += 1;
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            count_items(&entry.path(), counted);
        }
    }
}

fn describe(target: &Path) -> String {
    if !target.is_dir() {
        return "un archivo".into();
    }
    let mut count = 0;
    count_items(target, &mut count);
    match count {
        0 => "una carpeta vacía".into(),
        1 => "una carpeta con 1 elemento".into(),
        n if n >= COUNT_LIMIT => format!("una carpeta con más de {COUNT_LIMIT} elementos"),
        n => format!("una carpeta con {n} elementos"),
    }
}

impl Tool for DeleteFile {
    fn name(&self) -> &'static str {
        "delete_file"
    }

    fn description(&self) -> &'static str {
        "Borra un archivo o carpeta del Escritorio, Documentos o Descargas enviándolo a la Papelera de \
         Windows, de donde se puede recuperar. Indica location, subfolder (opcional) y name. Úsala \
         siempre que el usuario pida borrar o eliminar algo; no intentes mover nada a una carpeta \
         «Papelera». Nunca borra de forma permanente y pide confirmación detallada al usuario."
    }

    fn parameters(&self) -> Value {
        let mut properties = location_properties("", "dónde está el elemento").as_object().cloned().unwrap_or_default();
        properties.insert(
            "name".into(),
            json!({ "type": "string", "description": "Nombre del archivo o carpeta a borrar (un solo nombre, sin rutas)." }),
        );
        json!({ "type": "object", "properties": properties, "required": ["location", "name"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Dangerous
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.plan(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.plan(args) {
            Ok(target) => format!(
                "Enviar a la Papelera de Windows ({}):\n{}\nPodrás recuperarlo desde la Papelera.",
                describe(&target),
                display_path(&target)
            ),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let target = self.plan(args)?;
        let shown = display_path(&target);
        let kind = describe(&target);
        recycle(&target).map_err(ToolError::Failed)?;
        // Si Windows no pudo reciclarlo no se borra: se comprueba que de verdad ya no está.
        if target.symlink_metadata().is_ok() {
            return Err(ToolError::Failed(
                "Windows no pudo enviarlo a la Papelera (¿es demasiado grande o está en uso?). No lo borré de forma permanente.".into(),
            ));
        }
        Ok(ToolOutput {
            summary: format!("Envié a la Papelera ({kind}):\n{shown}\nPuedes restaurarlo desde la Papelera de Windows."),
            data: json!({ "path": shown, "recycled": true }),
        })
    }
}

/// `IFileOperation` con `FOFX_RECYCLEONDELETE`: si el elemento no se puede reciclar, la operación
/// falla en vez de borrarlo para siempre (que es lo que haría `SHFileOperation` con `FOF_ALLOWUNDO`).
fn recycle(path: &Path) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FILEOPERATION_FLAGS, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO,
        FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
    };

    let fail = |error: windows::core::Error| format!("No pude enviarlo a la Papelera: {error}");
    unsafe {
        // Ok también si el hilo ya tenía COM iniciado (S_FALSE); solo se cierra lo que se abrió aquí.
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = (|| {
            let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(fail)?;
            let flags = FOF_ALLOWUNDO.0 | FOF_NOCONFIRMATION.0 | FOF_NOERRORUI.0 | FOF_SILENT.0 | FOFX_RECYCLEONDELETE.0;
            operation.SetOperationFlags(FILEOPERATION_FLAGS(flags)).map_err(fail)?;
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(display_path(path)), None).map_err(fail)?;
            operation.DeleteItem(&item, None).map_err(fail)?;
            operation.PerformOperations().map_err(fail)
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn tool(t: &TempLocations) -> DeleteFile {
        DeleteFile::new(Arc::new(t.locations.clone()))
    }

    #[test]
    fn es_peligrosa_y_dice_que_va_a_la_papelera() {
        let t = temp_locations();
        std::fs::create_dir(t.locations_root("desktop").join("Viejo")).unwrap();
        std::fs::write(t.locations_root("desktop").join("Viejo").join("a.txt"), "x").unwrap();
        let args = json!({ "location": "desktop", "name": "Viejo" });
        let tool = tool(&t);
        assert_eq!(tool.risk(&args), RiskLevel::Dangerous);
        tool.validate(&args).unwrap();
        let preview = tool.preview(&args);
        assert!(preview.contains("Papelera") && preview.contains("1 elemento") && preview.contains("Viejo"), "{preview}");
        assert!(t.locations_root("desktop").join("Viejo").exists(), "la vista previa no toca nada");
    }

    #[test]
    fn rechaza_lo_que_no_existe_las_rutas_y_los_argumentos_raros() {
        let t = temp_locations();
        let tool = tool(&t);
        for args in [
            json!({ "location": "desktop", "name": "no_existe" }),
            json!({ "location": "desktop", "name": ".." }),
            json!({ "location": "desktop", "name": "a\\b" }),
            json!({ "location": "desktop", "name": "" }),
            json!({ "location": "escritorio", "name": "x" }),
            json!({ "location": "desktop", "name": "x", "permanent": true }),
            json!({ "location": "desktop" }),
        ] {
            assert!(tool.validate(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn cuenta_los_elementos_de_la_vista_previa() {
        let t = temp_locations();
        let base = t.locations_root("desktop").join("Carpeta");
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("sub").join("1.txt"), "").unwrap();
        std::fs::write(base.join("2.txt"), "").unwrap();
        assert_eq!(describe(&base), "una carpeta con 3 elementos");
        assert_eq!(describe(&base.join("2.txt")), "un archivo");
        let empty = t.locations_root("desktop").join("Vacia");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(describe(&empty), "una carpeta vacía");
    }

    /// Manda de verdad a la Papelera un archivo y una carpeta temporales (quedan en la Papelera: se vacían a mano).
    /// Se ejecuta con `cargo test -p zen-desktop -- --ignored --nocapture papelera_real`.
    #[test]
    #[ignore = "usa la Papelera real de Windows"]
    fn papelera_real_recicla_un_archivo_y_una_carpeta() {
        let t = temp_locations();
        let desktop = t.locations_root("desktop");
        std::fs::write(desktop.join("zen-prueba-borrar.txt"), "x").unwrap();
        std::fs::create_dir(desktop.join("zen-prueba-carpeta")).unwrap();
        std::fs::write(desktop.join("zen-prueba-carpeta").join("a.txt"), "x").unwrap();
        let tool = tool(&t);
        for name in ["zen-prueba-borrar.txt", "zen-prueba-carpeta"] {
            let output = tool.execute(&json!({ "location": "desktop", "name": name })).unwrap();
            println!("{}", output.summary);
            assert!(!desktop.join(name).exists(), "{name} sigue en su sitio");
        }
    }
}
