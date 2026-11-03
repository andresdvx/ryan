//! Control de la música y del volumen con las teclas multimedia de Windows.
//! Las teclas actúan sobre la sesión multimedia activa (Spotify, el navegador, etc.), así que no
//! hace falta ninguna API de ningún servicio.

use std::thread::sleep;
use std::time::Duration;

use serde_json::{json, Value};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
    VK_MEDIA_STOP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, required_str};

/// Cada pulsación de volumen mueve el volumen de Windows un 2 %.
const PERCENT_PER_STEP: u32 = 2;
const MAX_STEPS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    PlayPause,
    Next,
    Previous,
    Stop,
    VolumeUp,
    VolumeDown,
    Mute,
}

impl Action {
    const ALL: [Action; 7] = [
        Action::PlayPause,
        Action::Next,
        Action::Previous,
        Action::Stop,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
    ];

    fn key(self) -> &'static str {
        match self {
            Action::PlayPause => "play_pause",
            Action::Next => "next",
            Action::Previous => "previous",
            Action::Stop => "stop",
            Action::VolumeUp => "volume_up",
            Action::VolumeDown => "volume_down",
            Action::Mute => "mute",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.key() == key)
    }

    fn virtual_key(self) -> u8 {
        (match self {
            Action::PlayPause => VK_MEDIA_PLAY_PAUSE,
            Action::Next => VK_MEDIA_NEXT_TRACK,
            Action::Previous => VK_MEDIA_PREV_TRACK,
            Action::Stop => VK_MEDIA_STOP,
            Action::VolumeUp => VK_VOLUME_UP,
            Action::VolumeDown => VK_VOLUME_DOWN,
            Action::Mute => VK_VOLUME_MUTE,
        }) as u8
    }

    fn is_volume_step(self) -> bool {
        matches!(self, Action::VolumeUp | Action::VolumeDown)
    }

    fn describe(self, steps: u32) -> String {
        match self {
            Action::PlayPause => "Pausar o reanudar la reproducción".into(),
            Action::Next => "Pasar a la siguiente canción".into(),
            Action::Previous => "Volver a la canción anterior".into(),
            Action::Stop => "Detener la reproducción".into(),
            Action::VolumeUp => format!("Subir el volumen (≈{}%)", steps * PERCENT_PER_STEP),
            Action::VolumeDown => format!("Bajar el volumen (≈{}%)", steps * PERCENT_PER_STEP),
            Action::Mute => "Silenciar o reactivar el sonido".into(),
        }
    }
}

fn press(virtual_key: u8) {
    // SAFETY: `keybd_event` solo recibe enteros; no hay punteros ni memoria compartida.
    unsafe {
        keybd_event(virtual_key, 0, KEYEVENTF_EXTENDEDKEY, 0);
        keybd_event(virtual_key, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP, 0);
    }
}

fn parse(args: &Value) -> Result<(Action, u32), ToolError> {
    check_keys(args, &["action", "steps"])?;
    let key = required_str(args, "action")?;
    let action = Action::from_key(key).ok_or_else(|| {
        invalid(format!("acción desconocida «{key}»; usa: {}", Action::ALL.map(Action::key).join(", ")))
    })?;
    let steps = match args.get("steps") {
        None | Some(Value::Null) => 1,
        Some(value) => match value.as_u64() {
            Some(n) if (1..=MAX_STEPS).contains(&n) && action.is_volume_step() => n as u32,
            Some(_) if !action.is_volume_step() => return Err(invalid("«steps» solo se usa con volume_up y volume_down")),
            _ => return Err(invalid(format!("«steps» debe ser un entero entre 1 y {MAX_STEPS}"))),
        },
    };
    Ok((action, steps))
}

pub struct MediaControl;

impl Tool for MediaControl {
    fn name(&self) -> &'static str {
        "media_control"
    }

    fn description(&self) -> &'static str {
        "Controla la música y el volumen del equipo con las teclas multimedia: pausar o reanudar, siguiente, \
         anterior, detener, subir o bajar el volumen (cada paso es un 2 %) y silenciar. Actúa sobre lo que \
         esté sonando (Spotify, el navegador…). No puede elegir qué canción suena."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": Action::ALL.map(Action::key), "description": "Qué hacer." },
                "steps": { "type": "integer", "minimum": 1, "maximum": MAX_STEPS, "description": "Solo para volume_up / volume_down: cuántos pasos (opcional, por defecto 1)." }
            },
            "required": ["action"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse(args) {
            Ok((action, steps)) => action.describe(steps),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (action, steps) = parse(args)?;
        let presses = if action.is_volume_step() { steps } else { 1 };
        for i in 0..presses {
            press(action.virtual_key());
            if i + 1 < presses {
                sleep(Duration::from_millis(35));
            }
        }
        Ok(ToolOutput {
            summary: format!("{} ✔", action.describe(steps)),
            data: json!({ "action": action.key(), "steps": presses }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_accion_tiene_su_tecla_y_nombre_unicos() {
        let keys: Vec<u8> = Action::ALL.iter().map(|a| a.virtual_key()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "dos acciones comparten tecla");
        for action in Action::ALL {
            assert_eq!(Action::from_key(action.key()), Some(action));
        }
        // Valores documentados de Windows (VK_MEDIA_*, VK_VOLUME_*).
        assert_eq!(Action::PlayPause.virtual_key(), 0xB3);
        assert_eq!(Action::Next.virtual_key(), 0xB0);
        assert_eq!(Action::Previous.virtual_key(), 0xB1);
        assert_eq!(Action::VolumeUp.virtual_key(), 0xAF);
        assert_eq!(Action::VolumeDown.virtual_key(), 0xAE);
        assert_eq!(Action::Mute.virtual_key(), 0xAD);
    }

    #[test]
    fn valida_acciones_y_pasos() {
        assert_eq!(parse(&json!({ "action": "play_pause" })).unwrap(), (Action::PlayPause, 1));
        assert_eq!(parse(&json!({ "action": "volume_up", "steps": 5 })).unwrap(), (Action::VolumeUp, 5));
        for bad in [
            json!({}),
            json!({ "action": "explotar" }),
            json!({ "action": "volume_up", "steps": 0 }),
            json!({ "action": "volume_up", "steps": MAX_STEPS + 1 }),
            json!({ "action": "volume_up", "steps": -3 }),
            json!({ "action": "volume_up", "steps": "5" }),
            json!({ "action": "volume_up", "steps": 2.5 }),
            json!({ "action": "next", "steps": 3 }),
            json!({ "action": "mute", "otro": 1 }),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn es_segura_y_describe_lo_que_hace() {
        assert_eq!(MediaControl.risk(&json!({})), RiskLevel::Safe);
        assert_eq!(MediaControl.preview(&json!({ "action": "volume_down", "steps": 5 })), "Bajar el volumen (≈10%)");
        assert_eq!(MediaControl.preview(&json!({ "action": "next" })), "Pasar a la siguiente canción");
    }
}
