//! Reacciona a los eventos de la aplicación con sonidos y avisa a la interfaz del avance de las tareas.

use tauri::{AppHandle, Emitter, Manager};
use zen_core::events::{Event, EventBus};
use zen_desktop::sound::{self, SoundKind};

use crate::settings::Settings;

/// Qué sonido corresponde a cada evento (si corresponde alguno).
pub fn sound_for(event: &Event) -> Option<SoundKind> {
    match event {
        Event::ReminderDue { .. } => Some(SoundKind::Reminder),
        Event::BatteryLow { .. }
        | Event::RestSuggested { .. }
        | Event::ToolConfirmationRequired { .. }
        | Event::UsbConnected { .. }
        | Event::ReminderSoon { .. } => Some(SoundKind::Notify),
        Event::BatteryCharged | Event::DownloadFinished { .. } | Event::ToolCompleted { .. } => Some(SoundKind::Success),
        Event::ToolFailed { .. } | Event::AssistantFailed { .. } => Some(SoundKind::Error),
        _ => None,
    }
}

pub fn spawn(app: AppHandle) {
    let events = app.state::<std::sync::Arc<EventBus>>().subscribe();
    std::thread::spawn(move || {
        // Varias herramientas seguidas no deben sonar como una ametralladora.
        let mut last_sound = std::time::Instant::now() - std::time::Duration::from_secs(10);
        while let Ok(event) = events.recv() {
            if let Event::TaskProgress { step, total, tool } = &event {
                let _ = app.emit("task-progress", serde_json::json!({ "step": step, "total": total, "tool": tool }));
            }
            let Some(kind) = sound_for(&event) else { continue };
            // Los ajustes se leen en cada aviso: el cambio en Ajustes se nota al instante.
            let settings = Settings::load();
            if !settings.sound_enabled {
                continue;
            }
            let urgent = matches!(kind, SoundKind::Reminder | SoundKind::Error);
            if !urgent && last_sound.elapsed() < std::time::Duration::from_millis(1500) {
                continue;
            }
            last_sound = std::time::Instant::now();
            sound::play(kind, settings.sound_volume);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_evento_importante_tiene_su_sonido() {
        assert_eq!(sound_for(&Event::ReminderDue { text: "x".into() }), Some(SoundKind::Reminder));
        assert_eq!(sound_for(&Event::BatteryLow { percent: 9 }), Some(SoundKind::Notify));
        assert_eq!(sound_for(&Event::ToolConfirmationRequired { tool: "x".into() }), Some(SoundKind::Notify));
        assert_eq!(sound_for(&Event::ToolCompleted { tool: "x".into() }), Some(SoundKind::Success));
        assert_eq!(sound_for(&Event::DownloadFinished { name: "a.zip".into() }), Some(SoundKind::Success));
        assert_eq!(sound_for(&Event::UsbConnected { drive: 'E' }), Some(SoundKind::Notify));
        assert_eq!(sound_for(&Event::ReminderSoon { text: "x".into(), minutes: 10 }), Some(SoundKind::Notify));
        assert_eq!(sound_for(&Event::ToolFailed { tool: "x".into(), error: "e".into() }), Some(SoundKind::Error));
        assert_eq!(sound_for(&Event::AssistantFailed { error: "e".into() }), Some(SoundKind::Error));
    }

    #[test]
    fn el_movimiento_normal_no_suena() {
        for event in [Event::CharacterClicked, Event::WalkFinished, Event::ChatOpened, Event::AssistantThinking, Event::AssistantReplied, Event::ToolStarted { tool: "x".into() }, Event::TaskProgress { step: 1, total: 2, tool: "x".into() }] {
            assert_eq!(sound_for(&event), None, "{event:?}");
        }
    }
}
