//! Leer archivos de texto y buscar por nombre o dentro del texto. El contenido que estas herramientas
//! devuelven acaba en el servicio de IA, así que: lo que lee pide confirmación, los archivos con pinta
//! de secretos nunca se abren y todo queda dentro de las ubicaciones permitidas.

use std::collections::VecDeque;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, location_properties, optional_str, optional_u64, parse_location, required_str};
use crate::locations::{display_path, Locations};
use crate::names::{is_sensitive_file_name, validate_name};

const MAX_READ_BYTES: u64 = 2 * 1024 * 1024;
const PAGE_CHARS: usize = 20_000;

const MAX_DEPTH: usize = 4;
const MAX_DIRS: usize = 400;
const MAX_VISITED_FILES: usize = 5_000;
const MAX_CONTENT_FILE_BYTES: u64 = 1024 * 1024;
const MAX_CONTENT_FILES: usize = 1_500;
const MAX_RESULTS: usize = 30;
const MATCHES_PER_FILE: usize = 3;
const SNIPPET_CHARS: usize = 160;
const SEARCH_TIME: Duration = Duration::from_secs(8);

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
// Archivos de OneDrive que solo están en la nube: abrirlos provocaría su descarga.
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
const CLOUD_ONLY: u32 = ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS;

/// Carpetas que no merece la pena recorrer (y que suelen ser enormes).
pub(crate) const SKIPPED_DIRS: &[&str] = &["node_modules", ".git", "target", "__pycache__", ".venv", "$recycle.bin", "system volume information"];

/// Extensiones de texto que se miran por dentro al buscar.
const SEARCHABLE: &[&str] = &[
    "txt", "md", "csv", "json", "xml", "html", "htm", "css", "js", "ts", "tsx", "jsx", "py", "rs", "java", "c", "cpp", "h",
    "cs", "go", "php", "rb", "sh", "bat", "ini", "cfg", "conf", "log", "yaml", "yml", "toml", "sql", "tex", "srt", "lua",
];

pub(crate) fn has_cloud_only_flag(attributes: u32) -> bool {
    attributes & CLOUD_ONLY != 0
}

/// Decodifica texto de Windows: UTF-8 (con o sin BOM) o UTF-16 con BOM. `None` si parece binario.
fn decode_text(bytes: &[u8]) -> Option<String> {
    let text = match bytes {
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes)?,
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes)?,
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).ok()?,
        _ => String::from_utf8(bytes.to_vec()).ok()?,
    };
    // Un texto normal casi no tiene caracteres de control; un binario que por casualidad sea UTF-8 sí.
    let odd = text.chars().filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}')).count();
    (odd * 100 <= text.chars().count().max(1)).then_some(text)
}

fn decode_utf16(bytes: &[u8], read: fn([u8; 2]) -> u16) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.chunks_exact(2).map(|pair| read([pair[0], pair[1]])).collect();
    String::from_utf16(&units).ok()
}

/// Recorte seguro por caracteres (nunca parte un carácter por la mitad).
fn page(text: &str, offset: usize, size: usize) -> (String, usize) {
    let total = text.chars().count();
    let slice: String = text.chars().skip(offset).take(size).collect();
    (slice, total)
}

struct Target {
    path: PathBuf,
    name: String,
    size: u64,
}

fn describe_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

// ---------------------------------------------------------------------------------------------
// read_file
// ---------------------------------------------------------------------------------------------

pub struct ReadFile {
    locations: Arc<Locations>,
}

impl ReadFile {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn target(&self, args: &Value) -> Result<(Target, usize), ToolError> {
        check_keys(args, &["location", "subfolder", "name", "offset_chars"])?;
        let location = parse_location(required_str(args, "location")?)?;
        let name = required_str(args, "name")?;
        validate_name(name).map_err(|reason| invalid(format!("nombre inválido: {reason}")))?;
        let offset = optional_u64(args, "offset_chars", 0, 100_000_000)?.unwrap_or(0) as usize;
        if is_sensitive_file_name(name) {
            return Err(ToolError::NotAllowed(format!(
                "«{name}» parece contener contraseñas o claves por su nombre; no lo leo"
            )));
        }
        let dir = self.locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
        let path = dir
            .join(name)
            .canonicalize()
            .map_err(|_| ToolError::Failed(format!("No existe «{name}» en {}.", display_path(&dir))))?;
        if path.parent() != Some(dir.as_path()) || !self.locations.contains(&path) {
            return Err(ToolError::NotAllowed(format!("«{name}» es un enlace que apunta fuera de tus carpetas.")));
        }
        let metadata = fs::metadata(&path).map_err(|error| ToolError::Failed(format!("No se pudo leer «{name}»: {error}")))?;
        if !metadata.is_file() {
            return Err(ToolError::Failed(format!("«{name}» es una carpeta; usa list_files para ver su contenido.")));
        }
        if has_cloud_only_flag(metadata.file_attributes()) {
            return Err(ToolError::Failed(format!(
                "«{name}» está solo en la nube (OneDrive) y no está descargado en este equipo; no lo descargo."
            )));
        }
        if metadata.len() > MAX_READ_BYTES {
            return Err(ToolError::Failed(format!(
                "«{name}» pesa {} y solo leo archivos de hasta {}.",
                describe_size(metadata.len()),
                describe_size(MAX_READ_BYTES)
            )));
        }
        Ok((Target { path, name: name.to_string(), size: metadata.len() }, offset))
    }
}

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Lee el contenido de un archivo de texto (txt, md, csv, json, código, logs…) del Escritorio, Documentos \
         o Descargas. No lee PDF, Word ni imágenes, ni archivos con pinta de contener contraseñas o claves. \
         Devuelve hasta 20 000 caracteres por llamada; si el archivo es más largo, usa offset_chars para \
         seguir. Pide confirmación porque el contenido se envía al servicio de IA."
    }

    fn parameters(&self) -> Value {
        let mut properties = location_properties("", "dónde está el archivo").as_object().cloned().unwrap_or_default();
        properties.insert("name".into(), json!({ "type": "string", "description": "Nombre del archivo, con extensión." }));
        properties.insert(
            "offset_chars".into(),
            json!({ "type": "integer", "minimum": 0, "description": "Desde qué carácter leer (opcional; para continuar un archivo largo usa el next_offset de la lectura anterior)." }),
        );
        json!({ "type": "object", "properties": properties, "required": ["location", "name"] })
    }

    /// El contenido de un archivo es lo más privado que hay: el usuario decide cada lectura.
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.target(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.target(args) {
            Ok((target, offset)) => {
                let continuing = if offset > 0 { format!(" a partir del carácter {offset}") } else { String::new() };
                format!(
                    "Leer «{}» ({}){continuing} y enviar su contenido al servicio de IA.\n{}\nRevisa que no contenga datos privados.",
                    target.name,
                    describe_size(target.size),
                    display_path(&target.path)
                )
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (target, offset) = self.target(args)?;
        let bytes = fs::read(&target.path).map_err(|error| ToolError::Failed(format!("No se pudo leer «{}»: {error}", target.name)))?;
        let text = decode_text(&bytes).ok_or_else(|| {
            ToolError::Failed(format!(
                "«{}» no es un archivo de texto (puede ser un PDF, un documento de Word, una imagen…). Solo leo texto plano.",
                target.name
            ))
        })?;
        let (content, total) = page(&text, offset, PAGE_CHARS);
        let end = offset + content.chars().count();
        let truncated = end < total;
        let mut summary = format!("Leí «{}»: caracteres {}–{} de {total}", target.name, offset + 1, end.max(offset));
        if truncated {
            summary.push_str(&format!(" (sigue desde el carácter {end})"));
        }
        if offset >= total && total > 0 {
            summary = format!("«{}» tiene {total} caracteres; el desplazamiento {offset} queda fuera.", target.name);
        }
        Ok(ToolOutput {
            summary,
            data: json!({
                "name": target.name,
                "text": content,
                "offset": offset,
                "total_chars": total,
                "truncated": truncated,
                "next_offset": truncated.then_some(end),
            }),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// search_files
// ---------------------------------------------------------------------------------------------

struct Query {
    location_dir: PathBuf,
    start: PathBuf,
    name_contains: Option<String>,
    text: Option<String>,
    max_results: usize,
}

fn parse_query(locations: &Locations, args: &Value) -> Result<Query, ToolError> {
    check_keys(args, &["location", "subfolder", "name_contains", "text", "max_results"])?;
    let location = parse_location(required_str(args, "location")?)?;
    let location_dir = locations.resolve_dir(location, None).map_err(invalid)?;
    let start = locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)?;
    let clean = |value: Option<&str>, what: &str| -> Result<Option<String>, ToolError> {
        match value.map(str::trim) {
            None => Ok(None),
            Some(text) if text.is_empty() => Ok(None),
            Some(text) if text.chars().count() > 100 || text.chars().any(char::is_control) => {
                Err(invalid(format!("«{what}» debe tener como máximo 100 caracteres y ningún salto de línea")))
            }
            Some(text) => Ok(Some(text.to_lowercase())),
        }
    };
    let name_contains = clean(optional_str(args, "name_contains")?, "name_contains")?;
    let text = clean(optional_str(args, "text")?, "text")?;
    if name_contains.is_none() && text.is_none() {
        return Err(invalid("indica «name_contains» (parte del nombre) y/o «text» (texto a buscar dentro)"));
    }
    let max_results = optional_u64(args, "max_results", 1, MAX_RESULTS as u64)?.unwrap_or(20) as usize;
    Ok(Query { location_dir, start, name_contains, text, max_results })
}

struct Hit {
    relative: String,
    is_dir: bool,
    size: Option<u64>,
    lines: Vec<(usize, String)>,
}

#[derive(Default)]
struct SearchStats {
    dirs: usize,
    files: usize,
    content_files: usize,
    cloud_only_skipped: usize,
    stopped_by: Option<&'static str>,
}

pub(crate) fn is_hidden_or_system(attributes: u32, name: &str) -> bool {
    name.starts_with('.') || attributes & (ATTR_HIDDEN | ATTR_SYSTEM) != 0
}

fn snippet(line: &str) -> String {
    let trimmed = line.trim();
    let short: String = trimmed.chars().take(SNIPPET_CHARS).collect();
    if trimmed.chars().count() > SNIPPET_CHARS {
        format!("{short}…")
    } else {
        short
    }
}

/// Líneas (con su número) que contienen `needle` (ya en minúsculas), como mucho `MATCHES_PER_FILE`.
fn matching_lines(text: &str, needle: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(needle))
        .take(MATCHES_PER_FILE)
        .map(|(index, line)| (index + 1, snippet(line)))
        .collect()
}

fn search(locations: &Locations, query: &Query, deadline: Instant) -> (Vec<Hit>, SearchStats) {
    let mut hits = Vec::new();
    let mut stats = SearchStats::default();
    let mut queue = VecDeque::from([(query.start.clone(), 0usize)]);

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if stats.dirs >= MAX_DIRS {
            stats.stopped_by = Some("demasiadas carpetas");
            break;
        }
        // Un enlace o junction puede apuntar fuera: solo se entra si, ya resuelto, sigue dentro de lo permitido.
        let Ok(real_dir) = dir.canonicalize() else { continue };
        if !locations.contains(&real_dir) {
            continue;
        }
        stats.dirs += 1;
        let Ok(read) = fs::read_dir(&real_dir) else { continue };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());

        for entry in entries {
            if Instant::now() >= deadline {
                stats.stopped_by = Some("tiempo");
                break 'walk;
            }
            if hits.len() >= query.max_results {
                stats.stopped_by = Some("máximo de resultados");
                break 'walk;
            }
            let Ok(file_type) = entry.file_type() else { continue };
            let Ok(metadata) = entry.metadata() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let attributes = metadata.file_attributes();
            if is_hidden_or_system(attributes, &name) || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(&query.location_dir).map(|p| p.display().to_string()).unwrap_or_else(|_| name.clone());
            let name_matches = query.name_contains.as_ref().is_none_or(|needle| lower.contains(needle));

            if file_type.is_dir() {
                if SKIPPED_DIRS.contains(&lower.as_str()) {
                    continue;
                }
                // Las carpetas solo cuentan como resultado cuando se busca únicamente por nombre.
                if query.text.is_none() && name_matches {
                    hits.push(Hit { relative, is_dir: true, size: None, lines: Vec::new() });
                }
                if depth < MAX_DEPTH {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }

            stats.files += 1;
            if stats.files > MAX_VISITED_FILES {
                stats.stopped_by = Some("demasiados archivos");
                break 'walk;
            }
            if !name_matches || is_sensitive_file_name(&name) {
                continue;
            }
            match &query.text {
                None => hits.push(Hit { relative, is_dir: false, size: Some(metadata.len()), lines: Vec::new() }),
                Some(needle) => {
                    let extension = Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("");
                    if !SEARCHABLE.contains(&extension) || metadata.len() > MAX_CONTENT_FILE_BYTES {
                        continue;
                    }
                    if has_cloud_only_flag(attributes) {
                        stats.cloud_only_skipped += 1;
                        continue;
                    }
                    if stats.content_files >= MAX_CONTENT_FILES {
                        stats.stopped_by = Some("demasiados archivos de texto");
                        break 'walk;
                    }
                    stats.content_files += 1;
                    // Un enlace de archivo que apunte fuera no se lee.
                    if path.canonicalize().map(|real| !locations.contains(&real)).unwrap_or(true) {
                        continue;
                    }
                    let Some(text) = fs::read(&path).ok().and_then(|bytes| decode_text(&bytes)) else { continue };
                    let lines = matching_lines(&text, needle);
                    if !lines.is_empty() {
                        hits.push(Hit { relative, is_dir: false, size: Some(metadata.len()), lines });
                    }
                }
            }
        }
    }
    (hits, stats)
}

pub struct SearchFiles {
    locations: Arc<Locations>,
}

impl SearchFiles {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }
}

impl Tool for SearchFiles {
    fn name(&self) -> &'static str {
        "search_files"
    }

    fn description(&self) -> &'static str {
        "Busca archivos en el Escritorio, Documentos o Descargas (hasta 4 niveles de subcarpetas). Con \
         «name_contains» busca por parte del nombre; con «text» busca dentro de los archivos de texto y \
         devuelve las líneas donde aparece. Buscar solo por nombre es seguro; buscar dentro del texto pide \
         confirmación porque los fragmentos se envían al servicio de IA. Ignora archivos ocultos, de claves \
         o contraseñas, y los que solo están en la nube."
    }

    fn parameters(&self) -> Value {
        let mut properties = location_properties("", "dónde buscar").as_object().cloned().unwrap_or_default();
        properties.insert("name_contains".into(), json!({ "type": "string", "description": "Parte del nombre del archivo (sin distinguir mayúsculas)." }));
        properties.insert("text".into(), json!({ "type": "string", "description": "Texto a buscar dentro de los archivos de texto (sin distinguir mayúsculas)." }));
        properties.insert("max_results".into(), json!({ "type": "integer", "minimum": 1, "maximum": MAX_RESULTS, "description": "Máximo de resultados (opcional, por defecto 20)." }));
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    /// Por nombre equivale a listar; por contenido devuelve fragmentos de los archivos del usuario.
    fn risk(&self, args: &Value) -> RiskLevel {
        match parse_query(&self.locations, args) {
            Ok(query) if query.text.is_none() => RiskLevel::Safe,
            _ => RiskLevel::Confirm,
        }
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_query(&self.locations, args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_query(&self.locations, args) {
            Ok(query) => {
                let place = display_path(&query.start);
                match (&query.name_contains, &query.text) {
                    (name, Some(text)) => format!(
                        "Buscar «{text}» dentro de los archivos de texto de {place}{} y enviar los fragmentos encontrados al servicio de IA.\nRevisa que no puedan contener datos privados.",
                        name.as_ref().map(|n| format!(" cuyo nombre contenga «{n}»")).unwrap_or_default()
                    ),
                    (Some(name), None) => format!("Buscar archivos y carpetas cuyo nombre contenga «{name}» en {place}"),
                    (None, None) => String::new(),
                }
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let query = parse_query(&self.locations, args)?;
        let (hits, stats) = search(&self.locations, &query, Instant::now() + SEARCH_TIME);

        let mut summary = if hits.is_empty() {
            "No encontré nada.".to_string()
        } else {
            format!("Encontré {} resultado(s):", hits.len())
        };
        for hit in &hits {
            let icon = if hit.is_dir { "📁" } else { "📄" };
            let size = hit.size.map(|s| format!(" ({})", describe_size(s))).unwrap_or_default();
            summary.push_str(&format!("\n{icon} {}{size}", hit.relative));
            for (line, text) in &hit.lines {
                summary.push_str(&format!("\n    línea {line}: {text}"));
            }
        }
        if let Some(reason) = stats.stopped_by {
            summary.push_str(&format!("\n(Búsqueda parcial: me detuve por «{reason}».)"));
        }
        if stats.cloud_only_skipped > 0 {
            summary.push_str(&format!("\n({} archivo(s) están solo en la nube y no los miré.)", stats.cloud_only_skipped));
        }
        let results: Vec<Value> = hits
            .iter()
            .map(|hit| {
                json!({
                    "path": hit.relative,
                    "kind": if hit.is_dir { "folder" } else { "file" },
                    "size_bytes": hit.size,
                    "matches": hit.lines.iter().map(|(line, text)| json!({ "line": line, "text": text })).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(ToolOutput {
            summary,
            data: json!({
                "results": results,
                "partial": stats.stopped_by,
                "folders_scanned": stats.dirs,
                "files_scanned": stats.files,
                "cloud_only_skipped": stats.cloud_only_skipped,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn locations(t: &TempLocations) -> Arc<Locations> {
        Arc::new(t.locations.clone())
    }

    fn write(t: &TempLocations, place: &str, relative: &str, bytes: &[u8]) {
        let path = t.locations_root(place).join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn run_search(t: &TempLocations, args: Value) -> ToolOutput {
        SearchFiles::new(locations(t)).execute(&args).unwrap()
    }

    fn found(output: &ToolOutput) -> Vec<String> {
        let mut paths: Vec<String> =
            output.data["results"].as_array().unwrap().iter().map(|r| r["path"].as_str().unwrap().replace('\\', "/")).collect();
        paths.sort();
        paths
    }

    #[test]
    fn decodifica_utf8_utf16_y_rechaza_binarios() {
        assert_eq!(decode_text("hola ñandú ✔".as_bytes()).unwrap(), "hola ñandú ✔");
        assert_eq!(decode_text(&[&[0xEF, 0xBB, 0xBF][..], "con bom".as_bytes()].concat()).unwrap(), "con bom");
        let utf16le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Bloc de notas ñ".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(decode_text(&utf16le).unwrap(), "Bloc de notas ñ");
        let utf16be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("abc".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode_text(&utf16be).unwrap(), "abc");
        for binary in [&b"%PDF-1.7\x00\x01\x02binario"[..], &[0x89, b'P', b'N', b'G', 0, 0, 0][..], &[0xFF, 0xFE, 0x41][..], &[0xC3, 0x28][..]] {
            assert!(decode_text(binary).is_none(), "{binary:?}");
        }
        assert_eq!(decode_text(b"").unwrap(), "");
    }

    #[test]
    fn pagina_por_caracteres_sin_partir_ninguno() {
        let text = "ñ✔a".repeat(10);
        let (first, total) = page(&text, 0, 7);
        assert_eq!((first.chars().count(), total), (7, 30));
        let (second, _) = page(&text, 7, 7);
        assert_eq!(format!("{first}{second}"), text.chars().take(14).collect::<String>());
        assert_eq!(page(&text, 100, 5).0, "");
    }

    #[test]
    fn lee_un_archivo_y_pide_confirmacion() {
        let t = temp_locations();
        write(&t, "documents", "Proyecto/notas.txt", "línea uno\nlínea dos".as_bytes());
        let tool = ReadFile::new(locations(&t));
        let args = json!({ "location": "documents", "subfolder": "Proyecto", "name": "notas.txt" });
        assert_eq!(tool.risk(&args), RiskLevel::Confirm);
        assert!(tool.preview(&args).contains("notas.txt") && tool.preview(&args).contains("servicio de IA"));
        let output = tool.execute(&args).unwrap();
        assert_eq!(output.data["text"], "línea uno\nlínea dos");
        assert_eq!(output.data["truncated"], false);
        assert_eq!(output.data["next_offset"], Value::Null);
    }

    #[test]
    fn un_archivo_largo_se_lee_por_paginas() {
        let t = temp_locations();
        let long = "abcdefghij".repeat(PAGE_CHARS / 10 * 2 + 100);
        write(&t, "desktop", "largo.txt", long.as_bytes());
        let tool = ReadFile::new(locations(&t));
        let first = tool.execute(&json!({ "location": "desktop", "name": "largo.txt" })).unwrap();
        assert_eq!((first.data["truncated"].clone(), first.data["next_offset"].clone()), (json!(true), json!(PAGE_CHARS)));
        let second = tool.execute(&json!({ "location": "desktop", "name": "largo.txt", "offset_chars": PAGE_CHARS })).unwrap();
        let third = tool.execute(&json!({ "location": "desktop", "name": "largo.txt", "offset_chars": 2 * PAGE_CHARS })).unwrap();
        assert_eq!(third.data["truncated"], false);
        let joined = format!("{}{}{}", first.data["text"].as_str().unwrap(), second.data["text"].as_str().unwrap(), third.data["text"].as_str().unwrap());
        assert_eq!(joined, long, "las tres páginas reconstruyen el archivo entero");
        let past = tool.execute(&json!({ "location": "desktop", "name": "largo.txt", "offset_chars": 99_000_000 })).unwrap();
        assert!(past.summary.contains("queda fuera"));
    }

    #[test]
    fn rechaza_binarios_pesados_carpetas_y_lo_que_no_existe() {
        let t = temp_locations();
        write(&t, "desktop", "foto.png", &[0x89, b'P', b'N', b'G', 0, 1, 2, 3]);
        write(&t, "desktop", "enorme.txt", &vec![b'a'; (MAX_READ_BYTES + 1) as usize]);
        fs::create_dir(t.locations_root("desktop").join("Carpeta")).unwrap();
        let tool = ReadFile::new(locations(&t));
        let read = |name: &str| tool.execute(&json!({ "location": "desktop", "name": name }));
        assert!(matches!(read("foto.png"), Err(ToolError::Failed(m)) if m.contains("no es un archivo de texto")));
        assert!(matches!(read("enorme.txt"), Err(ToolError::Failed(m)) if m.contains("solo leo archivos de hasta")));
        assert!(matches!(read("Carpeta"), Err(ToolError::Failed(m)) if m.contains("es una carpeta")));
        assert!(matches!(read("fantasma.txt"), Err(ToolError::Failed(m)) if m.contains("No existe")));
    }

    #[test]
    fn nunca_lee_archivos_con_pinta_de_secretos() {
        let t = temp_locations();
        for name in [".env", "id_rsa", "contraseñas.txt", "clave.pem", "passwords.csv"] {
            write(&t, "desktop", name, b"super secreto");
        }
        let tool = ReadFile::new(locations(&t));
        for name in [".env", "id_rsa", "contraseñas.txt", "clave.pem", "passwords.csv"] {
            let args = json!({ "location": "desktop", "name": name });
            assert!(matches!(tool.validate(&args), Err(ToolError::NotAllowed(_))), "{name}");
            assert!(tool.execute(&args).is_err(), "{name}");
        }
    }

    #[test]
    fn valida_las_rutas_y_los_argumentos_de_lectura() {
        let t = temp_locations();
        write(&t, "desktop", "a.txt", b"x");
        let tool = ReadFile::new(locations(&t));
        for bad in [
            json!({ "location": "desktop", "name": "..\\..\\Windows\\win.ini" }),
            json!({ "location": "desktop", "name": "C:\\Windows\\win.ini" }),
            json!({ "location": "desktop", "subfolder": "..", "name": "a.txt" }),
            json!({ "location": "system", "name": "a.txt" }),
            json!({ "location": "desktop", "name": "a.txt", "offset_chars": -1 }),
            json!({ "location": "desktop", "name": "a.txt", "offset_chars": "0" }),
            json!({ "location": "desktop", "name": "a.txt", "extra": 1 }),
            json!({ "location": "desktop" }),
        ] {
            assert!(tool.validate(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn busca_por_nombre_en_subcarpetas_y_es_seguro() {
        let t = temp_locations();
        write(&t, "documents", "Informe final.docx", b"x");
        write(&t, "documents", "Proyectos/informe parcial.txt", b"x");
        write(&t, "documents", "Proyectos/Antiguos/INFORME viejo.md", b"x");
        write(&t, "documents", "Proyectos/otro.txt", b"x");
        fs::create_dir(t.locations_root("documents").join("Informes")).unwrap();
        let args = json!({ "location": "documents", "name_contains": "informe" });
        assert_eq!(SearchFiles::new(locations(&t)).risk(&args), RiskLevel::Safe);
        let output = run_search(&t, args);
        assert_eq!(
            found(&output),
            ["Informe final.docx", "Informes", "Proyectos/Antiguos/INFORME viejo.md", "Proyectos/informe parcial.txt"]
        );
        assert!(output.summary.contains("📁 Informes") && output.summary.contains("📄 Informe final.docx"));
    }

    #[test]
    fn busca_dentro_del_texto_con_numero_de_linea_y_pide_confirmacion() {
        let t = temp_locations();
        write(&t, "documents", "a.txt", "uno\ndos tiene la PALABRA clave\ntres\ncuarta PALABRA otra vez".as_bytes());
        write(&t, "documents", "sub/b.md", "sin nada\nPalabra suelta".as_bytes());
        write(&t, "documents", "c.txt", "nada que ver".as_bytes());
        write(&t, "documents", "binario.png", b"PALABRA\x00\x01");
        let tool = SearchFiles::new(locations(&t));
        let args = json!({ "location": "documents", "text": "palabra" });
        assert_eq!(tool.risk(&args), RiskLevel::Confirm);
        assert!(tool.preview(&args).contains("«palabra»") && tool.preview(&args).contains("servicio de IA"));
        let output = tool.execute(&args).unwrap();
        assert_eq!(found(&output), ["a.txt", "sub/b.md"], "el binario y el que no lo contiene no aparecen");
        let a = output.data["results"].as_array().unwrap().iter().find(|r| r["path"] == "a.txt").unwrap();
        assert_eq!(a["matches"][0], json!({ "line": 2, "text": "dos tiene la PALABRA clave" }));
        assert_eq!(a["matches"][1]["line"], 4);
        // Combinar nombre y texto restringe la búsqueda.
        let both = tool.execute(&json!({ "location": "documents", "text": "palabra", "name_contains": "b.md" })).unwrap();
        assert_eq!(found(&both), ["sub/b.md"]);
    }

    #[test]
    fn la_busqueda_ignora_ocultos_secretos_y_carpetas_pesadas() {
        let t = temp_locations();
        write(&t, "desktop", "normal.txt", b"clave-123");
        write(&t, "desktop", ".oculto.txt", b"clave-123");
        write(&t, "desktop", "passwords.txt", b"clave-123");
        write(&t, "desktop", ".env", b"clave-123");
        write(&t, "desktop", "node_modules/paquete/leeme.txt", b"clave-123");
        write(&t, "desktop", ".git/config", b"clave-123");
        let output = run_search(&t, json!({ "location": "desktop", "text": "clave-123" }));
        assert_eq!(found(&output), ["normal.txt"]);
        let by_name = run_search(&t, json!({ "location": "desktop", "name_contains": "passwords" }));
        assert!(found(&by_name).is_empty(), "un archivo de secretos tampoco aparece ni por nombre");
    }

    #[test]
    fn no_entra_en_una_junction_que_apunta_fuera_de_las_ubicaciones() {
        let t = temp_locations();
        write(&t, "data", "secreto.txt", b"contenido-fuera");
        write(&t, "desktop", "visible.txt", b"contenido-fuera");
        let link = t.locations_root("desktop").join("atajo");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(t.locations_root("data"))
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success() && link.exists(), "no se pudo crear la junction de prueba");
        let output = run_search(&t, json!({ "location": "desktop", "text": "contenido-fuera" }));
        assert_eq!(found(&output), ["visible.txt"], "el contenido de fuera no se recorre");
        let by_name = run_search(&t, json!({ "location": "desktop", "name_contains": "secreto" }));
        assert!(found(&by_name).is_empty());
        // Tampoco se puede leer a través de ella.
        let read = ReadFile::new(locations(&t));
        assert!(read.validate(&json!({ "location": "desktop", "subfolder": "atajo", "name": "secreto.txt" })).is_err());
        let _ = fs::remove_dir(&link);
    }

    #[test]
    fn respeta_los_limites_de_resultados_y_profundidad() {
        let t = temp_locations();
        for i in 0..40 {
            write(&t, "downloads", &format!("foto{i:02}.jpg"), b"x");
        }
        write(&t, "downloads", "a/b/c/d/e/muy_hondo.jpg", b"x");
        write(&t, "downloads", "a/b/c/d/cuarto.jpg", b"x");
        let output = run_search(&t, json!({ "location": "downloads", "name_contains": "jpg", "max_results": 5 }));
        assert_eq!(output.data["results"].as_array().unwrap().len(), 5);
        assert_eq!(output.data["partial"], "máximo de resultados");
        let deep = run_search(&t, json!({ "location": "downloads", "name_contains": "hondo" }));
        assert!(found(&deep).is_empty(), "más allá de {MAX_DEPTH} niveles no se busca");
        let fourth = run_search(&t, json!({ "location": "downloads", "name_contains": "cuarto" }));
        assert_eq!(found(&fourth), ["a/b/c/d/cuarto.jpg"]);
    }

    #[test]
    fn valida_los_argumentos_de_busqueda() {
        let t = temp_locations();
        let tool = SearchFiles::new(locations(&t));
        for bad in [
            json!({ "location": "desktop" }),
            json!({ "location": "desktop", "name_contains": "   " }),
            json!({ "location": "desktop", "text": "a\nb" }),
            json!({ "location": "desktop", "text": "x".repeat(101) }),
            json!({ "location": "desktop", "text": "x", "max_results": 0 }),
            json!({ "location": "desktop", "text": "x", "max_results": 31 }),
            json!({ "location": "desktop", "text": "x", "subfolder": ".." }),
            json!({ "location": "desktop", "text": 5 }),
            json!({ "location": "desktop", "text": "x", "otro": 1 }),
        ] {
            assert!(tool.validate(&bad).is_err(), "{bad}");
        }
        // Una consulta inválida nunca baja a «segura».
        assert_eq!(tool.risk(&json!({ "location": "desktop" })), RiskLevel::Confirm);
    }

    #[test]
    fn detecta_los_atributos_de_onedrive_solo_en_la_nube() {
        assert!(has_cloud_only_flag(ATTR_RECALL_ON_DATA_ACCESS | 0x20));
        assert!(has_cloud_only_flag(ATTR_OFFLINE));
        assert!(has_cloud_only_flag(ATTR_RECALL_ON_OPEN));
        assert!(!has_cloud_only_flag(0x20), "un archivo normal");
        assert!(!has_cloud_only_flag(0x400 | 0x20), "un reparse point (también los de OneDrive ya descargados) no basta");
    }

    #[test]
    fn el_resumen_corta_los_fragmentos_largos() {
        let long = "x".repeat(SNIPPET_CHARS * 2);
        let short = snippet(&format!("   {long}   "));
        assert_eq!(short.chars().count(), SNIPPET_CHARS + 1);
        assert!(short.ends_with('…'));
        assert_eq!(snippet("  corto  "), "corto");
    }

    /// Lectura y búsqueda reales (solo lectura) sobre tu Escritorio:
    /// `cargo test -p zen-desktop -- --ignored --nocapture archivos_reales`.
    #[test]
    #[ignore = "usa el Escritorio real"]
    fn archivos_reales_busqueda_y_lectura() {
        let locations = Arc::new(Locations::from_system().expect("carpetas del usuario"));
        let search = SearchFiles::new(locations.clone());
        let by_name = search.execute(&json!({ "location": "desktop", "name_contains": ".lnk", "max_results": 30 })).unwrap();
        println!("--- por nombre ---\n{}", by_name.summary);
        let by_text =
            search.execute(&json!({ "location": "desktop", "subfolder": "Cheki", "text": "ToolExecutor", "max_results": 6 })).unwrap();
        println!("--- por contenido ---\n{}\nparcial: {}", by_text.summary, by_text.data["partial"]);
        assert!(!found(&by_text).is_empty(), "el código del proyecto contiene ToolExecutor");
        let read = ReadFile::new(locations);
        let plan = read.execute(&json!({ "location": "desktop", "subfolder": "Cheki", "name": "plan.md" })).unwrap();
        println!("--- lectura de plan.md ---\n{}", plan.summary);
        assert!(plan.data["total_chars"].as_u64().unwrap() > 1000);
    }
}
