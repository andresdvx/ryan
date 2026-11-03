//! Reproductor del chat: cada segundo (solo con el chat visible) lee lo que suena y avisa a la interfaz.

use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use zen_desktop::media_session::{self, Control, NowPlaying};

const POLL: Duration = Duration::from_secs(1);

fn to_json(now: &NowPlaying) -> Value {
    json!({
        "app": now.app,
        "title": now.title,
        "artist": now.artist,
        "album": now.album,
        "state": now.state.as_str(),
        "position": now.position_secs,
        "duration": now.duration_secs,
        "can_play_pause": now.can_play_pause,
        "can_next": now.can_next,
        "can_previous": now.can_previous,
        "artwork_key": now.artwork_key,
    })
}

fn chat_visible(app: &AppHandle) -> bool {
    app.get_webview_window("chat").and_then(|chat| chat.is_visible().ok()).unwrap_or(false)
}

/// Hilo que vigila los controles multimedia. Solo trabaja mientras el chat está a la vista.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_state: Option<Value> = None;
        let mut last_art_key = String::new();
        loop {
            std::thread::sleep(POLL);
            if !chat_visible(&app) {
                // Al volver a abrir el chat se reenvía todo, incluida la carátula.
                last_state = None;
                last_art_key.clear();
                continue;
            }
            let state = match media_session::now_playing() {
                Ok(Some(now)) => {
                    if now.artwork_key != last_art_key {
                        last_art_key = now.artwork_key.clone();
                        let _ = app.emit("media-artwork", json!({ "key": now.artwork_key, "image": media_session::artwork() }));
                    }
                    to_json(&now)
                }
                // Sin sesión o sin poder consultarla: la interfaz oculta el reproductor.
                _ => {
                    last_art_key.clear();
                    Value::Null
                }
            };
            // La posición cambia cada segundo; la interfaz la interpola, así que solo se avisa si cambia otra cosa
            // o si la posición se desvía de lo esperado (el usuario movió la barra, saltó de pista…).
            if !same_apart_from_position(&last_state, &state) {
                let _ = app.emit("media-state", &state);
            }
            last_state = Some(state);
        }
    });
}

fn same_apart_from_position(previous: &Option<Value>, current: &Value) -> bool {
    let Some(previous) = previous else { return false };
    let (Some(a), Some(b)) = (previous.as_object(), current.as_object()) else { return previous == current };
    if a.keys().any(|key| key != "position" && a.get(key) != b.get(key)) || a.len() != b.len() {
        return false;
    }
    let (pa, pb) = (a["position"].as_f64().unwrap_or(0.0), b["position"].as_f64().unwrap_or(0.0));
    // Sonando avanza ~1 s por vuelta; una desviación mayor es un salto real.
    let expected = if a["state"] == "playing" { pa + POLL.as_secs_f64() } else { pa };
    (pb - expected).abs() < 1.5
}

/// Estado actual, para pintar el reproductor nada más abrir el chat.
#[tauri::command]
pub async fn get_media() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let now = media_session::now_playing()?;
        Ok(match now {
            Some(now) => json!({ "state": to_json(&now), "artwork": media_session::artwork() }),
            None => Value::Null,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Botón del reproductor: `play_pause`, `next` o `previous`.
#[tauri::command]
pub async fn media_control(action: String) -> Result<(), String> {
    let control = Control::from_key(&action).ok_or_else(|| format!("Orden desconocida: {action}"))?;
    let accepted = tauri::async_runtime::spawn_blocking(move || media_session::control(control)).await.map_err(|error| error.to_string())??;
    if accepted {
        Ok(())
    } else {
        Err("La aplicación no aceptó la orden.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(position: f64, title: &str, playing: &str) -> Value {
        json!({ "title": title, "state": playing, "position": position })
    }

    #[test]
    fn el_avance_normal_no_genera_eventos() {
        let prev = Some(state(10.0, "a", "playing"));
        assert!(same_apart_from_position(&prev, &state(11.0, "a", "playing")));
        assert!(same_apart_from_position(&prev, &state(11.9, "a", "playing")));
        let paused = Some(state(10.0, "a", "paused"));
        assert!(same_apart_from_position(&paused, &state(10.0, "a", "paused")));
    }

    #[test]
    fn los_cambios_reales_si_generan_eventos() {
        let prev = Some(state(10.0, "a", "playing"));
        assert!(!same_apart_from_position(&prev, &state(11.0, "b", "playing")), "otra pista");
        assert!(!same_apart_from_position(&prev, &state(11.0, "a", "paused")), "pausa");
        assert!(!same_apart_from_position(&prev, &state(60.0, "a", "playing")), "salto de posición");
        assert!(!same_apart_from_position(&prev, &state(3.0, "a", "playing")), "retroceso");
        assert!(!same_apart_from_position(&None, &state(1.0, "a", "playing")), "primera vez");
        assert!(!same_apart_from_position(&prev, &Value::Null), "se acabó la sesión");
        assert!(!same_apart_from_position(&Some(Value::Null), &state(1.0, "a", "playing")), "empieza una sesión");
        assert!(same_apart_from_position(&Some(Value::Null), &Value::Null));
    }
}
