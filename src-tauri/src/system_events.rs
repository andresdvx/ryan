//! Avisos del equipo: descargas que terminan, unidades USB que se conectan y recordatorios que vencen pronto.
//! Lo de Descargas y USB se puede apagar en Ajustes > Avisos; el aviso previo de un recordatorio no, porque es algo
//! que el propio usuario pidió.

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use zen_core::events::Event;
use zen_desktop::system_events::{removable_drives, scan_folder, DownloadWatcher, DriveWatcher, HeadsUpWatcher};
use zen_desktop::{unix_now, Reminders};

use crate::settings::Settings;

const POLL: Duration = Duration::from_secs(5);
/// Varias descargas a la vez se resumen en un solo mensaje.
const MAX_LISTED: usize = 3;
const MAX_NAME_CHARS: usize = 80;

pub fn spawn(app: AppHandle, reminders: Arc<Reminders>) {
    std::thread::spawn(move || {
        let downloads = dirs::download_dir();
        let mut download_watcher = DownloadWatcher::default();
        let mut drive_watcher = DriveWatcher::default();
        let mut heads_up = HeadsUpWatcher::default();
        loop {
            // Se sigue observando aunque los avisos estén apagados: al activarlos no salta lo que ya pasó.
            let enabled = Settings::load().system_events;
            let finished = downloads.as_deref().map(|dir| download_watcher.observe(&scan_folder(dir))).unwrap_or_default();
            let drives = drive_watcher.observe(removable_drives());
            if enabled {
                announce_downloads(&app, finished);
                for drive in drives {
                    let _ = app.emit("chat-log", format!("💾 Se conectó una unidad extraíble ({drive}:)."));
                    crate::publish(&app, Event::UsbConnected { drive });
                }
            }
            for (text, minutes) in heads_up.observe(unix_now(), &reminders.list()) {
                let _ = app.emit("chat-log", format!("⏰ En {minutes} min: {text}"));
                crate::show_chat_with(&app, false);
                crate::publish(&app, Event::ReminderSoon { text, minutes });
            }
            std::thread::sleep(POLL);
        }
    });
}

fn announce_downloads(app: &AppHandle, finished: Vec<String>) {
    let Some(first) = finished.first() else { return };
    let message = if finished.len() <= MAX_LISTED {
        let names: Vec<String> = finished.iter().map(|name| shorten(name)).collect();
        format!("⬇️ Descarga terminada: {}", names.join(", "))
    } else {
        format!("⬇️ Terminaron {} descargas (la primera: {}).", finished.len(), shorten(first))
    };
    let _ = app.emit("chat-log", message);
    crate::publish(app, Event::DownloadFinished { name: first.clone() });
}

/// Un nombre larguísimo no debe desbordar el chat.
fn shorten(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_string();
    }
    let head: String = name.chars().take(MAX_NAME_CHARS - 1).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_nombres_largos_se_acortan() {
        assert_eq!(shorten("corto.txt"), "corto.txt");
        let long = "a".repeat(200);
        assert_eq!(shorten(&long).chars().count(), MAX_NAME_CHARS);
        assert!(shorten(&long).ends_with('…'));
    }
}
