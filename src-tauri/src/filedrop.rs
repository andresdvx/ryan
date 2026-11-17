//! Soltar archivos sobre Ryan: arrastrar un archivo al chat o al personaje abre un menú pequeño para analizarlo,
//! resumirlo, preguntarle algo o convertirlo. Soltar es permiso para LEER esos archivos concretos; lo convertido se
//! guarda en la carpeta que el usuario eligió (el Escritorio, si no eligió otra) y nunca pisa un archivo existente.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{AppHandle, DragDropEvent, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use zen_core::assistant::Assistant;
use zen_core::events::Event;
use zen_desktop::incoming::{self, AiAction, FileInfo, Kind};

use crate::settings::Settings;

const MENU_WIDTH: f64 = 380.0;
const MENU_MIN_HEIGHT: f64 = 240.0;
const MENU_MAX_HEIGHT: f64 = 680.0;
const MENU_GAP: i32 = 10;
const MAX_QUESTION_CHARS: usize = 500;

/// Lo último que se soltó y lo último que se convirtió.
#[derive(Default)]
pub struct DropState {
    files: Mutex<Vec<FileInfo>>,
    outputs: Mutex<Vec<PathBuf>>,
}

impl DropState {
    fn files(&self) -> Vec<FileInfo> {
        self.files.lock().expect("archivos envenenado").clone()
    }
}

// ---------------------------------------------------------------------------------------------
// Detectar la soltada
// ---------------------------------------------------------------------------------------------

/// Escucha el arrastre sobre la mascota (`main`) y el chat (`chat`).
pub fn attach(app: &AppHandle) {
    for label in ["main", "chat"] {
        let Some(window) = app.get_webview_window(label) else { continue };
        let app = app.clone();
        window.on_window_event(move |event| {
            let WindowEvent::DragDrop(event) = event else { return };
            match event {
                DragDropEvent::Enter { paths, .. } if !paths.is_empty() => {
                    let _ = app.emit_to(label, "file-drag", true);
                }
                DragDropEvent::Leave => {
                    let _ = app.emit_to(label, "file-drag", false);
                }
                DragDropEvent::Drop { paths, .. } => {
                    let _ = app.emit_to(label, "file-drag", false);
                    let (app, paths) = (app.clone(), paths.clone());
                    std::thread::spawn(move || open_menu(&app, label, paths));
                }
                _ => {}
            }
        });
    }
}

fn open_menu(app: &AppHandle, source: &str, paths: Vec<PathBuf>) {
    let files = match incoming::inspect(&paths) {
        Ok(files) => files,
        Err(error) => return crate::say(app, format!("📎 {error}")),
    };
    let name = if files.len() == 1 { files[0].name.clone() } else { format!("{} archivos", files.len()) };
    *app.state::<DropState>().files.lock().expect("archivos envenenado") = files;
    app.state::<DropState>().outputs.lock().expect("salidas envenenado").clear();
    crate::publish(app, Event::FileDropped { name });
    let Some(menu) = app.get_webview_window("filemenu") else { return };
    let _ = menu.set_size(LogicalSize::new(MENU_WIDTH, 420.0));
    let _ = place_menu(app, source, &menu);
    let _ = menu.show();
    let _ = menu.set_focus();
    let _ = app.emit_to("filemenu", "file-menu-open", menu_json(app));
}

/// El menú aparece junto a donde se soltó: sobre el personaje, o en el centro del chat.
fn place_menu(app: &AppHandle, source: &str, menu: &WebviewWindow) -> tauri::Result<()> {
    let Some(anchor) = app.get_webview_window(source) else { return Ok(()) };
    let Some(monitor) = anchor.current_monitor()? else { return Ok(()) };
    let area = monitor.work_area();
    let size = menu.outer_size()?;
    let (anchor_pos, anchor_size) = (anchor.outer_position()?, anchor.outer_size()?);
    let (min_x, max_x) = (area.position.x, area.position.x + area.size.width as i32 - size.width as i32);
    let (min_y, max_y) = (area.position.y, area.position.y + area.size.height as i32 - size.height as i32);
    let (x, y) = if source == "chat" {
        (anchor_pos.x + (anchor_size.width as i32 - size.width as i32) / 2, anchor_pos.y + (anchor_size.height as i32 - size.height as i32) / 3)
    } else {
        (anchor_pos.x + anchor_size.width as i32 - size.width as i32, anchor_pos.y - size.height as i32 - MENU_GAP)
    };
    menu.set_position(PhysicalPosition::new(x.clamp(min_x, max_x.max(min_x)), y.clamp(min_y, max_y.max(min_y))))
}

// ---------------------------------------------------------------------------------------------
// Qué muestra el menú
// ---------------------------------------------------------------------------------------------

fn desktop() -> PathBuf {
    dirs::desktop_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Dónde se guarda lo convertido: la carpeta elegida si sigue existiendo, o el Escritorio.
fn output_dir(settings: &Settings) -> (PathBuf, bool) {
    match &settings.convert_dir {
        Some(path) if Path::new(path).is_dir() => (PathBuf::from(path), true),
        _ => (desktop(), false),
    }
}

fn output_json(settings: &Settings) -> Value {
    let (dir, custom) = output_dir(settings);
    let label = if custom { dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned()) } else { "Escritorio".to_string() };
    json!({ "label": label, "path": zen_desktop::display_path(&dir), "custom": custom })
}

fn menu_json(app: &AppHandle) -> Value {
    let files = app.state::<DropState>().files();
    let settings = Settings::load();
    let readable: Vec<&FileInfo> = files.iter().filter(|f| incoming::can_ask(f.kind)).collect();
    let provider = settings.provider.display_name();
    let only_images = !readable.is_empty() && readable.iter().all(|f| f.kind == Kind::Image);
    // Un modelo que no ve imágenes lee su texto con el OCR de Windows (en el equipo) y recibe solo ese texto.
    let needs_ocr = !settings.provider.supports_images() && readable.iter().any(|f| f.kind == Kind::Image);
    let (reason, hint) = if readable.is_empty() {
        (Some("No sé leer el contenido de este tipo de archivo, pero sí puedo convertirlo o comprimirlo.".to_string()), None)
    } else if needs_ocr {
        match zen_desktop::ocr::available() {
            Ok(language) => (None, Some(format!("{provider} no ve imágenes: leeré su texto con el OCR de Windows ({language}) y le enviaré solo ese texto."))),
            Err(why) if only_images => (Some(format!("{provider} no puede ver imágenes y este equipo no puede leer su texto: {why}.")), None),
            Err(why) => (None, Some(format!("Las imágenes se saltarán: {provider} no las ve y {why}."))),
        }
    } else {
        (None, None)
    };
    json!({
        "files": files.iter().map(|f| json!({ "name": f.name, "size": f.size, "kind": f.label(), "type": format!("{:?}", f.kind).to_lowercase(), "folder": f.kind == Kind::Folder, "image": f.kind == Kind::Image })).collect::<Vec<_>>(),
        "offers": incoming::offers_for(&files).iter().map(|o| json!({ "id": o.id, "label": o.label, "group": o.group })).collect::<Vec<_>>(),
        "ask": { "enabled": reason.is_none(), "reason": reason, "hint": hint },
        "provider": { "name": settings.provider.display_name(), "local": !settings.provider.needs_key() },
        "output": output_json(&settings),
    })
}

#[tauri::command]
pub fn file_menu_state(app: AppHandle) -> Value {
    menu_json(&app)
}

#[tauri::command]
pub fn file_menu_close(app: AppHandle) {
    if let Some(menu) = app.get_webview_window("filemenu") {
        let _ = menu.hide();
    }
}

/// La página ajusta la altura de la ventana a lo que muestra.
#[tauri::command]
pub fn file_menu_resize(app: AppHandle, height: f64) {
    if let Some(menu) = app.get_webview_window("filemenu") {
        let _ = menu.set_size(LogicalSize::new(MENU_WIDTH, height.clamp(MENU_MIN_HEIGHT, MENU_MAX_HEIGHT)));
    }
}

// ---------------------------------------------------------------------------------------------
// Analizar, resumir y preguntar
// ---------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn file_menu_ai(app: AppHandle, action: String, question: Option<String>) -> Result<(), String> {
    let action = match action.as_str() {
        "analyze" => AiAction::Analyze,
        "summarize" => AiAction::Summarize,
        "ask" => {
            let question = question.unwrap_or_default().trim().to_string();
            if question.is_empty() {
                return Err("Escribe qué quieres saber del archivo.".into());
            }
            if question.chars().count() > MAX_QUESTION_CHARS {
                return Err(format!("La pregunta es muy larga (máximo {MAX_QUESTION_CHARS} caracteres)."));
            }
            AiAction::Ask(question)
        }
        _ => return Err("Acción desconocida.".into()),
    };
    let files = app.state::<DropState>().files();
    if files.is_empty() {
        return Err("Ya no hay ningún archivo en el menú; suéltalo otra vez.".into());
    }
    let vision = Settings::load().provider.supports_images();
    let request = tauri::async_runtime::spawn_blocking(move || incoming::build_ai_request(&files, &action, vision)).await.map_err(|e| e.to_string())??;
    file_menu_close(app.clone());
    std::thread::spawn(move || run_request(&app, request));
    Ok(())
}

/// Muestra la petición en el chat y se la pasa al asistente, igual que un mensaje escrito.
fn run_request(app: &AppHandle, request: incoming::AiRequest) {
    crate::show_chat(app);
    let _ = app.emit("quick-user-message", request.display.clone());
    for note in &request.notes {
        crate::say(app, format!("ℹ️ {note}"));
    }
    let settings = Settings::load();
    let Some(provider) = crate::make_provider(&settings) else {
        let kind = settings.provider;
        crate::say(app, format!("Para hablar conmigo necesito tu API key de {}. Escríbela en la pestaña Ajustes (la consigues en {}).", kind.display_name(), kind.key_url()));
        crate::publish(app, Event::AssistantReplied);
        return;
    };
    // En el historial solo queda la línea corta: el contenido del archivo no se guarda en disco.
    crate::record_chat(app, zen_desktop::conversations::Role::User, &request.display);
    app.state::<Arc<Assistant>>().handle_user_message_with_images(provider.as_ref(), &crate::ChatUi::new(app.clone()), &request.prompt, request.images);
}

// ---------------------------------------------------------------------------------------------
// Convertir
// ---------------------------------------------------------------------------------------------

#[tauri::command]
pub async fn file_menu_convert(app: AppHandle, target: String) -> Result<Value, String> {
    let files = app.state::<DropState>().files();
    if files.is_empty() {
        return Err("Ya no hay ningún archivo en el menú; suéltalo otra vez.".into());
    }
    if !incoming::offers_for(&files).iter().any(|offer| offer.id == target) {
        return Err("Esa conversión no está disponible para lo que soltaste.".into());
    }
    let settings = Settings::load();
    let (dir, custom) = output_dir(&settings);
    let label = output_json(&settings)["label"].as_str().unwrap_or("Escritorio").to_string();
    let outcome = {
        let dir = dir.clone();
        tauri::async_runtime::spawn_blocking(move || zen_desktop::convert::convert(&files, &target, &dir)).await.map_err(|e| e.to_string())??
    };
    let names: Vec<String> = outcome.outputs.iter().map(|p| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())).collect();
    *app.state::<DropState>().outputs.lock().expect("salidas envenenado") = outcome.outputs;
    crate::say(&app, format!("📁 Guardé en {}{}: {}", if custom { "" } else { "el " }, label, names.join(", ")));
    Ok(json!({ "outputs": names, "dir": label, "failures": outcome.failures, "notes": outcome.notes }))
}

/// Deja elegir otra carpeta de salida. Solo esta ruta, que sale del diálogo nativo, se guarda y se usa.
#[tauri::command]
pub async fn file_menu_choose_dir(app: AppHandle) -> Result<Option<Value>, String> {
    let handle = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || handle.dialog().file().set_title("¿Dónde guardo lo que convierta?").blocking_pick_folder()).await.map_err(|e| e.to_string())?;
    let Some(picked) = picked else { return Ok(None) };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    if !path.is_dir() {
        return Err("Esa carpeta no existe.".into());
    }
    let mut settings = Settings::load();
    settings.convert_dir = Some(zen_desktop::display_path(&path));
    settings.save()?;
    if let Some(menu) = app.get_webview_window("filemenu") {
        let _ = menu.set_focus();
    }
    Ok(Some(output_json(&settings)))
}

#[tauri::command]
pub fn file_menu_reset_dir() -> Result<Value, String> {
    let mut settings = Settings::load();
    settings.convert_dir = None;
    settings.save()?;
    Ok(output_json(&settings))
}

/// Muestra en el Explorador lo último que se creó.
#[tauri::command]
pub fn file_menu_reveal(app: AppHandle) -> Result<(), String> {
    let outputs = app.state::<DropState>().outputs.lock().expect("salidas envenenado").clone();
    let Some(first) = outputs.first() else { return Err("Todavía no hay nada convertido.".into()) };
    zen_desktop::reveal_in_explorer(first).map_err(|e| format!("No pude abrir el Explorador: {e}"))
}
