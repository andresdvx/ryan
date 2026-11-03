//! Cosas que pasan en el equipo y merecen un comentario: una descarga que termina o una unidad USB que se conecta.
//! Solo se mira el nombre y el tamaño de los archivos de Descargas y qué letras de unidad extraíble existen; no se lee
//! ningún contenido. Las decisiones (`DownloadWatcher`, `DriveWatcher`) son lógica pura y se prueban con datos inventados.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

/// Extensiones con las que los navegadores y gestores nombran lo que aún se está descargando.
const PARTIAL_EXTENSIONS: [&str; 6] = ["crdownload", "part", "partial", "download", "opdownload", "tmp"];
/// `GetDriveTypeW` devuelve 2 para las unidades extraíbles (memorias USB, lectores de tarjetas).
const DRIVE_REMOVABLE: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
}

fn is_partial(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| PARTIAL_EXTENSIONS.iter().any(|partial| partial.eq_ignore_ascii_case(ext)))
}

/// Archivos (no carpetas) de `dir`, sin los ocultos de sistema como `desktop.ini`. Vacío si no se puede leer.
pub fn scan_folder(dir: &Path) -> Vec<FileEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            (metadata.is_file() && !name.starts_with('.') && !name.eq_ignore_ascii_case("desktop.ini"))
                .then_some(FileEntry { name, size: metadata.len() })
        })
        .collect()
}

/// Detecta descargas terminadas: un archivo nuevo, que no sea temporal y cuyo tamaño ya no cambia entre dos lecturas.
#[derive(Debug, Default)]
pub struct DownloadWatcher {
    known: HashSet<String>,
    /// Archivos nuevos vistos una vez, con el tamaño que tenían.
    settling: HashMap<String, u64>,
    seeded: bool,
}

impl DownloadWatcher {
    /// Devuelve los nombres que acaban de terminar. La primera lectura solo memoriza lo que ya había.
    pub fn observe(&mut self, entries: &[FileEntry]) -> Vec<String> {
        if !self.seeded {
            self.seeded = true;
            self.known = entries.iter().map(|entry| entry.name.clone()).collect();
            return Vec::new();
        }
        let mut finished = Vec::new();
        for entry in entries {
            if self.known.contains(&entry.name) || is_partial(&entry.name) {
                continue;
            }
            match self.settling.get(&entry.name) {
                Some(&size) if size == entry.size => {
                    self.settling.remove(&entry.name);
                    self.known.insert(entry.name.clone());
                    finished.push(entry.name.clone());
                }
                _ => {
                    self.settling.insert(entry.name.clone(), entry.size);
                }
            }
        }
        // Lo que desapareció se olvida: si vuelve a descargarse con el mismo nombre, se avisa otra vez.
        let present: HashSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        self.known.retain(|name| present.contains(name.as_str()));
        self.settling.retain(|name, _| present.contains(name.as_str()));
        finished.sort();
        finished
    }
}

/// Con cuánta antelación avisa un recordatorio (por ejemplo, una reunión).
pub const HEADS_UP_SECS: u64 = 10 * 60;

/// Avisa una vez, `HEADS_UP_SECS` antes, de los recordatorios que vencen pronto. Uno creado ya dentro de esa ventana
/// no avisa (el usuario acaba de pedirlo) y los que ya estaban así al arrancar tampoco.
#[derive(Debug, Default)]
pub struct HeadsUpWatcher {
    far_ahead: HashSet<u64>,
    warned: HashSet<u64>,
}

impl HeadsUpWatcher {
    /// Devuelve el texto y los minutos que faltan (redondeados hacia arriba) de cada recordatorio que toca avisar.
    pub fn observe(&mut self, now: u64, reminders: &[crate::reminders::Reminder]) -> Vec<(String, u64)> {
        let mut soon = Vec::new();
        for reminder in reminders {
            let remaining = reminder.due.saturating_sub(now);
            if remaining == 0 {
                continue;
            }
            if remaining > HEADS_UP_SECS {
                self.far_ahead.insert(reminder.id);
            } else if self.far_ahead.contains(&reminder.id) && self.warned.insert(reminder.id) {
                soon.push((reminder.text.clone(), remaining.div_ceil(60)));
            }
        }
        let alive: HashSet<u64> = reminders.iter().map(|reminder| reminder.id).collect();
        self.far_ahead.retain(|id| alive.contains(id));
        self.warned.retain(|id| alive.contains(id));
        soon
    }
}

/// Letras de las unidades extraíbles conectadas ahora mismo.
pub fn removable_drives() -> BTreeSet<char> {
    // SAFETY: GetLogicalDrives no recibe nada; GetDriveTypeW recibe una cadena propia terminada en cero.
    unsafe {
        let mask = GetLogicalDrives();
        ('A'..='Z')
            .filter(|letter| mask & (1 << (*letter as u32 - 'A' as u32)) != 0)
            .filter(|letter| {
                let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
                GetDriveTypeW(root.as_ptr()) == DRIVE_REMOVABLE
            })
            .collect()
    }
}

/// Avisa de las unidades extraíbles que aparecen. Lo que ya estaba conectado al arrancar no cuenta.
#[derive(Debug, Default)]
pub struct DriveWatcher {
    known: Option<BTreeSet<char>>,
}

impl DriveWatcher {
    pub fn observe(&mut self, now: BTreeSet<char>) -> Vec<char> {
        let new = match &self.known {
            Some(known) => now.difference(known).copied().collect(),
            None => Vec::new(),
        };
        self.known = Some(now);
        new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> FileEntry {
        FileEntry { name: name.into(), size }
    }

    #[test]
    fn lo_que_ya_estaba_no_se_anuncia() {
        let mut watcher = DownloadWatcher::default();
        assert!(watcher.observe(&[file("viejo.pdf", 10)]).is_empty());
        assert!(watcher.observe(&[file("viejo.pdf", 10)]).is_empty());
    }

    #[test]
    fn un_archivo_nuevo_se_anuncia_cuando_su_tamano_se_estabiliza() {
        let mut watcher = DownloadWatcher::default();
        watcher.observe(&[]);
        assert!(watcher.observe(&[file("foto.png", 100)]).is_empty(), "primera vez: aún puede estar escribiéndose");
        assert!(watcher.observe(&[file("foto.png", 250)]).is_empty(), "sigue creciendo");
        assert_eq!(watcher.observe(&[file("foto.png", 250)]), ["foto.png"]);
        assert!(watcher.observe(&[file("foto.png", 250)]).is_empty(), "una sola vez");
    }

    #[test]
    fn los_temporales_del_navegador_no_cuentan_hasta_que_se_renombran() {
        let mut watcher = DownloadWatcher::default();
        watcher.observe(&[]);
        for size in [10, 500, 900] {
            assert!(watcher.observe(&[file("informe.pdf.crdownload", size)]).is_empty());
        }
        assert!(watcher.observe(&[file("informe.pdf", 900)]).is_empty());
        assert_eq!(watcher.observe(&[file("informe.pdf", 900)]), ["informe.pdf"]);
    }

    #[test]
    fn reconoce_las_extensiones_parciales_sin_importar_mayusculas() {
        for name in ["a.crdownload", "a.PART", "a.Partial", "a.download", "a.opdownload", "a.tmp"] {
            assert!(is_partial(name), "{name}");
        }
        for name in ["a.pdf", "partial", "a.partido", "tmp"] {
            assert!(!is_partial(name), "{name}");
        }
    }

    #[test]
    fn volver_a_descargar_lo_mismo_tras_borrarlo_avisa_otra_vez() {
        let mut watcher = DownloadWatcher::default();
        watcher.observe(&[]);
        watcher.observe(&[file("a.zip", 5)]);
        assert_eq!(watcher.observe(&[file("a.zip", 5)]), ["a.zip"]);
        assert!(watcher.observe(&[]).is_empty(), "se borró");
        watcher.observe(&[file("a.zip", 7)]);
        assert_eq!(watcher.observe(&[file("a.zip", 7)]), ["a.zip"]);
    }

    #[test]
    fn varias_descargas_a_la_vez_salen_ordenadas() {
        let mut watcher = DownloadWatcher::default();
        watcher.observe(&[]);
        let batch = [file("b.txt", 1), file("a.txt", 1)];
        watcher.observe(&batch);
        assert_eq!(watcher.observe(&batch), ["a.txt", "b.txt"]);
    }

    #[test]
    fn solo_se_anuncian_unidades_que_aparecen_despues_de_arrancar() {
        let mut watcher = DriveWatcher::default();
        assert!(watcher.observe(BTreeSet::from(['E'])).is_empty(), "ya estaba al arrancar");
        assert!(watcher.observe(BTreeSet::from(['E'])).is_empty());
        assert_eq!(watcher.observe(BTreeSet::from(['E', 'F'])), ['F']);
        assert!(watcher.observe(BTreeSet::new()).is_empty(), "retirarla no es una novedad");
        assert_eq!(watcher.observe(BTreeSet::from(['E'])), ['E'], "volver a conectarla sí");
    }

    fn reminder(id: u64, due: u64) -> crate::reminders::Reminder {
        crate::reminders::Reminder { id, due, text: format!("reunión {id}") }
    }

    #[test]
    fn avisa_diez_minutos_antes_y_una_sola_vez() {
        let mut watcher = HeadsUpWatcher::default();
        let meeting = [reminder(1, 3_000)];
        assert!(watcher.observe(0, &meeting).is_empty(), "falta mucho");
        assert!(watcher.observe(2_300, &meeting).is_empty(), "11 min: aún no");
        assert_eq!(watcher.observe(2_400, &meeting), [("reunión 1".to_string(), 10)]);
        assert!(watcher.observe(2_460, &meeting).is_empty(), "no repite");
        assert!(watcher.observe(3_000, &meeting).is_empty(), "al vencer avisa el propio recordatorio");
    }

    #[test]
    fn un_recordatorio_creado_dentro_de_la_ventana_no_avisa() {
        let mut watcher = HeadsUpWatcher::default();
        assert!(watcher.observe(100, &[reminder(1, 400)]).is_empty());
        assert!(watcher.observe(160, &[reminder(1, 400)]).is_empty());
    }

    #[test]
    fn redondea_los_minutos_hacia_arriba_y_olvida_lo_cancelado() {
        let mut watcher = HeadsUpWatcher::default();
        watcher.observe(0, &[reminder(1, 5_000), reminder(2, 5_000)]);
        let soon = watcher.observe(4_450, &[reminder(1, 5_000)]);
        assert_eq!(soon, [("reunión 1".to_string(), 10)], "550 s son 10 min");
        assert!(watcher.observe(4_460, &[]).is_empty());
        assert!(watcher.far_ahead.is_empty() && watcher.warned.is_empty(), "no acumula recordatorios que ya no existen");
    }

    #[test]
    fn leer_unidades_y_carpetas_no_falla() {
        let _ = removable_drives();
        assert!(scan_folder(Path::new("Z:\\no\\existe")).is_empty());
    }
}
