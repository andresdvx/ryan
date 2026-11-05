//! Vigila si suena música en cualquier aplicación (Spotify, Apple Music, el navegador…) y lo cuenta: el
//! personaje se pone unos auriculares mientras dura. Se puede apagar en Ajustes > Compañero.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use zen_core::events::Event;
use zen_desktop::listening::{something_playing, MusicDetector};

use crate::settings::Settings;

const POLL: Duration = Duration::from_secs(2);

/// Si ahora mismo se da por sonando música, para que la mascota lo sepa nada más abrirse.
#[derive(Default)]
pub struct MusicFlag(AtomicBool);

/// `MusicFlag` se registra en el constructor de la aplicación (no aquí): la ventana de la mascota pregunta por él
/// nada más cargar y, si ese estado aún no existiera, el comando entraría en pánico dentro del webview.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut detector = MusicDetector::default();
        loop {
            // Con la reacción apagada se lee como silencio: si estaba puesta, los auriculares se quitan solos.
            let heard = Settings::load_effective().music_reaction && something_playing();
            if let Some(playing) = detector.observe(heard) {
                if let Some(flag) = app.try_state::<MusicFlag>() {
                    flag.0.store(playing, Ordering::Relaxed);
                }
                let _ = app.emit("pet-music", playing);
                crate::publish(&app, if playing { Event::MusicStarted } else { Event::MusicStopped });
            }
            std::thread::sleep(POLL);
        }
    });
}

#[tauri::command]
pub fn is_music_playing(app: AppHandle) -> bool {
    app.try_state::<MusicFlag>().is_some_and(|flag| flag.0.load(Ordering::Relaxed))
}
