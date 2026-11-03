//! Organizar una carpeta moviendo sus archivos sueltos a subcarpetas por tipo, con registro para deshacer.

use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, location_properties, optional_str, parse_location, required_str};
use crate::locations::{display_path, Locations};

const MAX_FILES: usize = 300;
const PREVIEW_LINES: usize = 40;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Shortcuts,
    Images,
    Documents,
    Videos,
    Audio,
    Archives,
    Other,
}

impl Category {
    const ALL: [Category; 7] = [
        Category::Shortcuts,
        Category::Images,
        Category::Documents,
        Category::Videos,
        Category::Audio,
        Category::Archives,
        Category::Other,
    ];

    fn key(self) -> &'static str {
        match self {
            Category::Shortcuts => "shortcuts",
            Category::Images => "images",
            Category::Documents => "documents",
            Category::Videos => "videos",
            Category::Audio => "audio",
            Category::Archives => "archives",
            Category::Other => "other",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Category::Shortcuts => "Accesos directos",
            Category::Images => "Imágenes",
            Category::Documents => "Documentos",
            Category::Videos => "Videos",
            Category::Audio => "Música",
            Category::Archives => "Comprimidos",
            Category::Other => "Otros",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.key() == key)
    }

    fn of(file_name: &str) -> Self {
        let extension = Path::new(file_name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match extension.as_str() {
            "lnk" | "url" => Category::Shortcuts,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "heic" | "tif" | "tiff" => Category::Images,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf" | "odt" | "ods" | "csv" => {
                Category::Documents
            }
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" => Category::Videos,
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => Category::Audio,
            "zip" | "rar" | "7z" | "tar" | "gz" => Category::Archives,
            _ => Category::Other,
        }
    }
}

struct Item {
    name: String,
    category: Category,
}

/// Archivos sueltos de la carpeta (no carpetas, ni ocultos o del sistema) de los tipos pedidos.
fn scan(dir: &Path, kinds: &[Category]) -> Result<Vec<Item>, ToolError> {
    let read = fs::read_dir(dir).map_err(|error| ToolError::Failed(format!("No se pudo leer la carpeta: {error}")))?;
    let mut items = Vec::new();
    for entry in read.flatten() {
        // `is_file` es falso para carpetas y para enlaces simbólicos: no se tocan.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
        if name.starts_with('.') || metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
            continue;
        }
        let category = Category::of(&name);
        if kinds.contains(&category) {
            items.push(Item { name, category });
        }
    }
    items.sort_by_key(|item| item.name.to_lowercase());
    if items.is_empty() {
        return Err(ToolError::Failed("No hay archivos sueltos de esos tipos que organizar.".into()));
    }
    if items.len() > MAX_FILES {
        return Err(ToolError::Failed(format!(
            "Hay {} archivos sueltos: son demasiados para organizarlos de una vez (máximo {MAX_FILES}). Prueba por tipos.",
            items.len()
        )));
    }
    Ok(items)
}

/// Registro de lo último que se organizó, para poder deshacerlo.
#[derive(Clone)]
pub struct Journal {
    path: PathBuf,
}

struct JournalData {
    moves: Vec<(PathBuf, PathBuf)>,
    created_dirs: Vec<PathBuf>,
}

impl Journal {
    pub fn new(dir: PathBuf) -> Self {
        Self { path: dir.join("last-organize.json") }
    }

    fn save(&self, data: &JournalData) -> Result<(), ToolError> {
        let text = |path: &Path| path.to_str().map(str::to_string);
        let moves: Option<Vec<Value>> = data
            .moves
            .iter()
            .map(|(from, to)| Some(json!({ "from": text(from)?, "to": text(to)? })))
            .collect();
        let created: Option<Vec<String>> = data.created_dirs.iter().map(|dir| text(dir)).collect();
        let (Some(moves), Some(created)) = (moves, created) else {
            return Err(ToolError::Failed("No se pudo registrar lo organizado (nombres no válidos).".into()));
        };
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = serde_json::to_string_pretty(&json!({ "moves": moves, "created_dirs": created })).unwrap_or_default();
        fs::write(&self.path, body)
            .map_err(|error| ToolError::Failed(format!("No se pudo guardar el registro para deshacer: {error}")))
    }

    fn load(&self) -> Option<JournalData> {
        let value: Value = serde_json::from_str(&fs::read_to_string(&self.path).ok()?).ok()?;
        let path_of = |value: &Value| value.as_str().map(PathBuf::from);
        let moves = value["moves"]
            .as_array()?
            .iter()
            .map(|entry| Some((path_of(&entry["from"])?, path_of(&entry["to"])?)))
            .collect::<Option<Vec<_>>>()?;
        let created_dirs = value["created_dirs"].as_array()?.iter().map(path_of).collect::<Option<Vec<_>>>()?;
        Some(JournalData { moves, created_dirs })
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub struct OrganizeFolder {
    locations: Arc<Locations>,
    journal: Journal,
}

impl OrganizeFolder {
    pub fn new(locations: Arc<Locations>, journal: Journal) -> Self {
        Self { locations, journal }
    }

    fn target(&self, args: &Value) -> Result<(PathBuf, Vec<Item>), ToolError> {
        check_keys(args, &["location", "subfolder", "kinds"])?;
        let location = parse_location(required_str(args, "location")?)?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let kinds = match args.get("kinds") {
            None | Some(Value::Null) => Category::ALL.to_vec(),
            Some(Value::Array(list)) if !list.is_empty() => list
                .iter()
                .map(|kind| {
                    kind.as_str().and_then(Category::from_key).ok_or_else(|| {
                        invalid(format!("tipo desconocido {kind}; usa: {}", Category::ALL.map(Category::key).join(", ")))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(invalid("«kinds» debe ser una lista no vacía de tipos")),
        };
        let items = scan(&dir, &kinds)?;
        Ok((dir, items))
    }
}

/// Crea la carpeta de categoría si falta; devuelve un error legible si no se puede usar.
fn ensure_folder(folder: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    if folder.is_dir() {
        return Ok(());
    }
    if folder.symlink_metadata().is_ok() {
        return Err("hay un archivo con el nombre de la carpeta de destino".into());
    }
    fs::create_dir(folder).map_err(|error| format!("no se pudo crear la carpeta de destino: {error}"))?;
    created.push(folder.to_path_buf());
    Ok(())
}

fn reason(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => "Windows no lo permite (¿está abierto en otro programa?)".into(),
        _ => error.to_string(),
    }
}

impl Tool for OrganizeFolder {
    fn name(&self) -> &'static str {
        "organize_folder"
    }

    fn description(&self) -> &'static str {
        "Organiza una carpeta (por ejemplo el Escritorio o Descargas): mueve sus archivos sueltos a subcarpetas \
         por tipo (Accesos directos, Imágenes, Documentos, Videos, Música, Comprimidos, Otros). No toca \
         carpetas ni archivos ocultos, ni sobrescribe nada. Con «kinds» se limita a algunos tipos. Pide \
         confirmación mostrando todo lo que va a mover y se puede deshacer con undo_organize."
    }

    fn parameters(&self) -> Value {
        let mut properties = location_properties("", "qué carpeta organizar").as_object().cloned().unwrap_or_default();
        properties.insert(
            "kinds".into(),
            json!({
                "type": "array",
                "items": { "type": "string", "enum": Category::ALL.map(Category::key) },
                "description": "Tipos a organizar (opcional; por defecto todos). Por ejemplo [\"shortcuts\"] para solo los accesos directos."
            }),
        );
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.target(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        let (dir, items) = match self.target(args) {
            Ok(target) => target,
            Err(error) => return error.to_string(),
        };
        let mut text =
            format!("Organizar {}: se moverán {} archivos a subcarpetas por tipo.\n", display_path(&dir), items.len());
        let counts: Vec<String> = Category::ALL
            .iter()
            .filter_map(|&category| {
                let count = items.iter().filter(|item| item.category == category).count();
                (count > 0).then(|| format!("{} ({count})", category.folder()))
            })
            .collect();
        text.push_str(&counts.join(", "));
        text.push_str("\n\nDetalle:");
        for item in items.iter().take(PREVIEW_LINES) {
            let conflict = dir.join(item.category.folder()).join(&item.name).symlink_metadata().is_ok();
            let note = if conflict { " (se omitirá: ya existe)" } else { "" };
            text.push_str(&format!("\n• {} → {}{note}", item.name, item.category.folder()));
        }
        if items.len() > PREVIEW_LINES {
            text.push_str(&format!("\n… y {} más", items.len() - PREVIEW_LINES));
        }
        text.push_str("\n\nPodrás deshacerlo después.");
        text
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (dir, items) = self.target(args)?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut created: Vec<PathBuf> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();

        for item in &items {
            let folder = dir.join(item.category.folder());
            if let Err(why) = ensure_folder(&folder, &mut created) {
                skipped.push((item.name.clone(), why));
                continue;
            }
            let (from, to) = (dir.join(&item.name), folder.join(&item.name));
            if to.symlink_metadata().is_ok() {
                skipped.push((item.name.clone(), "ya existe en la carpeta de destino".into()));
                continue;
            }
            match fs::rename(&from, &to) {
                Ok(()) => moved.push((from, to)),
                Err(error) => skipped.push((item.name.clone(), reason(&error))),
            }
        }

        if !moved.is_empty() {
            self.journal.save(&JournalData { moves: moved.clone(), created_dirs: created.clone() })?;
        }

        let mut summary = format!("Organicé {}: moví {} archivos", display_path(&dir), moved.len());
        if !skipped.is_empty() {
            summary.push_str(&format!(" y omití {}", skipped.len()));
            for (name, why) in skipped.iter().take(10) {
                summary.push_str(&format!("\n• {name}: {why}"));
            }
        }
        if !moved.is_empty() {
            summary.push_str("\nSe puede deshacer con «undo_organize».");
        }
        let moved_json: Vec<Value> =
            moved.iter().map(|(from, to)| json!({ "from": display_path(from), "to": display_path(to) })).collect();
        let skipped_json: Vec<Value> =
            skipped.iter().map(|(name, why)| json!({ "name": name, "reason": why })).collect();
        Ok(ToolOutput {
            summary,
            data: json!({ "moved": moved_json, "skipped": skipped_json, "undo_available": !moved.is_empty() }),
        })
    }
}

pub struct UndoOrganize {
    locations: Arc<Locations>,
    journal: Journal,
}

impl UndoOrganize {
    pub fn new(locations: Arc<Locations>, journal: Journal) -> Self {
        Self { locations, journal }
    }

    /// El registro es un archivo en disco: antes de usarlo se comprueba que todo lo que menciona
    /// sigue dentro de las carpetas permitidas, para que no sirva de atajo a otros movimientos.
    fn load(&self) -> Result<JournalData, ToolError> {
        let data = self
            .journal
            .load()
            .ok_or_else(|| ToolError::Failed("No hay ninguna organización reciente que deshacer.".into()))?;
        let all_inside = data.moves.iter().all(|(from, to)| self.locations.contains(from) && self.locations.contains(to))
            && data.created_dirs.iter().all(|dir| self.locations.contains(dir));
        if all_inside {
            Ok(data)
        } else {
            Err(ToolError::NotAllowed(
                "El registro de la última organización apunta fuera de tus carpetas; no lo uso.".into(),
            ))
        }
    }
}

impl Tool for UndoOrganize {
    fn name(&self) -> &'static str {
        "undo_organize"
    }

    fn description(&self) -> &'static str {
        "Deshace la última organización hecha con organize_folder: devuelve los archivos a su sitio original \
         y quita las subcarpetas que se crearon si quedaron vacías. Pide confirmación."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &[])?;
        self.load().map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        if let Err(error) = check_keys(args, &[]) {
            return error.to_string();
        }
        match self.load() {
            Ok(data) => {
                let restorable =
                    data.moves.iter().filter(|(from, to)| to.exists() && from.symlink_metadata().is_err()).count();
                format!(
                    "Deshacer la última organización: devolver {restorable} de {} archivos a su sitio original.",
                    data.moves.len()
                )
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        check_keys(args, &[])?;
        let data = self.load()?;
        let (mut restored, mut skipped) = (0, Vec::new());
        for (from, to) in data.moves.iter().rev() {
            let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !to.exists() {
                skipped.push(format!("{name}: ya no está donde lo dejé"));
            } else if from.symlink_metadata().is_ok() {
                skipped.push(format!("{name}: su sitio original está ocupado"));
            } else {
                match fs::rename(to, from) {
                    Ok(()) => restored += 1,
                    Err(error) => skipped.push(format!("{name}: {}", reason(&error))),
                }
            }
        }
        // Solo desaparecen las carpetas que se crearon y que ahora están vacías.
        for dir in &data.created_dirs {
            let _ = fs::remove_dir(dir);
        }
        self.journal.clear();

        let mut summary = format!("Deshice la organización: devolví {restored} archivos a su sitio");
        if !skipped.is_empty() {
            summary.push_str(&format!(" y omití {}", skipped.len()));
            for line in skipped.iter().take(10) {
                summary.push_str(&format!("\n• {line}"));
            }
        }
        Ok(ToolOutput { summary, data: json!({ "restored": restored, "skipped": skipped }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn tools(t: &TempLocations) -> (OrganizeFolder, UndoOrganize) {
        let locations = Arc::new(t.locations.clone());
        let journal = Journal::new(t.locations_root("data"));
        (OrganizeFolder::new(locations.clone(), journal.clone()), UndoOrganize::new(locations, journal))
    }

    fn touch(t: &TempLocations, name: &str) {
        std::fs::write(t.locations_root("desktop").join(name), b"x").unwrap();
    }

    fn names(path: PathBuf) -> Vec<String> {
        let mut list: Vec<String> =
            std::fs::read_dir(path).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        list.sort();
        list
    }

    #[test]
    fn clasifica_por_extension_sin_distinguir_mayusculas() {
        for (name, category) in [
            ("a.LNK", Category::Shortcuts),
            ("juego.url", Category::Shortcuts),
            ("foto.JPG", Category::Images),
            ("tesis.docx", Category::Documents),
            ("peli.mkv", Category::Videos),
            ("cancion.flac", Category::Audio),
            ("todo.7z", Category::Archives),
            ("setup.exe", Category::Other),
            ("sin_extension", Category::Other),
        ] {
            assert_eq!(Category::of(name), category, "{name}");
        }
        for category in Category::ALL {
            assert_eq!(Category::from_key(category.key()), Some(category));
        }
    }

    #[test]
    fn organiza_por_tipo_y_deja_intactas_las_carpetas() {
        let t = temp_locations();
        for name in ["Discord.lnk", "foto.png", "notas.txt", "setup.exe"] {
            touch(&t, name);
        }
        std::fs::create_dir(t.locations_root("desktop").join("MiCarpeta")).unwrap();
        let (organize, _) = tools(&t);
        let output = organize.execute(&json!({ "location": "desktop" })).unwrap();
        assert_eq!(output.data["moved"].as_array().unwrap().len(), 4);
        let root = t.locations_root("desktop");
        assert_eq!(names(root.clone()), ["Accesos directos", "Documentos", "Imágenes", "MiCarpeta", "Otros"]);
        assert_eq!(names(root.join("Accesos directos")), ["Discord.lnk"]);
        assert_eq!(names(root.join("Otros")), ["setup.exe"]);
        assert!(output.summary.contains("moví 4 archivos") && output.summary.contains("undo_organize"));
    }

    #[test]
    fn con_kinds_solo_toca_esos_tipos() {
        let t = temp_locations();
        for name in ["a.lnk", "b.png", "c.txt"] {
            touch(&t, name);
        }
        let (organize, _) = tools(&t);
        organize.execute(&json!({ "location": "desktop", "kinds": ["shortcuts"] })).unwrap();
        assert_eq!(names(t.locations_root("desktop")), ["Accesos directos", "b.png", "c.txt"]);
        assert!(organize.validate(&json!({ "location": "desktop", "kinds": ["fotos"] })).is_err());
        assert!(organize.validate(&json!({ "location": "desktop", "kinds": [] })).is_err());
        assert!(organize.validate(&json!({ "location": "desktop", "kinds": "images" })).is_err());
    }

    #[test]
    fn omite_ocultos_de_sistema_y_lo_que_ya_existe() {
        let t = temp_locations();
        let root = t.locations_root("desktop");
        for name in ["visible.txt", ".oculto.txt", "desktop.ini", "atributo.txt", "repetido.txt"] {
            touch(&t, name);
        }
        let attribute = |name: &str, flag: &str| {
            let status = std::process::Command::new("attrib").arg(flag).arg(root.join(name)).status().unwrap();
            assert!(status.success());
        };
        attribute("atributo.txt", "+h");
        attribute("desktop.ini", "+s");
        std::fs::create_dir(root.join("Documentos")).unwrap();
        std::fs::write(root.join("Documentos").join("repetido.txt"), b"original").unwrap();
        let (organize, _) = tools(&t);
        let output = organize.execute(&json!({ "location": "desktop" })).unwrap();
        assert_eq!(output.data["moved"].as_array().unwrap().len(), 1, "solo visible.txt");
        assert_eq!(output.data["skipped"][0]["name"], "repetido.txt");
        assert_eq!(std::fs::read_to_string(root.join("Documentos").join("repetido.txt")).unwrap(), "original");
        for kept in [".oculto.txt", "desktop.ini", "atributo.txt", "repetido.txt"] {
            assert!(root.join(kept).exists(), "{kept} no debía tocarse");
        }
    }

    #[test]
    fn la_vista_previa_lista_todo_lo_que_se_movera() {
        let t = temp_locations();
        for i in 0..PREVIEW_LINES + 5 {
            touch(&t, &format!("doc{i:02}.txt"));
        }
        touch(&t, "a.lnk");
        let (organize, _) = tools(&t);
        let preview = organize.preview(&json!({ "location": "desktop" }));
        assert!(preview.contains(&format!("se moverán {} archivos", PREVIEW_LINES + 6)), "{preview}");
        assert!(preview.contains("Documentos (45)") && preview.contains("Accesos directos (1)"));
        assert!(preview.contains("• a.lnk → Accesos directos") && preview.contains("… y 6 más"));
        assert_eq!(organize.risk(&json!({})), RiskLevel::Confirm);
    }

    #[test]
    fn rechaza_si_no_hay_nada_o_hay_demasiado() {
        let t = temp_locations();
        let (organize, _) = tools(&t);
        assert!(matches!(organize.validate(&json!({ "location": "desktop" })), Err(ToolError::Failed(m)) if m.contains("No hay archivos")));
        for i in 0..MAX_FILES + 1 {
            touch(&t, &format!("f{i:03}.txt"));
        }
        assert!(matches!(organize.validate(&json!({ "location": "desktop" })), Err(ToolError::Failed(m)) if m.contains("demasiados")));
    }

    #[test]
    fn organiza_una_subcarpeta_y_no_se_sale() {
        let t = temp_locations();
        std::fs::create_dir(t.locations_root("downloads").join("Viejos")).unwrap();
        std::fs::write(t.locations_root("downloads").join("Viejos").join("a.zip"), b"x").unwrap();
        let (organize, _) = tools(&t);
        organize.execute(&json!({ "location": "downloads", "subfolder": "Viejos" })).unwrap();
        assert!(t.locations_root("downloads").join("Viejos").join("Comprimidos").join("a.zip").exists());
        assert!(organize.validate(&json!({ "location": "downloads", "subfolder": ".." })).is_err());
    }

    #[test]
    fn deshacer_devuelve_todo_y_quita_las_carpetas_que_creo() {
        let t = temp_locations();
        for name in ["a.lnk", "b.png", "c.txt"] {
            touch(&t, name);
        }
        std::fs::create_dir(t.locations_root("desktop").join("Documentos")).unwrap();
        let (organize, undo) = tools(&t);
        organize.execute(&json!({ "location": "desktop" })).unwrap();
        assert!(undo.preview(&json!({})).contains("devolver 3 de 3"));
        let output = undo.execute(&json!({})).unwrap();
        assert_eq!(output.data["restored"], 3);
        let root = t.locations_root("desktop");
        assert_eq!(
            names(root),
            ["Documentos", "a.lnk", "b.png", "c.txt"],
            "Documentos ya existía y se conserva; las demás se quitan"
        );
        assert!(undo.validate(&json!({})).is_err(), "el registro se consume");
    }

    #[test]
    fn deshacer_no_pisa_lo_que_cambio_despues() {
        let t = temp_locations();
        touch(&t, "a.txt");
        touch(&t, "b.txt");
        let (organize, undo) = tools(&t);
        organize.execute(&json!({ "location": "desktop" })).unwrap();
        let root = t.locations_root("desktop");
        std::fs::remove_file(root.join("Documentos").join("a.txt")).unwrap();
        std::fs::write(root.join("b.txt"), b"nuevo").unwrap();
        let output = undo.execute(&json!({})).unwrap();
        assert_eq!(output.data["restored"], 0);
        assert_eq!(output.data["skipped"].as_array().unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "nuevo");
        assert!(root.join("Documentos").join("b.txt").exists(), "lo organizado que sigue ahí no se pierde");
    }

    #[test]
    fn un_registro_manipulado_que_apunta_fuera_no_se_usa() {
        let t = temp_locations();
        let outside = t.locations_root("data").join("secreto.txt");
        std::fs::write(&outside, b"x").unwrap();
        let journal = Journal::new(t.locations_root("data"));
        journal
            .save(&JournalData { moves: vec![(t.locations_root("desktop").join("a.txt"), outside)], created_dirs: vec![] })
            .unwrap();
        let (_, undo) = tools(&t);
        assert!(matches!(undo.validate(&json!({})), Err(ToolError::NotAllowed(_))));
        assert!(undo.execute(&json!({})).is_err());
    }

    #[test]
    fn sin_registro_no_hay_nada_que_deshacer() {
        let t = temp_locations();
        let (_, undo) = tools(&t);
        assert!(matches!(undo.validate(&json!({})), Err(ToolError::Failed(_))));
        assert!(undo.validate(&json!({ "algo": 1 })).is_err());
    }

    /// Recorrido completo sobre el Escritorio real (con OneDrive), en una subcarpeta propia que se
    /// borra al terminar: `cargo test -p zen-desktop -- --ignored prueba_real`.
    #[test]
    #[ignore = "usa el Escritorio real"]
    fn prueba_real_en_el_escritorio() {
        use crate::browse_tools::ListFiles;
        use crate::move_tools::MoveFile;

        let locations = Arc::new(Locations::from_system().expect("carpetas del usuario"));
        let sandbox_name = format!("ZenPet-prueba-{}", std::process::id());
        let desktop = locations.resolve_dir(crate::Location::Desktop, None).unwrap();
        let desktop_before = names(desktop.clone());
        let sandbox = desktop.join(&sandbox_name);
        fs::create_dir(&sandbox).unwrap();
        let data = std::env::temp_dir().join(&sandbox_name);
        let journal = Journal::new(data.clone());
        let organize = OrganizeFolder::new(locations.clone(), journal.clone());
        let undo = UndoOrganize::new(locations.clone(), journal);
        let list = ListFiles::new(locations.clone());
        let mover = MoveFile::new(locations);
        let place = json!({ "location": "desktop", "subfolder": sandbox_name });

        let outcome = std::panic::catch_unwind(|| {
            for name in ["Atajo.lnk", "foto.png", "notas.txt", "setup.exe"] {
                fs::write(sandbox.join(name), b"x").unwrap();
            }
            let before = list.execute(&place).unwrap();
            assert_eq!(before.data["total"], 4);

            let organized = organize.execute(&place).unwrap();
            assert_eq!(organized.data["moved"].as_array().unwrap().len(), 4, "{}", organized.summary);
            assert!(sandbox.join("Accesos directos").join("Atajo.lnk").exists());

            let rename = json!({ "location": "desktop", "subfolder": sandbox_name, "name": "Otros", "new_name": "Varios" });
            mover.validate(&rename).unwrap();
            mover.execute(&rename).unwrap();
            assert!(sandbox.join("Varios").join("setup.exe").exists());
            let deshacer = undo.execute(&json!({})).unwrap();
            assert_eq!(deshacer.data["restored"], 3, "setup.exe quedó en una carpeta renombrada: {}", deshacer.summary);
        });
        let _ = fs::remove_dir_all(&sandbox);
        let _ = fs::remove_dir_all(&data);
        outcome.unwrap();
        assert!(!sandbox.exists(), "la carpeta de prueba debe desaparecer");
        assert_eq!(names(desktop), desktop_before, "nada debe haberse salido de la subcarpeta de prueba");
    }
}
