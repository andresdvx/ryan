//! Recorrido de carpetas para las herramientas que miden o comparan archivos (qué ocupa espacio,
//! duplicados, comprimir). Mismas reglas de seguridad que la búsqueda: no sale de las ubicaciones
//! permitidas (aunque un enlace o una junction apunte fuera), salta ocultos, de sistema y carpetas
//! pesadas, y se corta por número de elementos o por tiempo.

use std::collections::VecDeque;
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::locations::Locations;
use crate::read_tools::{has_cloud_only_flag, is_hidden_or_system, SKIPPED_DIRS};

pub(crate) const MAX_DEPTH: usize = 10;
pub(crate) const MAX_DIRS: usize = 5_000;
pub(crate) const MAX_FILES: usize = 50_000;

pub(crate) struct FileEntry {
    pub path: PathBuf,
    /// Ruta relativa al punto de partida, con `\`.
    pub relative: String,
    pub size: u64,
    /// Archivo de OneDrive que solo está en la nube: no ocupa espacio local y leerlo lo descargaría.
    pub cloud_only: bool,
}

#[derive(Default)]
pub(crate) struct Scan {
    pub files: Vec<FileEntry>,
    pub dirs: usize,
    pub stopped_by: Option<&'static str>,
}

/// Todos los archivos bajo `start`, que debe estar dentro de una ubicación permitida.
pub(crate) fn scan(locations: &Locations, start: &Path, deadline: Instant) -> Scan {
    let mut result = Scan::default();
    let mut queue = VecDeque::from([(start.to_path_buf(), 0usize)]);

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        if result.dirs >= MAX_DIRS {
            result.stopped_by = Some("demasiadas carpetas");
            break;
        }
        // Un enlace o junction puede apuntar fuera: solo se entra si, ya resuelto, sigue dentro de lo permitido.
        let Ok(real_dir) = dir.canonicalize() else { continue };
        if !locations.contains(&real_dir) {
            continue;
        }
        result.dirs += 1;
        let Ok(read) = fs::read_dir(&real_dir) else { continue };

        for entry in read.flatten() {
            if Instant::now() >= deadline {
                result.stopped_by = Some("tiempo");
                break 'walk;
            }
            let (Ok(file_type), Ok(metadata)) = (entry.file_type(), entry.metadata()) else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let attributes = metadata.file_attributes();
            if is_hidden_or_system(attributes, &name) || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if SKIPPED_DIRS.contains(&name.to_lowercase().as_str()) {
                    continue;
                }
                if depth < MAX_DEPTH {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if result.files.len() >= MAX_FILES {
                result.stopped_by = Some("demasiados archivos");
                break 'walk;
            }
            let relative = path.strip_prefix(start).map(|p| p.display().to_string()).unwrap_or(name);
            result.files.push(FileEntry { path, relative, size: metadata.len(), cloud_only: has_cloud_only_flag(attributes) });
        }
    }
    result
}

/// «1,5 GB», «320 MB», «12 KB».
pub(crate) fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.0} MB", value / (KB * KB))
    } else {
        format!("{} KB", bytes.div_ceil(1024).max(1))
    }
}

/// Texto que explica por qué un recorrido se quedó a medias (vacío si terminó).
pub(crate) fn partial_note(stopped_by: Option<&'static str>) -> String {
    stopped_by.map(|why| format!(" (la revisión se detuvo por {why}: puede haber más)")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::testutil::temp_locations;

    fn later() -> Instant {
        Instant::now() + Duration::from_secs(30)
    }

    #[test]
    fn recorre_subcarpetas_y_omite_ocultos_y_carpetas_pesadas() {
        let t = temp_locations();
        let root = t.locations_root("desktop");
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(root.join("raiz.txt"), "1").unwrap();
        std::fs::write(root.join("a").join("uno.txt"), "22").unwrap();
        std::fs::write(root.join("a").join("b").join("dos.txt"), "333").unwrap();
        std::fs::write(root.join(".oculto"), "x").unwrap();
        std::fs::write(root.join("node_modules").join("pesado.js"), "x").unwrap();
        let start = t.locations.resolve_dir(crate::locations::Location::Desktop, None).unwrap();
        let scan = scan(&t.locations, &start, later());
        let mut names: Vec<_> = scan.files.iter().map(|f| f.relative.clone()).collect();
        names.sort();
        assert_eq!(names, ["a\\b\\dos.txt", "a\\uno.txt", "raiz.txt"]);
        assert_eq!(scan.files.iter().map(|f| f.size).sum::<u64>(), 6);
        assert!(scan.stopped_by.is_none());
    }

    #[test]
    fn un_tiempo_agotado_lo_dice() {
        let t = temp_locations();
        std::fs::write(t.locations_root("desktop").join("a.txt"), "1").unwrap();
        let start = t.locations.resolve_dir(crate::locations::Location::Desktop, None).unwrap();
        let scan = scan(&t.locations, &start, Instant::now());
        assert_eq!(scan.stopped_by, Some("tiempo"));
        assert!(partial_note(scan.stopped_by).contains("tiempo") && partial_note(None).is_empty());
    }

    #[test]
    fn describe_tamanos_legibles() {
        assert_eq!(human_size(0), "1 KB");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(320 * 1024 * 1024), "320 MB");
        assert_eq!(human_size(1536 * 1024 * 1024), "1.5 GB");
    }
}
