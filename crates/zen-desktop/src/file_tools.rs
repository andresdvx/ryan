//! Copiar, abrir, comprimir y descomprimir. Como el resto de herramientas de archivos, solo actúan
//! dentro del Escritorio, Documentos y Descargas, aceptan nombres sueltos (nunca rutas), no siguen
//! enlaces que salgan de ahí y jamás sobrescriben nada.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::args::{check_keys, invalid, location_properties, optional_str, parse_location, required_str};
use crate::locations::{display_path, Locations};
use crate::move_tools::{MoveFile, MovePlan};
use crate::names::validate_name;
use crate::read_tools::has_cloud_only_flag;
use crate::walk::human_size;

const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 40;
const ZIP_ENTRY_NAME_DEPTH: usize = 30;

// ---------------------------------------------------------------------------------------------
// Medidas de un árbol de archivos
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    files: usize,
    bytes: u64,
    /// Archivos de OneDrive que solo están en la nube: no se copian ni se comprimen (habría que descargarlos).
    cloud_only: usize,
    too_big: bool,
}

fn measure(path: &Path, stats: &mut Stats, depth: usize) {
    if stats.too_big {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else { return };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_file() {
        if has_cloud_only_flag(metadata.file_attributes()) {
            stats.cloud_only += 1;
            return;
        }
        stats.files += 1;
        stats.bytes += metadata.len();
        stats.too_big = stats.files > MAX_FILES || stats.bytes > MAX_BYTES;
    } else if metadata.is_dir() {
        if depth >= MAX_DEPTH {
            stats.too_big = true;
            return;
        }
        if let Ok(read) = fs::read_dir(path) {
            for entry in read.flatten() {
                measure(&entry.path(), stats, depth + 1);
            }
        }
    }
}

fn tree_stats(path: &Path) -> Stats {
    let mut stats = Stats::default();
    measure(path, &mut stats, 0);
    stats
}

fn describe_tree(path: &Path, stats: &Stats) -> String {
    let cloud = if stats.cloud_only > 0 { format!("; {} solo en la nube, no se incluyen", stats.cloud_only) } else { String::new() };
    if path.is_dir() {
        format!("una carpeta con {} archivos, {}{cloud}", stats.files, human_size(stats.bytes))
    } else {
        format!("un archivo de {}", human_size(stats.bytes))
    }
}

fn too_big_error() -> ToolError {
    ToolError::Failed(format!("Es demasiado grande para hacerlo de una vez (más de {MAX_FILES} archivos o {})." , human_size(MAX_BYTES)))
}

fn place_properties(prefix: &str, what: &str) -> serde_json::Map<String, Value> {
    location_properties(prefix, what).as_object().cloned().unwrap_or_default()
}

fn name_property(what: &str) -> Value {
    json!({ "type": "string", "description": what })
}

/// El elemento `name` dentro de `dir`: debe existir y no ser un enlace que lleve fuera de esa carpeta.
fn existing_item(dir: &Path, name: &str) -> Result<PathBuf, ToolError> {
    validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
    let item = dir.join(name).canonicalize().map_err(|_| ToolError::Failed(format!("No existe «{name}» en {}.", display_path(dir))))?;
    if item.parent() != Some(dir) {
        return Err(ToolError::NotAllowed(format!("«{name}» es un enlace que apunta fuera de la carpeta.")));
    }
    Ok(item)
}

fn must_not_exist(dir: &Path, name: &str) -> Result<PathBuf, ToolError> {
    let target = dir.join(name);
    if target.symlink_metadata().is_ok() {
        return Err(ToolError::Failed(format!("Ya existe «{name}» en {}. No lo sobrescribo.", display_path(dir))));
    }
    Ok(target)
}

// ---------------------------------------------------------------------------------------------
// copy_file
// ---------------------------------------------------------------------------------------------

pub struct CopyFile {
    locations: Arc<Locations>,
    planner: MoveFile,
}

impl CopyFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { planner: MoveFile::new(locations.clone()), locations }
    }

    /// Mismas reglas que mover. Sin destino ni nombre nuevo se copia junto al original, como «X - copia».
    fn plan(&self, args: &Value) -> Result<MovePlan, ToolError> {
        let object = args.as_object().ok_or_else(|| invalid("los argumentos deben ser un objeto"))?;
        let given = |key: &str| object.get(key).is_some_and(|value| !value.is_null());
        if given("to_location") || given("to_subfolder") || given("new_name") {
            return self.planner.plan(args);
        }
        let location = parse_location(required_str(args, "location")?)?;
        let name = required_str(args, "name")?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let mut with_name = args.clone();
        with_name["new_name"] = Value::String(unique_copy_name(&dir, name)?);
        self.planner.plan(&with_name)
    }
}

/// «informe.docx» -> «informe - copia.docx», y si existe, «informe - copia (2).docx»…
fn unique_copy_name(dir: &Path, name: &str) -> Result<String, ToolError> {
    validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && dir.join(name).is_file() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for number in 1..=99 {
        let suffix = if number == 1 { " - copia".to_string() } else { format!(" - copia ({number})") };
        let candidate = format!("{stem}{suffix}{extension}");
        if validate_name(&candidate).is_ok() && dir.join(&candidate).symlink_metadata().is_err() {
            return Ok(candidate);
        }
    }
    Err(ToolError::Failed("Ya hay muchas copias de ese elemento; indica un nombre nuevo con new_name.".into()))
}

/// Copia un árbol. Se salta los enlaces y los archivos que solo están en la nube.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)?.flatten() {
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let metadata = fs::symlink_metadata(&source)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            copy_tree(&source, &target)?;
        } else if metadata.is_file() && !has_cloud_only_flag(metadata.file_attributes()) {
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

impl Tool for CopyFile {
    fn name(&self) -> &'static str {
        "copy_file"
    }

    fn description(&self) -> &'static str {
        "Copia un archivo o carpeta dentro del Escritorio, Documentos y Descargas. Indica el origen (location, \
         subfolder, name). Sin destino, deja la copia junto al original como «nombre - copia». Para copiar a otro \
         sitio indica to_location y/o to_subfolder (y new_name si quieres otro nombre). Pide confirmación y nunca sobrescribe."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties("", "dónde está el elemento");
        properties.extend(place_properties("to_", "destino de la copia; si no se indica, junto al original"));
        properties.insert("name".into(), name_property("Nombre del archivo o carpeta a copiar (un solo nombre, sin rutas)."));
        properties.insert("new_name".into(), name_property("Nombre de la copia (opcional)."));
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
            Ok(plan) => {
                let stats = tree_stats(&plan.from);
                format!("Copiar ({}):\n{}\n→\n{}", describe_tree(&plan.from, &stats), display_path(&plan.from), display_path(&plan.to))
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let plan = self.plan(args)?;
        let metadata = fs::symlink_metadata(&plan.from).map_err(|e| ToolError::Failed(format!("No se pudo leer el origen: {e}")))?;
        if metadata.is_file() && has_cloud_only_flag(metadata.file_attributes()) {
            return Err(ToolError::Failed("Ese archivo está solo en la nube (OneDrive): cópialo cuando esté descargado.".into()));
        }
        let stats = tree_stats(&plan.from);
        if stats.too_big {
            return Err(too_big_error());
        }
        let copied = if plan.from.is_dir() { copy_tree(&plan.from, &plan.to) } else { fs::copy(&plan.from, &plan.to).map(|_| ()) };
        if let Err(error) = copied {
            // Lo que haya quedado a medias lo creó esta misma operación (el destino no existía).
            let _ = if plan.to.is_dir() { fs::remove_dir_all(&plan.to) } else { fs::remove_file(&plan.to) };
            return Err(ToolError::Failed(match error.kind() {
                io::ErrorKind::PermissionDenied => "Windows no permite copiarlo (¿está abierto en otro programa?).".into(),
                _ => format!("No se pudo copiar: {error}"),
            }));
        }
        let (from, to) = (display_path(&plan.from), display_path(&plan.to));
        Ok(ToolOutput { summary: format!("Copié {from}\n→ {to}"), data: json!({ "from": from, "to": to, "files": stats.files, "bytes": stats.bytes }) })
    }
}

// ---------------------------------------------------------------------------------------------
// open_path
// ---------------------------------------------------------------------------------------------

/// Solo se abren documentos, imágenes, audio y vídeo y archivos comprimidos: nunca programas ni scripts,
/// que se ejecutarían al abrirlos.
const OPENABLE: &[&str] = &[
    "txt", "md", "csv", "rtf", "pdf", "doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "png", "jpg", "jpeg", "gif",
    "bmp", "webp", "svg", "ico", "tif", "tiff", "heic", "mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "mp4", "mkv", "avi", "mov",
    "webm", "wmv", "m4v", "zip", "7z", "rar", "epub",
];

pub struct OpenPath {
    locations: Arc<Locations>,
}

struct OpenPlan {
    target: PathBuf,
    reveal: bool,
}

impl OpenPath {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn plan(&self, args: &Value) -> Result<OpenPlan, ToolError> {
        check_keys(args, &["location", "subfolder", "name", "reveal"])?;
        let reveal = match args.get("reveal") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(invalid("«reveal» debe ser verdadero o falso")),
        };
        let location = parse_location(required_str(args, "location")?)?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let Some(name) = optional_str(args, "name")? else {
            if reveal {
                return Err(invalid("para mostrar algo en el Explorador indica su «name»"));
            }
            return Ok(OpenPlan { target: dir, reveal: false });
        };
        let target = existing_item(&dir, name)?;
        if target.is_file() && !reveal {
            let extension = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if !OPENABLE.contains(&extension.as_str()) {
                return Err(ToolError::NotAllowed(format!(
                    "No abro archivos «.{extension}» por seguridad (podrían ser programas o scripts). Puedo mostrártelo en el Explorador con reveal."
                )));
            }
        }
        Ok(OpenPlan { target, reveal })
    }
}

fn reveal_in_explorer(path: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // `/select,` exige la ruta entre comillas y pegada; `raw_arg` evita que Rust la re-escape.
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", display_path(path))).spawn().map(|_| ())
}

impl Tool for OpenPath {
    fn name(&self) -> &'static str {
        "open_path"
    }

    fn description(&self) -> &'static str {
        "Abre una carpeta en el Explorador de Windows, o un documento, imagen, audio, vídeo o archivo comprimido con su \
         programa predeterminado (location, subfolder y name; sin name abre la carpeta). Con reveal=true muestra el \
         elemento seleccionado en el Explorador en vez de abrirlo. Nunca abre programas ni scripts. Pide confirmación."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties("", "dónde está");
        properties.insert("name".into(), name_property("Archivo o carpeta a abrir (un solo nombre). Sin él, se abre la carpeta indicada."));
        properties.insert("reveal".into(), json!({ "type": "boolean", "description": "Mostrarlo seleccionado en el Explorador en vez de abrirlo." }));
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.plan(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.plan(args) {
            Ok(plan) if plan.reveal => format!("Mostrar en el Explorador:\n{}", display_path(&plan.target)),
            Ok(plan) if plan.target.is_dir() => format!("Abrir la carpeta:\n{}", display_path(&plan.target)),
            Ok(plan) => format!("Abrir con su programa predeterminado:\n{}", display_path(&plan.target)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let plan = self.plan(args)?;
        let shown = display_path(&plan.target);
        let opened = if plan.reveal { reveal_in_explorer(&plan.target) } else { crate::web_tools::open_with_shell(&shown) };
        opened.map_err(|error| ToolError::Failed(format!("No pude abrirlo: {error}")))?;
        let verb = if plan.reveal { "Mostré en el Explorador" } else { "Abrí" };
        Ok(ToolOutput { summary: format!("{verb}: {shown}"), data: json!({ "path": shown, "reveal": plan.reveal }) })
    }
}

// ---------------------------------------------------------------------------------------------
// zip_files
// ---------------------------------------------------------------------------------------------

pub struct ZipFiles {
    locations: Arc<Locations>,
}

struct ZipPlan {
    source: PathBuf,
    destination: PathBuf,
}

impl ZipFiles {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn plan(&self, args: &Value) -> Result<ZipPlan, ToolError> {
        check_keys(args, &["location", "subfolder", "name", "zip_name"])?;
        let location = parse_location(required_str(args, "location")?)?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let name = required_str(args, "name")?;
        let source = existing_item(&dir, name)?;
        let zip_name = match optional_str(args, "zip_name")? {
            Some(given) => {
                validate_name(given).map_err(|reason| invalid(format!("nombre del zip inválido: {reason}")))?;
                if given.to_lowercase().ends_with(".zip") { given.to_string() } else { format!("{given}.zip") }
            }
            None => {
                let base = if source.is_file() { source.file_stem().map(|s| s.to_string_lossy().into_owned()) } else { None };
                format!("{}.zip", base.unwrap_or_else(|| name.to_string()))
            }
        };
        validate_name(&zip_name).map_err(|reason| invalid(format!("nombre del zip inválido: {reason}")))?;
        let destination = must_not_exist(&dir, &zip_name)?;
        Ok(ZipPlan { source, destination })
    }
}

/// Nombre dentro del zip: ruta relativa a la carpeta que contiene el origen, con «/».
fn entry_name(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn write_tree(zip: &mut ZipWriter<File>, path: &Path, base: &Path, depth: usize) -> Result<(), ToolError> {
    let failed = |what: &str, error: &dyn std::fmt::Display| ToolError::Failed(format!("{what}: {error}"));
    let metadata = fs::symlink_metadata(path).map_err(|e| failed("No se pudo leer", &e))?;
    if metadata.file_type().is_symlink() || depth > MAX_DEPTH {
        return Ok(());
    }
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(metadata.len() >= 0xFFFF_FFFF);
    let name = entry_name(path, base);
    if metadata.is_dir() {
        zip.add_directory(format!("{name}/"), options).map_err(|e| failed("No se pudo comprimir", &e))?;
        let mut entries: Vec<_> = fs::read_dir(path).map_err(|e| failed("No se pudo leer", &e))?.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            write_tree(zip, &entry.path(), base, depth + 1)?;
        }
    } else if metadata.is_file() && !has_cloud_only_flag(metadata.file_attributes()) {
        zip.start_file(name, options).map_err(|e| failed("No se pudo comprimir", &e))?;
        let mut file = File::open(path).map_err(|e| failed(&format!("No se pudo abrir «{}»", path.display()), &e))?;
        io::copy(&mut file, zip).map_err(|e| failed("No se pudo comprimir", &e))?;
    }
    Ok(())
}

impl Tool for ZipFiles {
    fn name(&self) -> &'static str {
        "zip_files"
    }

    fn description(&self) -> &'static str {
        "Comprime un archivo o carpeta del Escritorio, Documentos o Descargas en un .zip que queda junto al original \
         (location, subfolder, name y, si quieres, zip_name). No modifica ni borra el original y nunca sobrescribe. \
         Pide confirmación."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties("", "dónde está el elemento");
        properties.insert("name".into(), name_property("Archivo o carpeta a comprimir (un solo nombre, sin rutas)."));
        properties.insert("zip_name".into(), name_property("Nombre del zip (opcional; por defecto, el del elemento)."));
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
            Ok(plan) => {
                let stats = tree_stats(&plan.source);
                format!("Comprimir ({}):\n{}\n→\n{}", describe_tree(&plan.source, &stats), display_path(&plan.source), display_path(&plan.destination))
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let plan = self.plan(args)?;
        let stats = tree_stats(&plan.source);
        if stats.too_big {
            return Err(too_big_error());
        }
        // Se escribe en un archivo temporal y se renombra al terminar: nunca queda un zip a medias con el nombre final.
        let partial = plan.destination.with_extension("zip.part");
        let result = (|| {
            let file = File::create_new(&partial).map_err(|e| ToolError::Failed(format!("No se pudo crear el zip: {e}")))?;
            let mut zip = ZipWriter::new(file);
            let base = plan.source.parent().unwrap_or(&plan.source);
            write_tree(&mut zip, &plan.source, base, 0)?;
            zip.finish().map_err(|e| ToolError::Failed(format!("No se pudo terminar el zip: {e}")))?;
            fs::rename(&partial, &plan.destination).map_err(|e| ToolError::Failed(format!("No se pudo guardar el zip: {e}")))
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
        let packed = fs::metadata(&plan.destination).map(|m| m.len()).unwrap_or(0);
        let destination = display_path(&plan.destination);
        Ok(ToolOutput {
            summary: format!("Comprimí {} archivos ({}) en {destination} ({}).", stats.files, human_size(stats.bytes), human_size(packed)),
            data: json!({ "zip": destination, "files": stats.files, "original_bytes": stats.bytes, "zip_bytes": packed }),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// unzip_file
// ---------------------------------------------------------------------------------------------

pub struct UnzipFile {
    locations: Arc<Locations>,
}

struct UnzipPlan {
    archive: PathBuf,
    destination: PathBuf,
}

struct ArchiveInfo {
    files: usize,
    bytes: u64,
}

impl UnzipFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn plan(&self, args: &Value) -> Result<UnzipPlan, ToolError> {
        check_keys(args, &["location", "subfolder", "name", "folder_name"])?;
        let location = parse_location(required_str(args, "location")?)?;
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let archive = existing_item(&dir, required_str(args, "name")?)?;
        let is_zip = archive.is_file() && archive.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        if !is_zip {
            return Err(invalid("solo sé descomprimir archivos .zip"));
        }
        let folder = match optional_str(args, "folder_name")? {
            Some(given) => given.to_string(),
            None => archive.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        validate_name(&folder).map_err(|reason| invalid(format!("nombre de la carpeta inválido: {reason}")))?;
        let destination = must_not_exist(&dir, &folder)?;
        Ok(UnzipPlan { archive, destination })
    }
}

/// Mira el contenido sin extraer nada y rechaza el archivo entero si algo no es seguro:
/// rutas que se salen de la carpeta (zip-slip), nombres que Windows no admite o un tamaño exagerado.
fn inspect(archive: &mut ZipArchive<File>) -> Result<ArchiveInfo, ToolError> {
    if archive.len() > MAX_FILES {
        return Err(ToolError::Failed(format!("El zip tiene demasiados elementos (más de {MAX_FILES}).")));
    }
    let mut info = ArchiveInfo { files: 0, bytes: 0 };
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| ToolError::Failed(format!("El zip está dañado o protegido con contraseña: {e}")))?;
        let relative = entry.enclosed_name().ok_or_else(|| ToolError::NotAllowed(format!("El zip contiene una ruta insegura («{}»); no lo extraigo.", String::from_utf8_lossy(entry.name_raw()))))?;
        let parts: Vec<_> = relative.components().collect();
        if parts.len() > ZIP_ENTRY_NAME_DEPTH
            || parts.iter().any(|part| validate_name(&part.as_os_str().to_string_lossy()).is_err())
        {
            return Err(ToolError::NotAllowed(format!("El zip contiene nombres que Windows no admite («{}»); no lo extraigo.", String::from_utf8_lossy(entry.name_raw()))));
        }
        if entry.is_symlink() {
            continue;
        }
        if !entry.is_dir() {
            info.files += 1;
            info.bytes += entry.size();
        }
        if info.bytes > MAX_BYTES {
            return Err(ToolError::Failed(format!("El zip ocuparía más de {} descomprimido; no lo extraigo.", human_size(MAX_BYTES))));
        }
    }
    Ok(info)
}

fn open_archive(path: &Path) -> Result<ZipArchive<File>, ToolError> {
    let file = File::open(path).map_err(|e| ToolError::Failed(format!("No se pudo abrir el zip: {e}")))?;
    ZipArchive::new(file).map_err(|e| ToolError::Failed(format!("No parece un zip válido: {e}")))
}

fn extract(archive: &mut ZipArchive<File>, destination: &Path) -> Result<(), ToolError> {
    let failed = |error: &dyn std::fmt::Display| ToolError::Failed(format!("No se pudo extraer: {error}"));
    fs::create_dir(destination).map_err(|e| failed(&e))?;
    let mut budget = MAX_BYTES;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| failed(&e))?;
        let Some(relative) = entry.enclosed_name() else { continue };
        if entry.is_symlink() {
            continue;
        }
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|e| failed(&e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| failed(&e))?;
        }
        // `create_new` no sobrescribe: dos entradas con el mismo nombre dentro del zip son un error.
        let mut out = File::create_new(&target).map_err(|e| failed(&e))?;
        // El tamaño declarado en el zip puede mentir: se limita lo que realmente se escribe.
        let written = io::copy(&mut (&mut entry).take(budget), &mut out).map_err(|e| failed(&e))?;
        budget = budget.checked_sub(written).ok_or_else(|| failed(&"supera el tamaño máximo"))?;
        if budget == 0 {
            return Err(failed(&"supera el tamaño máximo"));
        }
    }
    Ok(())
}

impl Tool for UnzipFile {
    fn name(&self) -> &'static str {
        "unzip_file"
    }

    fn description(&self) -> &'static str {
        "Descomprime un archivo .zip del Escritorio, Documentos o Descargas en una carpeta nueva junto al zip \
         (location, subfolder, name y, si quieres, folder_name). Nunca sobrescribe, rechaza zips con rutas \
         inseguras y no ejecuta nada de lo que contiene. Pide confirmación."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties("", "dónde está el zip");
        properties.insert("name".into(), name_property("Nombre del archivo .zip (un solo nombre, sin rutas)."));
        properties.insert("folder_name".into(), name_property("Nombre de la carpeta de destino (opcional; por defecto, el nombre del zip)."));
        json!({ "type": "object", "properties": properties, "required": ["location", "name"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.plan(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        let plan = match self.plan(args) {
            Ok(plan) => plan,
            Err(error) => return error.to_string(),
        };
        match open_archive(&plan.archive).and_then(|mut archive| inspect(&mut archive)) {
            Ok(info) => format!(
                "Descomprimir ({} archivos, {}):\n{}\n→\n{}",
                info.files,
                human_size(info.bytes),
                display_path(&plan.archive),
                display_path(&plan.destination)
            ),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let plan = self.plan(args)?;
        let mut archive = open_archive(&plan.archive)?;
        let info = inspect(&mut archive)?;
        if let Err(error) = extract(&mut archive, &plan.destination) {
            // La carpeta de destino no existía: lo que haya dentro lo creó esta operación.
            let _ = fs::remove_dir_all(&plan.destination);
            return Err(error);
        }
        let destination = display_path(&plan.destination);
        Ok(ToolOutput {
            summary: format!("Descomprimí {} archivos ({}) en {destination}.", info.files, human_size(info.bytes)),
            data: json!({ "folder": destination, "files": info.files, "bytes": info.bytes }),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn locations(t: &TempLocations) -> Arc<Locations> {
        Arc::new(t.locations.clone())
    }

    fn write(t: &TempLocations, place: &str, relative: &str, text: &str) {
        let path = t.locations_root(place).join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(t: &TempLocations, place: &str, relative: &str) -> String {
        fs::read_to_string(t.locations_root(place).join(relative)).unwrap()
    }

    /// Un zip construido a mano para probar entradas peligrosas.
    fn evil_zip(t: &TempLocations, entry: &str) {
        let file = File::create(t.locations_root("downloads").join("malo.zip")).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(entry, SimpleFileOptions::default()).unwrap();
        zip.write_all(b"contenido").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn copia_un_archivo_junto_al_original_sin_sobrescribir() {
        let t = temp_locations();
        write(&t, "documents", "informe.docx", "datos");
        let tool = CopyFile::new(locations(&t));
        let args = json!({ "location": "documents", "name": "informe.docx" });
        tool.validate(&args).unwrap();
        assert_eq!(tool.risk(&args), RiskLevel::Confirm);
        assert!(tool.preview(&args).contains("informe - copia.docx"));
        tool.execute(&args).unwrap();
        tool.execute(&args).unwrap();
        assert_eq!(read(&t, "documents", "informe.docx"), "datos");
        assert_eq!(read(&t, "documents", "informe - copia.docx"), "datos");
        assert_eq!(read(&t, "documents", "informe - copia (2).docx"), "datos");
    }

    #[test]
    fn copia_carpetas_enteras_a_otra_ubicacion() {
        let t = temp_locations();
        write(&t, "desktop", "Fotos\\a.jpg", "A");
        write(&t, "desktop", "Fotos\\viaje\\b.jpg", "B");
        let tool = CopyFile::new(locations(&t));
        let args = json!({ "location": "desktop", "name": "Fotos", "to_location": "documents", "new_name": "Respaldo" });
        assert!(tool.preview(&args).contains("2 archivos"));
        tool.execute(&args).unwrap();
        assert_eq!(read(&t, "documents", "Respaldo\\viaje\\b.jpg"), "B");
        assert_eq!(read(&t, "desktop", "Fotos\\a.jpg"), "A", "el original sigue ahí");
        // Un destino que ya existe no se pisa.
        assert!(tool.validate(&args).is_err());
    }

    #[test]
    fn copiar_rechaza_lo_que_no_debe() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", "x");
        let tool = CopyFile::new(locations(&t));
        for args in [
            json!({ "location": "desktop", "name": "no_existe.txt" }),
            json!({ "location": "desktop", "name": "..\\a.txt" }),
            json!({ "location": "desktop", "name": "a.txt", "to_subfolder": "no_existe" }),
            json!({ "location": "desktop", "name": "a.txt", "otra": 1 }),
            json!({ "location": "desktop" }),
        ] {
            assert!(tool.validate(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn open_path_solo_abre_carpetas_y_documentos_seguros() {
        let t = temp_locations();
        write(&t, "downloads", "foto.PNG", "x");
        write(&t, "downloads", "instalador.exe", "x");
        write(&t, "downloads", "script.ps1", "x");
        write(&t, "downloads", "atajo.lnk", "x");
        write(&t, "downloads", "Carpeta\\dentro.txt", "x");
        let tool = OpenPath::new(locations(&t));
        assert_eq!(tool.risk(&json!({ "location": "downloads" })), RiskLevel::Confirm);
        for ok in [
            json!({ "location": "downloads" }),
            json!({ "location": "downloads", "name": "foto.PNG" }),
            json!({ "location": "downloads", "name": "Carpeta" }),
            json!({ "location": "downloads", "subfolder": "Carpeta", "name": "dentro.txt" }),
            json!({ "location": "downloads", "name": "instalador.exe", "reveal": true }),
        ] {
            tool.validate(&ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            json!({ "location": "downloads", "name": "instalador.exe" }),
            json!({ "location": "downloads", "name": "script.ps1" }),
            json!({ "location": "downloads", "name": "atajo.lnk" }),
            json!({ "location": "downloads", "reveal": true }),
            json!({ "location": "downloads", "name": "foto.PNG", "reveal": "si" }),
            json!({ "location": "downloads", "name": "no_existe.txt" }),
            json!({ "location": "downloads", "subfolder": ".." }),
        ] {
            assert!(tool.validate(&bad).is_err(), "{bad}");
        }
        assert!(tool.preview(&json!({ "location": "downloads", "name": "foto.PNG", "reveal": true })).starts_with("Mostrar en el Explorador"));
    }

    #[test]
    fn comprime_y_descomprime_conservando_la_estructura() {
        let t = temp_locations();
        write(&t, "documents", "Proyecto\\leeme.txt", "hola");
        write(&t, "documents", "Proyecto\\src\\main.rs", "fn main() {}");
        fs::create_dir_all(t.locations_root("documents").join("Proyecto").join("vacia")).unwrap();
        let zip = ZipFiles::new(locations(&t));
        let args = json!({ "location": "documents", "name": "Proyecto" });
        assert!(zip.preview(&args).contains("2 archivos"));
        let output = zip.execute(&args).unwrap();
        assert!(t.locations_root("documents").join("Proyecto.zip").is_file());
        assert!(!t.locations_root("documents").join("Proyecto.zip.part").exists(), "no queda el temporal");
        assert!(output.summary.contains("2 archivos"));
        // No se puede comprimir dos veces sobre el mismo zip.
        assert!(zip.validate(&args).is_err());

        let unzip = UnzipFile::new(locations(&t));
        let args = json!({ "location": "documents", "name": "Proyecto.zip", "folder_name": "Restaurado" });
        assert!(unzip.preview(&args).contains("2 archivos"));
        unzip.execute(&args).unwrap();
        assert_eq!(read(&t, "documents", "Restaurado\\Proyecto\\leeme.txt"), "hola");
        assert_eq!(read(&t, "documents", "Restaurado\\Proyecto\\src\\main.rs"), "fn main() {}");
        assert!(t.locations_root("documents").join("Restaurado").join("Proyecto").join("vacia").is_dir());
        assert!(unzip.validate(&args).is_err(), "la carpeta ya existe");
    }

    #[test]
    fn un_archivo_suelto_se_comprime_con_su_nombre() {
        let t = temp_locations();
        write(&t, "desktop", "nota.txt", "abc");
        ZipFiles::new(locations(&t)).execute(&json!({ "location": "desktop", "name": "nota.txt", "zip_name": "mis notas" })).unwrap();
        assert!(t.locations_root("desktop").join("mis notas.zip").is_file());
        let default = ZipFiles::new(locations(&t)).plan(&json!({ "location": "desktop", "name": "nota.txt" })).unwrap();
        assert!(default.destination.ends_with("nota.zip"));
    }

    #[test]
    fn un_zip_con_rutas_que_se_salen_no_se_extrae() {
        for entry in ["../fuera.txt", "..\\fuera.txt", "/absoluta.txt", "C:/Windows/x.txt", "ok/../../fuera.txt"] {
            let t = temp_locations();
            evil_zip(&t, entry);
            let tool = UnzipFile::new(locations(&t));
            let args = json!({ "location": "downloads", "name": "malo.zip" });
            let destination = t.locations_root("downloads").join("malo");
            match tool.execute(&args) {
                Err(error) => {
                    let text = error.to_string();
                    assert!(text.contains("insegura") || text.contains("no admite"), "{entry}: {text}");
                    assert!(!destination.exists(), "{entry}: no se crea la carpeta");
                }
                // La librería convierte una ruta absoluta («/x» o «C:/x») en relativa: el archivo queda
                // dentro de la carpeta de destino, sin escapar. Solo esas dos formas se toleran.
                Ok(_) => {
                    assert!(["/absoluta.txt", "C:/Windows/x.txt"].contains(&entry), "{entry} no debía extraerse");
                    assert!(!walkdir_names(&destination).is_empty());
                }
            }
            // Sea rechazado o no, la ruta con «..» nunca deja nada fuera de la carpeta de destino.
            let base = t.locations_root("downloads").parent().unwrap().to_path_buf();
            assert!(!walkdir_names(&base).iter().any(|name| name.ends_with("fuera.txt")), "{entry}");
        }
    }

    #[test]
    fn un_zip_con_nombres_que_windows_no_admite_se_rechaza() {
        for entry in ["flujo.txt:oculto", "CON.txt", "termina.", "a|b.txt"] {
            let t = temp_locations();
            evil_zip(&t, entry);
            let args = json!({ "location": "downloads", "name": "malo.zip" });
            assert!(UnzipFile::new(locations(&t)).execute(&args).is_err(), "{entry}");
            assert!(!t.locations_root("downloads").join("malo").exists(), "{entry}");
        }
    }

    #[test]
    fn solo_descomprime_zips_validos() {
        let t = temp_locations();
        write(&t, "downloads", "falso.zip", "esto no es un zip");
        write(&t, "downloads", "texto.txt", "hola");
        let tool = UnzipFile::new(locations(&t));
        assert!(tool.validate(&json!({ "location": "downloads", "name": "texto.txt" })).is_err());
        let error = tool.execute(&json!({ "location": "downloads", "name": "falso.zip" })).unwrap_err().to_string();
        assert!(error.contains("zip"), "{error}");
        assert!(!t.locations_root("downloads").join("falso").exists());
    }
}

#[cfg(test)]
fn walkdir_names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        out.push(e.path().strip_prefix(dir).unwrap().display().to_string());
        if e.path().is_dir() { out.extend(walkdir_names(&e.path())); }
    }
    out
}
