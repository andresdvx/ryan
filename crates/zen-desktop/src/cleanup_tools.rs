//! Herramientas para saber qué ocupa espacio y liberarlo: archivos grandes, tamaño de carpetas,
//! archivos duplicados y vaciar la Papelera. Las tres primeras solo miden (no cambian nada); borrar
//! es siempre una decisión aparte, con `delete_file` (a la Papelera) y confirmación.

use std::collections::BTreeMap;
use std::fs::File;
use std::hash::Hasher;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, location_properties, optional_str, optional_u64, parse_location, required_str};
use crate::locations::{display_path, Locations};
use crate::names::is_sensitive_file_name;
use crate::walk::{human_size, partial_note, scan, FileEntry};

const SCAN_TIME: Duration = Duration::from_secs(10);
const HASH_TIME: Duration = Duration::from_secs(20);
/// Como mucho se leen tantos bytes buscando duplicados.
const HASH_BUDGET: u64 = 4 * 1024 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

/// Carpeta donde medir: la ubicación o una subcarpeta ya existente dentro de ella.
fn start_dir(locations: &Locations, args: &Value) -> Result<PathBuf, ToolError> {
    let location = parse_location(required_str(args, "location")?)?;
    locations.resolve_dir(location, optional_str(args, "subfolder")?).map_err(invalid)
}

fn place_properties() -> serde_json::Map<String, Value> {
    location_properties("", "dónde medir").as_object().cloned().unwrap_or_default()
}

fn number_property(description: &str) -> Value {
    json!({ "type": "integer", "description": description })
}

// ---------------------------------------------------------------------------------------------
// find_large_files
// ---------------------------------------------------------------------------------------------

pub struct FindLargeFiles {
    locations: Arc<Locations>,
}

impl FindLargeFiles {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn parse(&self, args: &Value) -> Result<(PathBuf, u64, usize), ToolError> {
        check_keys(args, &["location", "subfolder", "min_mb", "max_results"])?;
        let start = start_dir(&self.locations, args)?;
        let min_mb = optional_u64(args, "min_mb", 1, 100_000)?.unwrap_or(100);
        let max_results = optional_u64(args, "max_results", 1, 30)?.unwrap_or(15) as usize;
        Ok((start, min_mb, max_results))
    }
}

impl Tool for FindLargeFiles {
    fn name(&self) -> &'static str {
        "find_large_files"
    }

    fn description(&self) -> &'static str {
        "Busca los archivos más grandes del Escritorio, Documentos o Descargas (o de una subcarpeta), para saber \
         qué ocupa espacio. Indica location y, si quieres, subfolder, min_mb (tamaño mínimo, por defecto 100) y \
         max_results. Solo mide: no borra nada. Si el usuario quiere liberar espacio, enséñale la lista y pregúntale \
         qué borrar antes de usar delete_file."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties();
        properties.insert("min_mb".into(), number_property("Tamaño mínimo en MB (por defecto 100)."));
        properties.insert("max_results".into(), number_property("Cuántos archivos listar, de 1 a 30 (por defecto 15)."));
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.parse(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.parse(args) {
            Ok((start, min_mb, _)) => format!("Buscar archivos de más de {min_mb} MB en {}", display_path(&start)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (start, min_mb, max_results) = self.parse(args)?;
        let scanned = scan(&self.locations, &start, Instant::now() + SCAN_TIME);
        let min_bytes = min_mb * 1024 * 1024;
        let cloud_only = scanned.files.iter().filter(|file| file.cloud_only).count();
        let mut big: Vec<&FileEntry> = scanned.files.iter().filter(|f| !f.cloud_only && f.size >= min_bytes).collect();
        big.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.relative.cmp(&b.relative)));
        let shown = &big[..big.len().min(max_results)];

        let mut summary = if big.is_empty() {
            format!("No hay archivos de más de {min_mb} MB en {}.", display_path(&start))
        } else {
            let lines: Vec<String> = shown.iter().map(|f| format!("• {}  {}", human_size(f.size), f.relative)).collect();
            format!("Los {} archivos más grandes (de {} de más de {min_mb} MB) en {}:\n{}", shown.len(), big.len(), display_path(&start), lines.join("\n"))
        };
        summary.push_str(&format!("\nRevisé {} archivos{}.", scanned.files.len(), partial_note(scanned.stopped_by)));
        if cloud_only > 0 {
            summary.push_str(&format!(" {cloud_only} están solo en la nube (OneDrive) y no ocupan espacio aquí."));
        }
        let files: Vec<Value> = shown.iter().map(|f| json!({ "path": f.relative, "size_bytes": f.size })).collect();
        Ok(ToolOutput { summary, data: json!({ "files": files, "total_matches": big.len(), "scanned": scanned.files.len() }) })
    }
}

// ---------------------------------------------------------------------------------------------
// folder_sizes
// ---------------------------------------------------------------------------------------------

pub struct FolderSizes {
    locations: Arc<Locations>,
}

impl FolderSizes {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn parse(&self, args: &Value) -> Result<(PathBuf, usize), ToolError> {
        check_keys(args, &["location", "subfolder", "max_results"])?;
        let start = start_dir(&self.locations, args)?;
        let max_results = optional_u64(args, "max_results", 1, 30)?.unwrap_or(15) as usize;
        Ok((start, max_results))
    }
}

/// Primer tramo de una ruta relativa y si es una carpeta (tiene algo más detrás).
fn top_level(relative: &str) -> (&str, bool) {
    match relative.split_once('\\') {
        Some((first, _)) => (first, true),
        None => (relative, false),
    }
}

impl Tool for FolderSizes {
    fn name(&self) -> &'static str {
        "folder_sizes"
    }

    fn description(&self) -> &'static str {
        "Dice cuánto ocupa cada carpeta y archivo de primer nivel del Escritorio, Documentos o Descargas (o de una \
         subcarpeta), de mayor a menor. Úsala para responder «¿qué ocupa más espacio?». Solo mide: no borra nada."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties();
        properties.insert("max_results".into(), number_property("Cuántos elementos listar, de 1 a 30 (por defecto 15)."));
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.parse(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.parse(args) {
            Ok((start, _)) => format!("Medir lo que ocupa cada elemento de {}", display_path(&start)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (start, max_results) = self.parse(args)?;
        let scanned = scan(&self.locations, &start, Instant::now() + SCAN_TIME);
        // Lo que está solo en la nube no ocupa espacio en este equipo.
        let mut totals: BTreeMap<String, (u64, bool)> = BTreeMap::new();
        let mut local_total = 0u64;
        let mut cloud_only = 0usize;
        for file in &scanned.files {
            if file.cloud_only {
                cloud_only += 1;
                continue;
            }
            let (name, is_folder) = top_level(&file.relative);
            let entry = totals.entry(name.to_string()).or_insert((0, is_folder));
            entry.0 += file.size;
            local_total += file.size;
        }
        let mut sorted: Vec<(String, (u64, bool))> = totals.into_iter().collect();
        sorted.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
        let shown = &sorted[..sorted.len().min(max_results)];

        let lines: Vec<String> = shown
            .iter()
            .map(|(name, (size, folder))| format!("• {}  {}{}", human_size(*size), name, if *folder { "\\ (carpeta)" } else { "" }))
            .collect();
        let mut summary = format!("{} en total en {}:\n{}", human_size(local_total), display_path(&start), lines.join("\n"));
        if sorted.is_empty() {
            summary = format!("No encontré archivos en {}.", display_path(&start));
        }
        summary.push_str(&format!("\nRevisé {} archivos{}.", scanned.files.len(), partial_note(scanned.stopped_by)));
        if cloud_only > 0 {
            summary.push_str(&format!(" {cloud_only} están solo en la nube (OneDrive) y no se cuentan."));
        }
        let items: Vec<Value> = shown.iter().map(|(name, (size, folder))| json!({ "name": name, "size_bytes": size, "is_folder": folder })).collect();
        Ok(ToolOutput { summary, data: json!({ "items": items, "total_bytes": local_total }) })
    }
}

// ---------------------------------------------------------------------------------------------
// find_duplicates
// ---------------------------------------------------------------------------------------------

pub struct FindDuplicates {
    locations: Arc<Locations>,
}

impl FindDuplicates {
    pub fn new(locations: Arc<Locations>) -> Self {
        Self { locations }
    }

    fn parse(&self, args: &Value) -> Result<(PathBuf, u64, usize), ToolError> {
        check_keys(args, &["location", "subfolder", "min_kb", "max_results"])?;
        let start = start_dir(&self.locations, args)?;
        let min_kb = optional_u64(args, "min_kb", 1, 1_000_000)?.unwrap_or(100);
        let max_results = optional_u64(args, "max_results", 1, 20)?.unwrap_or(10) as usize;
        Ok((start, min_kb, max_results))
    }
}

/// Huella del contenido (no es criptográfica: solo sirve para agrupar candidatos, que luego se comparan byte a byte).
fn content_hash(path: &std::path::Path, budget: &mut u64, deadline: Instant) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hasher.finish());
        }
        *budget = budget.checked_sub(read as u64)?;
        hasher.write(&buffer[..read]);
    }
}

fn files_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    let (Ok(mut a), Ok(mut b)) = (File::open(a), File::open(b)) else { return false };
    let (mut left, mut right) = (vec![0u8; CHUNK], vec![0u8; CHUNK]);
    loop {
        let (Ok(n), Ok(m)) = (read_full(&mut a, &mut left), read_full(&mut b, &mut right)) else { return false };
        if n != m || left[..n] != right[..m] {
            return false;
        }
        if n == 0 {
            return true;
        }
    }
}

/// Lee hasta llenar el búfer o llegar al final (un `read` suelto puede devolver menos).
fn read_full(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

struct DuplicateGroup {
    size: u64,
    files: Vec<String>,
}

impl DuplicateGroup {
    fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

/// Grupos de archivos con exactamente el mismo contenido. Devuelve también por qué se quedó a medias, si fue así.
fn find_duplicate_groups(locations: &Locations, files: &[FileEntry], min_bytes: u64) -> (Vec<DuplicateGroup>, Option<&'static str>) {
    let mut by_size: BTreeMap<u64, Vec<&FileEntry>> = BTreeMap::new();
    for file in files.iter().filter(|f| !f.cloud_only && f.size >= min_bytes) {
        let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_sensitive_file_name(&name) {
            by_size.entry(file.size).or_default().push(file);
        }
    }

    let deadline = Instant::now() + HASH_TIME;
    let mut budget = HASH_BUDGET;
    let mut stopped_by = None;
    let mut groups = Vec::new();
    'sizes: for (size, same_size) in by_size.into_iter().filter(|(_, v)| v.len() > 1) {
        let mut by_hash: BTreeMap<u64, Vec<&FileEntry>> = BTreeMap::new();
        for file in same_size {
            // Un enlace que apunte fuera de lo permitido no se lee.
            if file.path.canonicalize().map(|real| !locations.contains(&real)).unwrap_or(true) {
                continue;
            }
            match content_hash(&file.path, &mut budget, deadline) {
                Some(hash) => by_hash.entry(hash).or_default().push(file),
                None => {
                    stopped_by = Some(if Instant::now() >= deadline { "tiempo" } else { "demasiado contenido que leer" });
                    break 'sizes;
                }
            }
        }
        for candidates in by_hash.into_values().filter(|v| v.len() > 1) {
            // La huella agrupa, pero «idéntico» se confirma comparando el contenido.
            let mut classes: Vec<Vec<&FileEntry>> = Vec::new();
            for file in candidates {
                match classes.iter_mut().find(|class| files_equal(&class[0].path, &file.path)) {
                    Some(class) => class.push(file),
                    None => classes.push(vec![file]),
                }
            }
            for class in classes.into_iter().filter(|c| c.len() > 1) {
                let mut names: Vec<String> = class.iter().map(|f| f.relative.clone()).collect();
                names.sort();
                groups.push(DuplicateGroup { size, files: names });
            }
        }
    }
    groups.sort_by(|a, b| b.wasted().cmp(&a.wasted()).then_with(|| a.files.cmp(&b.files)));
    (groups, stopped_by)
}

impl Tool for FindDuplicates {
    fn name(&self) -> &'static str {
        "find_duplicates"
    }

    fn description(&self) -> &'static str {
        "Busca archivos duplicados (con exactamente el mismo contenido) en el Escritorio, Documentos o Descargas, o \
         en una subcarpeta, y dice cuánto espacio se podría liberar. Indica location y, si quieres, subfolder, \
         min_kb (tamaño mínimo, por defecto 100) y max_results (grupos). Solo mide: no borra nada. Para liberar \
         espacio, enséñale al usuario los grupos, pregúntale cuál conservar y envía los demás a la Papelera con delete_file."
    }

    fn parameters(&self) -> Value {
        let mut properties = place_properties();
        properties.insert("min_kb".into(), number_property("Tamaño mínimo en KB (por defecto 100)."));
        properties.insert("max_results".into(), number_property("Cuántos grupos listar, de 1 a 20 (por defecto 10)."));
        json!({ "type": "object", "properties": properties, "required": ["location"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.parse(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.parse(args) {
            Ok((start, min_kb, _)) => format!("Buscar archivos duplicados de más de {min_kb} KB en {}", display_path(&start)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (start, min_kb, max_results) = self.parse(args)?;
        let scanned = scan(&self.locations, &start, Instant::now() + SCAN_TIME);
        let (groups, hash_stop) = find_duplicate_groups(&self.locations, &scanned.files, min_kb * 1024);
        let note = partial_note(scanned.stopped_by.or(hash_stop));
        if groups.is_empty() {
            return Ok(ToolOutput {
                summary: format!("No encontré archivos duplicados de más de {min_kb} KB en {}{note}.", display_path(&start)),
                data: json!({ "groups": [], "wasted_bytes": 0 }),
            });
        }
        let wasted: u64 = groups.iter().map(DuplicateGroup::wasted).sum();
        let shown = &groups[..groups.len().min(max_results)];
        let lines: Vec<String> = shown
            .iter()
            .map(|g| format!("• {} × {} copias:\n    {}", human_size(g.size), g.files.len(), g.files.join("\n    ")))
            .collect();
        let summary = format!(
            "Encontré {} grupos de archivos idénticos; borrando las copias sobrantes se liberarían {}{note}.\n{}\nNo borré nada.",
            groups.len(),
            human_size(wasted),
            lines.join("\n")
        );
        let data: Vec<Value> = shown.iter().map(|g| json!({ "size_bytes": g.size, "files": g.files })).collect();
        Ok(ToolOutput { summary, data: json!({ "groups": data, "wasted_bytes": wasted }) })
    }
}

// ---------------------------------------------------------------------------------------------
// empty_recycle_bin
// ---------------------------------------------------------------------------------------------

pub struct EmptyRecycleBin;

/// Cuántos elementos hay en la Papelera y cuánto ocupan.
fn recycle_bin_contents() -> Result<(u64, u64), String> {
    use windows_sys::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

    // SAFETY: la estructura se inicializa a cero con su tamaño indicado y solo se lee después de que Windows la rellene.
    unsafe {
        let mut info: SHQUERYRBINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
        let result = SHQueryRecycleBinW(std::ptr::null(), &mut info);
        if result < 0 {
            return Err(format!("Windows no pudo consultar la Papelera (código {result:#x})"));
        }
        // Se copian los campos: la estructura puede estar empaquetada y no se debe tomar referencia a ellos.
        let (size, items) = (info.i64Size, info.i64NumItems);
        Ok((items.max(0) as u64, size.max(0) as u64))
    }
}

fn describe_bin(items: u64, bytes: u64) -> String {
    match items {
        0 => "La Papelera ya está vacía.".to_string(),
        1 => format!("Vaciar la Papelera de Windows: 1 elemento ({}). Se borrará PARA SIEMPRE y no se podrá recuperar.", human_size(bytes)),
        n => format!("Vaciar la Papelera de Windows: {n} elementos ({}). Se borrarán PARA SIEMPRE y no se podrán recuperar.", human_size(bytes)),
    }
}

impl Tool for EmptyRecycleBin {
    fn name(&self) -> &'static str {
        "empty_recycle_bin"
    }

    fn description(&self) -> &'static str {
        "Vacía la Papelera de Windows. Es IRREVERSIBLE: lo que hay dentro se borra para siempre. Úsala solo si el \
         usuario lo pide de forma explícita; le mostrará cuántos elementos y cuánto espacio hay y tendrá que confirmar."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Dangerous
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &[])
    }

    fn preview(&self, _args: &Value) -> String {
        match recycle_bin_contents() {
            Ok((items, bytes)) => describe_bin(items, bytes),
            Err(error) => format!("Vaciar la Papelera de Windows (no pude contar lo que hay: {error}). Se borrará PARA SIEMPRE."),
        }
    }

    fn execute(&self, _args: &Value) -> Result<ToolOutput, ToolError> {
        use windows_sys::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND};

        let (items, bytes) = recycle_bin_contents().map_err(ToolError::Failed)?;
        if items == 0 {
            return Ok(ToolOutput { summary: describe_bin(0, 0), data: json!({ "items": 0, "bytes": 0 }) });
        }
        // SAFETY: sin ventana ni ruta (todas las unidades); los indicadores son los documentados.
        let result = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) };
        if result < 0 {
            return Err(ToolError::Failed(format!("Windows no pudo vaciar la Papelera (código {result:#x}).")));
        }
        Ok(ToolOutput {
            summary: format!("Vacié la Papelera: {items} elementos borrados para siempre ({} liberados).", human_size(bytes)),
            data: json!({ "items": items, "bytes": bytes }),
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
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn los_archivos_grandes_salen_de_mayor_a_menor() {
        let t = temp_locations();
        write(&t, "downloads", "video.mp4", &vec![1u8; 3 * 1024 * 1024]);
        write(&t, "downloads", "sub\\foto.png", &vec![2u8; 2 * 1024 * 1024]);
        write(&t, "downloads", "nota.txt", b"pequeno");
        let tool = FindLargeFiles::new(locations(&t));
        let args = json!({ "location": "downloads", "min_mb": 1 });
        tool.validate(&args).unwrap();
        assert_eq!(tool.risk(&args), RiskLevel::Safe);
        let output = tool.execute(&args).unwrap();
        let files = output.data["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["path"], "video.mp4");
        assert_eq!(files[1]["path"], "sub\\foto.png");
        assert!(output.summary.contains("3 MB") && output.summary.contains("Revisé 3 archivos"), "{}", output.summary);
        let none = tool.execute(&json!({ "location": "downloads", "min_mb": 50 })).unwrap();
        assert!(none.summary.contains("No hay archivos de más de 50 MB"));
    }

    #[test]
    fn el_tamano_de_las_carpetas_suma_lo_de_dentro() {
        let t = temp_locations();
        write(&t, "documents", "Fotos\\a.jpg", &vec![0u8; 5000]);
        write(&t, "documents", "Fotos\\viaje\\b.jpg", &vec![0u8; 3000]);
        write(&t, "documents", "Informe.docx", &vec![0u8; 1000]);
        let output = FolderSizes::new(locations(&t)).execute(&json!({ "location": "documents" })).unwrap();
        let items = output.data["items"].as_array().unwrap();
        assert_eq!((items[0]["name"].as_str(), items[0]["size_bytes"].as_u64(), items[0]["is_folder"].as_bool()), (Some("Fotos"), Some(8000), Some(true)));
        assert_eq!((items[1]["name"].as_str(), items[1]["is_folder"].as_bool()), (Some("Informe.docx"), Some(false)));
        assert_eq!(output.data["total_bytes"], 9000);
        assert!(output.summary.contains("Fotos\\ (carpeta)"), "{}", output.summary);
    }

    #[test]
    fn detecta_duplicados_exactos_y_no_se_deja_enganar_por_el_tamano() {
        let t = temp_locations();
        let same = vec![7u8; 200 * 1024];
        let mut almost = same.clone();
        *almost.last_mut().unwrap() = 8; // mismo tamaño, distinto contenido
        write(&t, "downloads", "original.bin", &same);
        write(&t, "downloads", "copias\\copia.bin", &same);
        write(&t, "downloads", "copias\\otra copia.bin", &same);
        write(&t, "downloads", "parecido.bin", &almost);
        write(&t, "downloads", "chico.txt", b"x");
        write(&t, "downloads", "chico2.txt", b"x");
        let tool = FindDuplicates::new(locations(&t));
        let output = tool.execute(&json!({ "location": "downloads" })).unwrap();
        let groups = output.data["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{}", output.summary);
        assert_eq!(groups[0]["files"].as_array().unwrap().len(), 3);
        assert_eq!(output.data["wasted_bytes"], 2 * 200 * 1024);
        assert!(output.summary.contains("No borré nada"));
        let files: Vec<&str> = groups[0]["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(!files.contains(&"parecido.bin") && !files.contains(&"chico.txt"));
    }

    #[test]
    fn sin_duplicados_lo_dice_y_los_archivos_con_pinta_de_secretos_no_se_comparan() {
        let t = temp_locations();
        write(&t, "documents", "a.bin", &vec![1u8; 150 * 1024]);
        write(&t, "documents", "b.bin", &vec![2u8; 150 * 1024]);
        write(&t, "documents", "clave.pem", &vec![3u8; 150 * 1024]);
        write(&t, "documents", "copia\\clave.pem", &vec![3u8; 150 * 1024]);
        let output = FindDuplicates::new(locations(&t)).execute(&json!({ "location": "documents" })).unwrap();
        assert!(output.summary.contains("No encontré archivos duplicados"), "{}", output.summary);
    }

    #[test]
    fn valida_argumentos_y_rutas() {
        let t = temp_locations();
        for tool in [
            Box::new(FindLargeFiles::new(locations(&t))) as Box<dyn Tool>,
            Box::new(FolderSizes::new(locations(&t))),
            Box::new(FindDuplicates::new(locations(&t))),
        ] {
            for args in [
                json!({}),
                json!({ "location": "escritorio" }),
                json!({ "location": "desktop", "subfolder": "no_existe" }),
                json!({ "location": "desktop", "subfolder": ".." }),
                json!({ "location": "desktop", "max_results": 0 }),
                json!({ "location": "desktop", "max_results": 31 }),
                json!({ "location": "desktop", "extra": 1 }),
            ] {
                assert!(tool.validate(&args).is_err(), "{}: {args}", tool.name());
            }
            assert!(tool.validate(&json!({ "location": "desktop" })).is_ok(), "{}", tool.name());
        }
        assert!(FindLargeFiles::new(locations(&t)).validate(&json!({ "location": "desktop", "min_mb": 0 })).is_err());
        assert!(FindDuplicates::new(locations(&t)).validate(&json!({ "location": "desktop", "min_kb": 0 })).is_err());
    }

    #[test]
    fn vaciar_la_papelera_es_peligroso_y_la_vista_previa_avisa_de_que_no_hay_vuelta_atras() {
        assert_eq!(EmptyRecycleBin.risk(&json!({})), RiskLevel::Dangerous);
        assert!(EmptyRecycleBin.validate(&json!({ "todo": true })).is_err());
        assert_eq!(describe_bin(0, 0), "La Papelera ya está vacía.");
        assert!(describe_bin(1, 2048).contains("1 elemento") && describe_bin(1, 2048).contains("PARA SIEMPRE"));
        let many = describe_bin(42, 5 * 1024 * 1024);
        assert!(many.contains("42 elementos") && many.contains("5 MB") && many.contains("PARA SIEMPRE"), "{many}");
    }

    /// Mide tus carpetas reales y muestra solo cifras (nunca nombres). No cambia nada.
    /// Se ejecuta con `cargo test -p zen-desktop -- --ignored --nocapture medicion_real`.
    #[test]
    #[ignore = "recorre tus carpetas reales"]
    fn medicion_real_de_tus_carpetas() {
        use crate::locations::Location;
        let locations = Arc::new(Locations::from_system().expect("carpetas del usuario"));
        for location in [Location::Desktop, Location::Documents, Location::Downloads] {
            let start = locations.resolve_dir(location, None).unwrap();
            let began = Instant::now();
            let scanned = scan(&locations, &start, Instant::now() + SCAN_TIME);
            let local: u64 = scanned.files.iter().filter(|f| !f.cloud_only).map(|f| f.size).sum();
            let cloud = scanned.files.iter().filter(|f| f.cloud_only).count();
            let (groups, hash_stop) = find_duplicate_groups(&locations, &scanned.files, 100 * 1024);
            println!(
                "{:<9} {:>6} archivos, {:>9} locales, {:>4} solo en la nube, {:>3} grupos duplicados ({} MB sobrantes), {} carpetas, {:.1} s, cortado por: {:?}/{:?}",
                location.key(),
                scanned.files.len(),
                human_size(local),
                cloud,
                groups.len(),
                groups.iter().map(DuplicateGroup::wasted).sum::<u64>() / (1024 * 1024),
                scanned.dirs,
                began.elapsed().as_secs_f32(),
                scanned.stopped_by,
                hash_stop
            );
        }
    }

    /// Solo CUENTA lo que hay en la Papelera real (no borra nada).
    /// Se ejecuta con `cargo test -p zen-desktop -- --ignored --nocapture papelera_contenido`.
    #[test]
    #[ignore = "consulta la Papelera real de Windows"]
    fn papelera_contenido_real() {
        let (items, bytes) = recycle_bin_contents().expect("Windows debería poder consultar la Papelera");
        println!("{}", describe_bin(items, bytes));
    }
}
