//! Comentarios de la mascota sobre cómo usas el equipo: saltito al arrancar, sugerencia de descanso,
//! bienvenida tras una ausencia y un poco de aburrimiento si no le haces caso. Se puede apagar en Ajustes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use zen_core::events::{Event, EventBus};
use zen_desktop::usage::{describe_minutes, idle_seconds, UsageNotice, UsageTracker};

use crate::settings::Settings;

const POLL: Duration = Duration::from_secs(30);
/// El chat tarda un momento en cargar al arrancar: el saludo espera para no perderse.
const GREETING_DELAY: Duration = Duration::from_secs(5);

/// Lo que el resto de la aplicación puede saber de tu actividad: solo cuánto llevas trabajando.
pub struct Activity {
    tracker: Arc<Mutex<UsageTracker>>,
    started: Instant,
}

impl Activity {
    pub fn worked_minutes(&self) -> u64 {
        self.tracker.lock().expect("seguimiento envenenado").worked_secs(self.started.elapsed().as_secs()) / 60
    }
}

pub fn spawn(app: AppHandle) {
    let tracker = Arc::new(Mutex::new(UsageTracker::default()));
    let started = Instant::now();
    app.manage(Activity { tracker: tracker.clone(), started });

    // Lo que cuenta como «hacerle caso» a la mascota.
    let events = app.state::<Arc<EventBus>>().subscribe();
    let watcher = tracker.clone();
    std::thread::spawn(move || {
        while let Ok(event) = events.recv() {
            if matches!(event, Event::CharacterClicked | Event::ChatOpened | Event::UserMessageReceived { .. }) {
                watcher.lock().expect("seguimiento envenenado").note_interaction(started.elapsed().as_secs());
            }
        }
    });

    std::thread::spawn(move || {
        std::thread::sleep(GREETING_DELAY);
        if Settings::load().usage_comments {
            // Solo un saludo con la mascota: el chat no recibe ningún texto extra.
            crate::publish(&app, Event::RandomAnimation);
        }
        loop {
            // Se sigue observando aunque los comentarios estén apagados: al activarlos no salta uno atrasado.
            let notice = tracker.lock().expect("seguimiento envenenado").observe(idle_seconds(), started.elapsed().as_secs());
            if let (Some(notice), true) = (notice, Settings::load().usage_comments) {
                announce(&app, notice);
            }
            std::thread::sleep(POLL);
        }
    });
}

fn announce(app: &AppHandle, notice: UsageNotice) {
    match notice {
        UsageNotice::TakeBreak { minutes } => {
            let _ = app.emit("chat-log", format!("☕ Llevas {} sin parar. ¿Un descansito? Estira las piernas y descansa la vista.", describe_minutes(minutes)));
            let _ = app.emit("pet-alert-kind", "rest");
            crate::publish(app, Event::RestSuggested { minutes });
        }
        UsageNotice::WelcomeBack { away_minutes } => {
            let _ = app.emit("chat-log", format!("¡Qué bueno verte de vuelta! Estuviste fuera {}.", describe_minutes(away_minutes)));
            crate::publish(app, Event::RandomAnimation);
        }
        UsageNotice::Bored => {
            let _ = app.emit("chat-log", "Me estoy aburriendo un poquito… ¿me pides algo o jugamos? 🐾");
            crate::publish(app, Event::RandomAnimation);
        }
    }
}
