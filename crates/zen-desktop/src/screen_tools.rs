//! `look_at_screen`: la IA mira UNA ventana (una captura) para responder sobre lo que muestra.
//! Cada vez pide confirmación y nombra la ventana exacta; nunca captura terminales, administradores
//! ni ventanas de contraseñas o pagos, no guarda nada en disco y solo funciona con modelos con visión.

use std::sync::Mutex;

use serde_json::{json, Value};
use zen_core::ai::Image;
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, optional_str};
use crate::input::{self, WindowInfo};
use crate::input_tools::{describe_window, rate_limit, resolve, Previewed};
use crate::screen;

#[derive(Default)]
pub struct LookAtScreen {
    previewed: Previewed,
    images: Mutex<Vec<Image>>,
}

impl LookAtScreen {
    pub fn new() -> Self {
        Self::default()
    }
}

/// La ventana de más arriba que no sea esta aplicación (que ya viene excluida) ni el escritorio.
fn pick_default(windows: Vec<WindowInfo>, minimized: impl Fn(isize) -> bool) -> Option<WindowInfo> {
    windows.into_iter().find(|w| !w.title.eq_ignore_ascii_case("program manager") && !minimized(w.hwnd))
}

/// La ventana pedida por su título o, sin título, la que el usuario tenía delante antes de abrir el chat.
fn target(args: &Value) -> Result<WindowInfo, ToolError> {
    check_keys(args, &["window"])?;
    match optional_str(args, "window")?.map(str::trim).filter(|query| !query.is_empty()) {
        Some(query) if query.chars().count() > 200 => Err(invalid("«window» es demasiado largo (máximo 200 caracteres)")),
        Some(query) => resolve(query),
        None => {
            let window = pick_default(input::visible_windows(), screen::is_minimized)
                .ok_or_else(|| ToolError::Failed("No hay ninguna ventana abierta que mirar.".into()))?;
            input::check_target(&window).map_err(ToolError::NotAllowed)?;
            Ok(window)
        }
    }
}

impl Tool for LookAtScreen {
    fn name(&self) -> &'static str {
        "look_at_screen"
    }

    fn description(&self) -> &'static str {
        "Mira una ventana del usuario mediante una captura de imagen, para responder sobre lo que muestra \
         (un error, un documento, una web, una app). Sin «window» mira la ventana en la que el usuario \
         estaba trabajando; con «window» (parte del título, mira list_windows) mira esa. Solo si el usuario \
         lo pide o lo necesitas para ayudarle. Pide confirmación y nunca mira terminales ni ventanas de \
         contraseñas o pagos. Después de llamarla recibirás la imagen: descríbela o responde con lo que ves."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "window": { "type": "string", "description": "Parte del título de la ventana a mirar (opcional; sin ella, la ventana en la que el usuario estaba)." }
            }
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        // Lo que muestra una ventana puede ser privado: siempre se confirma, con detalle.
        RiskLevel::Dangerous
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        target(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match target(args) {
            Ok(window) => {
                self.previewed.remember(&window);
                format!(
                    "Mirar la ventana {}.\n\nSe hace una captura solo de esa ventana y se le muestra al modelo de IA. \
                     Si usas un servicio en la nube, la imagen sale de tu equipo; con Ollama se queda en él. No se guarda en disco.",
                    describe_window(&window)
                )
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        self.images.lock().expect("imágenes envenenadas").clear();
        let window = target(args)?;
        // Si la ventana de delante cambió desde que el usuario confirmó, no se captura otra distinta.
        self.previewed.verify(&window)?;
        rate_limit()?;
        let shot = screen::capture_window(window.hwnd).map_err(ToolError::Failed)?;
        let image = Image { mime: "image/png".into(), base64: shot.base64() };
        *self.images.lock().expect("imágenes envenenadas") = vec![image];
        Ok(ToolOutput {
            summary: format!("Miré la ventana «{}» ({}×{})", window.title, shot.width, shot.height),
            data: json!({
                "window": window.title, "program": window.process, "width": shot.width, "height": shot.height,
                "window_size": [shot.original.0, shot.original.1],
            }),
        })
    }

    fn take_images(&self) -> Vec<Image> {
        std::mem::take(&mut *self.images.lock().expect("imágenes envenenadas"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(hwnd: isize, title: &str) -> WindowInfo {
        WindowInfo { hwnd, title: title.into(), process: "notepad".into(), pid: 4 }
    }

    #[test]
    fn la_ventana_por_defecto_es_la_primera_que_no_es_el_escritorio_ni_esta_minimizada() {
        let windows = vec![window(1, "Program Manager"), window(2, "Minimizada"), window(3, "Informe - Word"), window(4, "Otra")];
        let picked = pick_default(windows, |hwnd| hwnd == 2).unwrap();
        assert_eq!(picked.title, "Informe - Word");
        assert!(pick_default(vec![window(1, "PROGRAM MANAGER")], |_| false).is_none());
    }

    #[test]
    fn es_peligrosa_y_rechaza_argumentos_raros() {
        let tool = LookAtScreen::new();
        assert_eq!(tool.risk(&json!({})), RiskLevel::Dangerous);
        for args in [json!({ "monitor": 1 }), json!({ "window": 5 }), json!({ "window": "x".repeat(201) })] {
            assert!(tool.validate(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn una_ventana_inexistente_no_se_captura() {
        let tool = LookAtScreen::new();
        assert!(tool.validate(&json!({ "window": "zzz-no-existe-esta-ventana-zzz" })).is_err());
        assert!(tool.take_images().is_empty());
    }

    #[test]
    fn es_una_herramienta_de_vision_para_el_asistente() {
        assert!(zen_core::tools::VISION_TOOLS.contains(&LookAtScreen::new().name()));
    }
}
