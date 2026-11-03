//! Herramientas para actuar sobre otras aplicaciones: escribir, pulsar teclas y pulsar controles.
//! Son las más delicadas del programa, así que: vienen apagadas (interruptor en Ajustes), cada acción
//! pide confirmación detallada que nombra la ventana exacta, se rechazan terminales y ventanas de
//! contraseñas, y hay un límite de acciones por minuto.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, required_str};
use crate::input::{self, WindowInfo};
use crate::uia;

/// Herramientas que dependen del interruptor «Control de otras aplicaciones».
pub const TOOL_NAMES: [&str; 5] = ["list_windows", "list_controls", "type_text", "press_keys", "click_control"];

const MAX_ACTIONS_PER_MINUTE: usize = 20;
static RECENT_ACTIONS: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());

/// Cuenta una acción y rechaza si se pasa del límite: una IA desbocada no puede teclear sin parar.
pub(crate) fn rate_limit() -> Result<(), ToolError> {
    let mut recent = RECENT_ACTIONS.lock().expect("contador envenenado");
    let now = Instant::now();
    while recent.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
        recent.pop_front();
    }
    if recent.len() >= MAX_ACTIONS_PER_MINUTE {
        return Err(ToolError::NotAllowed(format!("demasiadas acciones sobre otras aplicaciones en un minuto (máximo {MAX_ACTIONS_PER_MINUTE})")));
    }
    recent.push_back(now);
    Ok(())
}

/// Encuentra la ventana pedida y comprueba que es un destino permitido.
pub(crate) fn resolve(query: &str) -> Result<WindowInfo, ToolError> {
    let windows = input::visible_windows();
    let matches = input::find_windows(&windows, query);
    let window = match matches.as_slice() {
        [one] => one.clone(),
        [] => {
            let open: Vec<String> = windows.iter().take(12).map(|w| format!("«{}»", w.title)).collect();
            return Err(ToolError::Failed(format!("No hay ninguna ventana con «{query}» en el título. Abiertas: {}.", open.join(", "))));
        }
        many => {
            let titles: Vec<String> = many.iter().take(8).map(|w| format!("«{}»", w.title)).collect();
            return Err(ToolError::Failed(format!("Varias ventanas coinciden con «{query}»: {}. Indica el título completo.", titles.join(", "))));
        }
    };
    input::check_target(&window).map_err(ToolError::NotAllowed)?;
    Ok(window)
}

fn window_arg(args: &Value) -> Result<&str, ToolError> {
    let query = required_str(args, "window")?.trim();
    if query.is_empty() || query.chars().count() > 200 {
        return Err(invalid("«window» debe ser parte del título de una ventana (hasta 200 caracteres)"));
    }
    Ok(query)
}

fn window_property() -> Value {
    json!({ "type": "string", "description": "Parte del título de la ventana de destino (por ejemplo «Bloc de notas»). Usa list_windows para ver los títulos." })
}

/// Recuerda sobre qué ventana se pidió la confirmación: si al ejecutar es otra, no se hace nada.
#[derive(Default)]
pub(crate) struct Previewed(Mutex<Option<isize>>);

impl Previewed {
    pub(crate) fn remember(&self, window: &WindowInfo) {
        *self.0.lock().expect("estado envenenado") = Some(window.hwnd);
    }

    pub(crate) fn verify(&self, window: &WindowInfo) -> Result<(), ToolError> {
        match *self.0.lock().expect("estado envenenado") {
            Some(previous) if previous != window.hwnd => {
                Err(ToolError::Failed("La ventana de destino cambió desde que confirmaste; no hice nada.".into()))
            }
            _ => Ok(()),
        }
    }
}

pub(crate) fn describe_window(window: &WindowInfo) -> String {
    format!("«{}» ({})", window.title, window.process)
}

// ---------------------------------------------------------------------------------------------

pub struct ListWindows;

impl Tool for ListWindows {
    fn name(&self) -> &'static str {
        "list_windows"
    }
    fn description(&self) -> &'static str {
        "Lista las ventanas abiertas (título y programa). Úsala antes de escribir o pulsar algo en otra aplicación, \
         para saber qué título usar."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }
    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &[])
    }
    fn preview(&self, _args: &Value) -> String {
        "Listar las ventanas abiertas".into()
    }
    fn execute(&self, _args: &Value) -> Result<ToolOutput, ToolError> {
        let windows: Vec<WindowInfo> = input::visible_windows().into_iter().take(40).collect();
        let list: Vec<Value> = windows.iter().map(|w| json!({ "title": w.title, "program": w.process, "allowed": input::check_target(w).is_ok() })).collect();
        Ok(ToolOutput { summary: format!("{} ventanas abiertas", windows.len()), data: json!({ "windows": list }) })
    }
}

pub struct ListControls;

impl Tool for ListControls {
    fn name(&self) -> &'static str {
        "list_controls"
    }
    fn description(&self) -> &'static str {
        "Lista los botones, casillas, pestañas, enlaces y campos de texto visibles de una ventana, con su nombre. \
         Úsala para saber qué nombre darle a click_control."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": { "window": window_property() }, "required": ["window"] })
    }
    fn risk(&self, _args: &Value) -> RiskLevel {
        // Leer lo que muestra otra aplicación puede revelar datos privados: se confirma.
        RiskLevel::Confirm
    }
    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &["window"])?;
        resolve(window_arg(args)?).map(|_| ())
    }
    fn preview(&self, args: &Value) -> String {
        match window_arg(args).and_then(resolve) {
            Ok(window) => format!("Leer los botones y campos de la ventana {}", describe_window(&window)),
            Err(error) => error.to_string(),
        }
    }
    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let window = resolve(window_arg(args)?)?;
        let controls = uia::list_controls(window.hwnd).map_err(ToolError::Failed)?;
        let list: Vec<Value> = controls.iter().map(|c| json!({ "name": c.name, "type": c.kind })).collect();
        Ok(ToolOutput { summary: format!("{} controles en «{}»", controls.len(), window.title), data: json!({ "controls": list }) })
    }
}

// ---------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct TypeText(Previewed);

impl TypeText {
    pub fn new() -> Self {
        Self::default()
    }
}

fn type_args(args: &Value) -> Result<(&str, &str), ToolError> {
    check_keys(args, &["window", "text"])?;
    let text = required_str(args, "text")?;
    input::validate_text(text).map_err(invalid)?;
    Ok((window_arg(args)?, text))
}

impl Tool for TypeText {
    fn name(&self) -> &'static str {
        "type_text"
    }
    fn description(&self) -> &'static str {
        "Escribe texto con el teclado en otra aplicación (trae esa ventana al frente y teclea). Solo si el usuario lo \
         pide de forma explícita. No funciona en terminales ni en ventanas de contraseñas. Un salto de línea pulsa Enter."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "window": window_property(),
                "text": { "type": "string", "description": format!("Texto a escribir (hasta {} caracteres).", input::MAX_TYPED_CHARS) }
            },
            "required": ["window", "text"]
        })
    }
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Dangerous
    }
    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        let (query, _) = type_args(args)?;
        resolve(query).map(|_| ())
    }
    fn preview(&self, args: &Value) -> String {
        match type_args(args).and_then(|(query, text)| Ok((resolve(query)?, text))) {
            Ok((window, text)) => {
                self.0.remember(&window);
                format!("Escribir en la ventana {}:\n\n{text}", describe_window(&window))
            }
            Err(error) => error.to_string(),
        }
    }
    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (query, text) = type_args(args)?;
        let window = resolve(query)?;
        self.0.verify(&window)?;
        rate_limit()?;
        input::bring_to_front(&window).map_err(ToolError::Failed)?;
        let typed = input::type_text(&window, text).map_err(ToolError::Failed)?;
        Ok(ToolOutput { summary: format!("Escribí {typed} caracteres en «{}»", window.title), data: json!({ "window": window.title, "characters": typed }) })
    }
}

#[derive(Default)]
pub struct PressKeys(Previewed);

impl PressKeys {
    pub fn new() -> Self {
        Self::default()
    }
}

fn keys_args(args: &Value) -> Result<(&str, input::Combo), ToolError> {
    check_keys(args, &["window", "keys"])?;
    let combo = input::parse_combo(required_str(args, "keys")?).map_err(invalid)?;
    Ok((window_arg(args)?, combo))
}

impl Tool for PressKeys {
    fn name(&self) -> &'static str {
        "press_keys"
    }
    fn description(&self) -> &'static str {
        "Pulsa una tecla o combinación en otra aplicación: enter, tab, esc, flechas, f1-f12, ctrl+c, ctrl+shift+t, \
         alt+left… No admite la tecla Windows, Alt+F4, Ctrl+Alt ni Ctrl+Mayús+Esc. Solo si el usuario lo pide."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "window": window_property(),
                "keys": { "type": "string", "description": "Tecla o combinación separada por «+», por ejemplo «ctrl+s»." }
            },
            "required": ["window", "keys"]
        })
    }
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Dangerous
    }
    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        let (query, _) = keys_args(args)?;
        resolve(query).map(|_| ())
    }
    fn preview(&self, args: &Value) -> String {
        match keys_args(args).and_then(|(query, combo)| Ok((resolve(query)?, combo))) {
            Ok((window, combo)) => {
                self.0.remember(&window);
                format!("Pulsar {} en la ventana {}", combo.display(), describe_window(&window))
            }
            Err(error) => error.to_string(),
        }
    }
    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (query, combo) = keys_args(args)?;
        let window = resolve(query)?;
        self.0.verify(&window)?;
        rate_limit()?;
        input::bring_to_front(&window).map_err(ToolError::Failed)?;
        input::press_combo(&window, &combo).map_err(ToolError::Failed)?;
        Ok(ToolOutput { summary: format!("Pulsé {} en «{}»", combo.display(), window.title), data: json!({ "window": window.title, "keys": combo.display() }) })
    }
}

#[derive(Default)]
pub struct ClickControl(Previewed);

impl ClickControl {
    pub fn new() -> Self {
        Self::default()
    }
}

fn click_args(args: &Value) -> Result<(&str, &str), ToolError> {
    check_keys(args, &["window", "control"])?;
    let control = required_str(args, "control")?.trim();
    if control.is_empty() || control.chars().count() > 100 {
        return Err(invalid("«control» debe ser el nombre de un botón o campo (hasta 100 caracteres)"));
    }
    Ok((window_arg(args)?, control))
}

impl Tool for ClickControl {
    fn name(&self) -> &'static str {
        "click_control"
    }
    fn description(&self) -> &'static str {
        "Pulsa un botón, casilla, pestaña, enlace o campo de otra ventana por su nombre (por ejemplo «Guardar»). \
         Usa list_controls antes para conocer los nombres. Solo si el usuario lo pide de forma explícita."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "window": window_property(),
                "control": { "type": "string", "description": "Nombre del control tal como lo muestra list_controls." }
            },
            "required": ["window", "control"]
        })
    }
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Dangerous
    }
    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        let (query, control) = click_args(args)?;
        let window = resolve(query)?;
        uia::describe(window.hwnd, control).map(|_| ()).map_err(ToolError::Failed)
    }
    fn preview(&self, args: &Value) -> String {
        let result = click_args(args).and_then(|(query, control)| {
            let window = resolve(query)?;
            let found = uia::describe(window.hwnd, control).map_err(ToolError::Failed)?;
            Ok((window, found))
        });
        match result {
            Ok((window, found)) => {
                self.0.remember(&window);
                format!("Pulsar «{}» ({}) en la ventana {}", found.name, found.kind, describe_window(&window))
            }
            Err(error) => error.to_string(),
        }
    }
    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (query, control) = click_args(args)?;
        let window = resolve(query)?;
        self.0.verify(&window)?;
        rate_limit()?;
        input::bring_to_front(&window).map_err(ToolError::Failed)?;
        let pressed = uia::activate(window.hwnd, control).map_err(ToolError::Failed)?;
        Ok(ToolOutput {
            summary: format!("Pulsé «{}» en «{}»", pressed.name, window.title),
            data: json!({ "window": window.title, "control": pressed.name, "type": pressed.kind }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_acciones_son_peligrosas_y_leer_pide_confirmacion() {
        assert_eq!(TypeText::new().risk(&json!({})), RiskLevel::Dangerous);
        assert_eq!(PressKeys::new().risk(&json!({})), RiskLevel::Dangerous);
        assert_eq!(ClickControl::new().risk(&json!({})), RiskLevel::Dangerous);
        assert_eq!(ListControls.risk(&json!({})), RiskLevel::Confirm);
        assert_eq!(ListWindows.risk(&json!({})), RiskLevel::Safe);
    }

    #[test]
    fn los_nombres_coinciden_con_la_lista_del_interruptor() {
        let names = [ListWindows.name(), ListControls.name(), TypeText::new().name(), PressKeys::new().name(), ClickControl::new().name()];
        assert_eq!(names, TOOL_NAMES);
    }

    #[test]
    fn rechaza_argumentos_invalidos_antes_de_tocar_ninguna_ventana() {
        let type_text = TypeText::new();
        for bad in [
            json!({}),
            json!({ "window": "x" }),
            json!({ "text": "hola" }),
            json!({ "window": "x", "text": "" }),
            json!({ "window": "x", "text": "\u{7}" }),
            json!({ "window": "x", "text": "ok", "extra": 1 }),
            json!({ "window": 5, "text": "ok" }),
        ] {
            assert!(matches!(type_text.validate(&bad), Err(ToolError::InvalidArguments(_))), "{bad}");
        }
        let press = PressKeys::new();
        for bad in [json!({ "window": "x", "keys": "win+r" }), json!({ "window": "x", "keys": "alt+f4" }), json!({ "window": "x", "keys": "ctrl+alt+del" }), json!({ "window": "x" })] {
            assert!(matches!(press.validate(&bad), Err(ToolError::InvalidArguments(_))), "{bad}");
        }
        let click = ClickControl::new();
        for bad in [json!({ "window": "x" }), json!({ "window": "x", "control": "  " }), json!({ "window": "x", "control": "a".repeat(101) })] {
            assert!(matches!(click.validate(&bad), Err(ToolError::InvalidArguments(_))), "{bad}");
        }
    }

    #[test]
    fn una_ventana_inexistente_se_explica_sin_actuar() {
        let error = TypeText::new().validate(&json!({ "window": "ventana-que-no-existe-xyz", "text": "hola" })).unwrap_err();
        assert!(matches!(error, ToolError::Failed(ref m) if m.contains("No hay ninguna ventana")), "{error}");
    }

    #[test]
    fn si_cambia_la_ventana_entre_la_confirmacion_y_la_ejecucion_no_se_hace_nada() {
        let previewed = Previewed::default();
        let window = |hwnd| WindowInfo { hwnd, title: "t".into(), process: "notepad".into(), pid: 1 };
        assert!(previewed.verify(&window(1)).is_ok(), "sin vista previa no hay nada que comparar");
        previewed.remember(&window(7));
        assert!(previewed.verify(&window(7)).is_ok());
        assert!(previewed.verify(&window(8)).is_err());
    }

    /// Prueba real con una ventana propia (un formulario con un campo y un botón «Guardar» que escribe el texto en un archivo).
    /// No toca ninguna aplicación del usuario: `cargo test -p zen-desktop -- --ignored --nocapture ventana_de_prueba`
    #[test]
    #[ignore = "abre una ventana y teclea en ella"]
    fn ventana_de_prueba_recibe_texto_teclas_y_clics() {
        let dir = std::env::temp_dir().join("zen-probe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("resultado.txt");
        let script = dir.join("probe.ps1");
        std::fs::write(&script, include_str!("../tests/probe.ps1")).unwrap();
        // Una copia de PowerShell con otro nombre: así el programa de destino no es un terminal bloqueado.
        let host = dir.join("zenprobe.exe");
        std::fs::copy(std::path::Path::new(&std::env::var("SystemRoot").unwrap()).join("System32/WindowsPowerShell/v1.0/powershell.exe"), &host).unwrap();
        let mut child = std::process::Command::new(&host)
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script).arg(&output).spawn().expect("abrir la ventana de prueba");
        let query = "Zen Probe Window";
        let mut window = None;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(250));
            if let Ok(found) = resolve(query) {
                window = Some(found);
                break;
            }
        }
        let window = window.expect("la ventana de prueba debe aparecer");
        println!("ventana: {window:?}");

        let outcome = (|| -> Result<(), ToolError> {
            let controls = ListControls.execute(&json!({ "window": query }))?;
            println!("{}: {}", controls.summary, controls.data);
            let text = "Hola ñandú 😀\nsegunda línea";
            let args = json!({ "window": query, "text": text });
            TypeText::new().validate(&args)?;
            let typed = TypeText::new().execute(&args)?;
            println!("{}", typed.summary);
            PressKeys::new().execute(&json!({ "window": query, "keys": "tab" }))?;
            let click = ClickControl::new().execute(&json!({ "window": query, "control": "Guardar" }))?;
            println!("{}", click.summary);
            std::thread::sleep(Duration::from_millis(600));
            let saved = std::fs::read_to_string(&output).map_err(|e| ToolError::Failed(format!("no se guardó nada: {e}")))?;
            println!("guardado: {saved:?}");
            assert_eq!(saved.trim_start_matches('\u{feff}').replace("\r\n", "\n"), text);
            Ok(())
        })();

        let _ = child.kill();
        let _ = std::process::Command::new("taskkill").args(["/PID", &window.pid.to_string(), "/F"]).output();
        let _ = std::fs::remove_dir_all(&dir);
        outcome.expect("las herramientas deben funcionar sobre la ventana de prueba");
    }

    /// Diagnóstico (solo lectura): qué ve Windows en la ventana de Spotify, si está abierta.
    #[test]
    #[ignore = "diagnóstico manual"]
    fn explora_spotify() {
        let windows = input::visible_windows();
        for w in windows.iter().filter(|w| w.process.contains("spotify")) {
            println!("ventana: {w:?}");
            match uia::list_controls(w.hwnd) {
                Ok(c) => { println!("{} controles", c.len()); for x in c.iter().take(200) { println!("  [{}] {}", x.kind, x.name); } }
                Err(e) => println!("error: {e}"),
            }
        }
        println!("({} ventanas en total)", windows.len());
    }

    #[test]
    fn el_limite_por_minuto_corta_las_rafagas() {
        RECENT_ACTIONS.lock().unwrap().clear();
        for _ in 0..MAX_ACTIONS_PER_MINUTE {
            assert!(rate_limit().is_ok());
        }
        assert!(matches!(rate_limit(), Err(ToolError::NotAllowed(_))));
        RECENT_ACTIONS.lock().unwrap().clear();
    }
}
