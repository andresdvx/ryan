#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use serde_json::{json, Value};
use zen_ai::ProviderKind;
use zen_core::ai::ToolInvocation;
use zen_core::assistant::{Assistant, AssistantUi, ToolReport};
use zen_core::events::{Event, EventBus};
use zen_core::executor::{PendingAction, ToolExecutor};
use zen_core::tools::{RiskLevel, ToolRegistry};
use zen_desktop::Locations;

use behavior::BehaviorFlags;
use settings::Settings;

mod alarms;
mod behavior;
mod characters;
mod media;
mod music;
mod notify;
mod power;
mod profiles;
mod secrets;
mod system_events;
mod usage;
mod settings;

const MARGIN: i32 = 24;
const PET_SIZE: f64 = 90.0;
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(300);
const CHAT_GAP: i32 = 8;
const CHAT_MAX_SIZE: (f64, f64) = (620.0, 720.0);
const CHAT_MIN_SIZE: (f64, f64) = (380.0, 460.0);
const CHAT_WIDTH_RATIO: f64 = 0.32;
const CHAT_HEIGHT_RATIO: f64 = 0.78;

/// Coloca la mascota en la esquina inferior derecha del área de trabajo (sin tapar la barra de tareas).
fn place_in_corner(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = window.primary_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let x = area.position.x + area.size.width as i32 - size.width as i32 - MARGIN;
        let y = area.position.y + area.size.height as i32 - size.height as i32 - MARGIN;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

/// Ajusta el tamaño del chat a la pantalla: ocupa una fracción del área de trabajo, sin pasar del tamaño
/// pensado para un monitor grande ni encogerse hasta ser incómodo.
fn fit_chat_to_monitor(chat: &WebviewWindow, area_width: u32, area_height: u32, scale: f64) -> tauri::Result<()> {
    let logical_width = area_width as f64 / scale;
    let logical_height = area_height as f64 / scale;
    let width = (logical_width * CHAT_WIDTH_RATIO).clamp(CHAT_MIN_SIZE.0, CHAT_MAX_SIZE.0);
    let height = (logical_height * CHAT_HEIGHT_RATIO).clamp(CHAT_MIN_SIZE.1, CHAT_MAX_SIZE.1);
    chat.set_size(LogicalSize::new(width.round(), height.round()))
}

/// Coloca el chat encima de la mascota, alineado a su borde derecho y dentro del área de trabajo.
fn place_chat_near_pet(pet: &WebviewWindow, chat: &WebviewWindow) -> tauri::Result<()> {
    let Some(monitor) = pet.current_monitor()? else {
        return Ok(());
    };
    let area = monitor.work_area();
    fit_chat_to_monitor(chat, area.size.width, area.size.height, monitor.scale_factor())?;
    let pet_pos = pet.outer_position()?;
    let pet_size = pet.outer_size()?;
    let chat_size = chat.outer_size()?;

    let min_x = area.position.x;
    let max_x = area.position.x + area.size.width as i32 - chat_size.width as i32;
    let min_y = area.position.y;
    let max_y = area.position.y + area.size.height as i32 - chat_size.height as i32;

    let x = (pet_pos.x + pet_size.width as i32 - chat_size.width as i32).clamp(min_x, max_x.max(min_x));
    let y = (pet_pos.y - chat_size.height as i32 - CHAT_GAP).clamp(min_y, max_y.max(min_y));
    chat.set_position(PhysicalPosition::new(x, y))
}

fn publish(app: &AppHandle, event: Event) {
    app.state::<Arc<EventBus>>().publish(event);
}

fn show_chat(app: &AppHandle) {
    show_chat_with(app, true);
}

/// Con `focus` falso el chat aparece sin quitarle el teclado a lo que el usuario esté haciendo
/// (lo usan los avisos de recordatorios).
fn show_chat_with(app: &AppHandle, focus: bool) {
    let (Some(pet), Some(chat)) = (app.get_webview_window("main"), app.get_webview_window("chat")) else {
        return;
    };
    let _ = place_chat_near_pet(&pet, &chat);
    let _ = chat.show();
    if focus {
        let _ = chat.set_focus();
    }
    publish(app, Event::ChatOpened);
}

fn hide_chat_window(app: &AppHandle) {
    if let Some(chat) = app.get_webview_window("chat") {
        let _ = chat.hide();
        publish(app, Event::ChatClosed);
    }
}

fn toggle_chat(app: &AppHandle) {
    match app.get_webview_window("chat") {
        Some(chat) if chat.is_visible().unwrap_or(false) => hide_chat_window(app),
        Some(_) => show_chat(app),
        None => {}
    }
}

#[tauri::command]
fn toggle_chat_cmd(app: AppHandle) {
    publish(&app, Event::CharacterClicked);
    toggle_chat(&app);
}

#[tauri::command]
fn hide_chat(app: AppHandle) {
    hide_chat_window(&app);
}

// ---------------------------------------------------------------------------------------------
// Barra rápida: un campo pequeño sobre el escritorio, con su propio atajo, para pedir algo sin abrir el chat.
// ---------------------------------------------------------------------------------------------

const QUICK_WIDTH: f64 = 640.0;
const QUICK_MIN_HEIGHT: f64 = 90.0;
const QUICK_MAX_HEIGHT: f64 = 560.0;
/// A qué altura del área de trabajo aparece la barra (como una fracción desde arriba).
const QUICK_TOP_RATIO: f64 = 0.16;

/// Atajo global de la barra rápida registrado ahora mismo y su texto.
#[derive(Default)]
struct ActiveQuickShortcut(Mutex<Option<(Shortcut, String)>>);

fn position_quick(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(monitor) = window.primary_monitor()? {
        let area = monitor.work_area();
        let size = window.outer_size()?;
        let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
        let y = area.position.y + (area.size.height as f64 * QUICK_TOP_RATIO) as i32;
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

fn show_quick(app: &AppHandle) {
    let Some(quick) = app.get_webview_window("quick") else { return };
    let _ = position_quick(&quick);
    let _ = quick.show();
    let _ = quick.set_focus();
    // La página se limpia y se pone a escribir.
    let _ = app.emit("quick-opened", ());
}

fn hide_quick(app: &AppHandle) {
    if let Some(quick) = app.get_webview_window("quick") {
        let _ = quick.hide();
    }
}

fn toggle_quick(app: &AppHandle) {
    match app.get_webview_window("quick") {
        Some(quick) if quick.is_visible().unwrap_or(false) => hide_quick(app),
        Some(_) => show_quick(app),
        None => {}
    }
}

#[tauri::command]
fn hide_quick_cmd(app: AppHandle) {
    hide_quick(&app);
}

/// La página ajusta la altura de la ventana a lo que muestra (la ventana es transparente: no debe tapar nada de más).
#[tauri::command]
fn resize_quick(app: AppHandle, height: f64) {
    if let Some(quick) = app.get_webview_window("quick") {
        let _ = quick.set_size(LogicalSize::new(QUICK_WIDTH, height.clamp(QUICK_MIN_HEIGHT, QUICK_MAX_HEIGHT)));
    }
}

/// Lo escrito en la barra rápida sigue el mismo camino que el chat; el chat (aunque esté oculto) lo muestra como suyo.
#[tauri::command]
fn quick_submit(app: AppHandle, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let _ = app.emit("quick-user-message", text.clone());
    std::thread::spawn(move || handle_chat_text(&app, &text, Vec::new()));
}

#[tauri::command]
fn open_chat_from_quick(app: AppHandle) {
    hide_quick(&app);
    show_chat(&app);
}

/// Cambia el atajo de la barra rápida: registra el nuevo antes de soltar el anterior.
#[tauri::command]
fn set_quick_shortcut(app: AppHandle, shortcut: String) -> Result<(), String> {
    let new = parse_shortcut(&shortcut)?;
    if app.state::<ActiveShortcut>().0.lock().expect("atajo envenenado").as_ref().is_some_and(|(chat, _)| *chat == new) {
        return Err("Ese atajo ya abre el chat: elige otro distinto.".into());
    }
    let state = app.state::<ActiveQuickShortcut>();
    let mut active = state.0.lock().expect("atajo envenenado");
    if active.as_ref().is_some_and(|(current, _)| *current == new) {
        let mut settings = Settings::load();
        settings.quick_shortcut = shortcut.clone();
        settings.save()?;
        *active = Some((new, shortcut));
        return Ok(());
    }
    app.global_shortcut()
        .register(new.clone())
        .map_err(|_| "No se pudo registrar ese atajo: probablemente Windows u otra aplicación ya lo usa.".to_string())?;
    if let Some((old, _)) = active.take() {
        let _ = app.global_shortcut().unregister(old);
    }
    *active = Some((new, shortcut.clone()));
    let mut settings = Settings::load();
    settings.quick_shortcut = shortcut;
    settings.save()
}

/// Registra el atajo guardado de la barra rápida; si no se puede, la barra sigue disponible desde el chat.
fn register_quick_shortcut(app: &AppHandle, wanted: &str) {
    let Ok(shortcut) = parse_shortcut(wanted) else { return };
    let chat_uses_it = app.state::<ActiveShortcut>().0.lock().expect("atajo envenenado").as_ref().is_some_and(|(chat, _)| *chat == shortcut);
    if chat_uses_it {
        eprintln!("El atajo {wanted} ya abre el chat; la barra rápida queda sin atajo");
        return;
    }
    if app.global_shortcut().register(shortcut.clone()).is_ok() {
        *app.state::<ActiveQuickShortcut>().0.lock().expect("atajo envenenado") = Some((shortcut, wanted.to_string()));
    } else {
        eprintln!("El atajo {wanted} no está disponible; la barra rápida queda sin atajo");
    }
}

/// Atajo global registrado ahora mismo (puede ser el de respaldo) y su texto.
#[derive(Default)]
struct ActiveShortcut(Mutex<Option<(Shortcut, String)>>);

/// Confirmaciones del usuario pendientes: el hilo del asistente espera en el receptor.
#[derive(Default)]
struct ConfirmationWaiters {
    next_id: AtomicU64,
    senders: Mutex<HashMap<u64, Sender<bool>>>,
}

impl ConfirmationWaiters {
    fn add(&self, sender: Sender<bool>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.senders.lock().expect("esperas envenenado").insert(id, sender);
        id
    }

    fn resolve(&self, id: u64, approve: bool) {
        if let Some(sender) = self.senders.lock().expect("esperas envenenado").remove(&id) {
            let _ = sender.send(approve);
        }
    }

    fn discard(&self, id: u64) {
        self.senders.lock().expect("esperas envenenado").remove(&id);
    }
}

fn say(app: &AppHandle, text: impl Into<String>) {
    let text = text.into();
    #[cfg(debug_assertions)]
    println!("chat: {text}");
    let _ = app.emit("chat-log", text);
}

fn risk_name(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Safe => "safe",
        RiskLevel::Confirm => "confirm",
        RiskLevel::Dangerous => "dangerous",
    }
}

/// Puente entre el asistente y el chat.
struct ChatUi {
    app: AppHandle,
    /// Hay una respuesta en directo a medias: el siguiente `say` la completa en vez de añadir otro mensaje.
    streaming: std::sync::atomic::AtomicBool,
}

impl ChatUi {
    fn new(app: AppHandle) -> Self {
        Self { app, streaming: std::sync::atomic::AtomicBool::new(false) }
    }

    /// ¿Había una respuesta en directo abierta? (y la deja cerrada)
    fn take_streaming(&self) -> bool {
        self.streaming.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

impl AssistantUi for ChatUi {
    fn say(&self, text: &str) {
        record_chat(&self.app, zen_desktop::conversations::Role::Assistant, text);
        if self.take_streaming() {
            // El mensaje completo sustituye al que se fue escribiendo (ahora ya con formato).
            let _ = self.app.emit("chat-final", text);
        } else {
            say(&self.app, text);
        }
    }

    /// Lo que hizo una herramienta: la línea del chat con su detalle desplegable, el historial y el registro de actividad.
    fn say_tool(&self, text: &str, report: &ToolReport) {
        record_chat(&self.app, zen_desktop::conversations::Role::Assistant, text);
        let prepared = zen_desktop::activity::prepare(report);
        let at = zen_desktop::unix_now();
        // Con el historial apagado no se guarda nada (y sin entrada guardada no hay nada que deshacer).
        let saved = if Settings::load().save_history {
            match (self.app.try_state::<Arc<zen_desktop::activity::Activity>>(), self.app.try_state::<Arc<Locations>>()) {
                (Some(store), Some(locations)) => {
                    let undo = zen_desktop::activity::undo_for(&locations, report);
                    store.record(report, undo, at).ok()
                }
                _ => None,
            }
        } else {
            None
        };
        let _ = self.app.emit(
            "chat-tool",
            json!({
                "text": text, "id": saved.as_ref().map(|e| e.id), "tool": report.tool, "ok": report.ok, "summary": prepared.summary,
                "args": prepared.args, "data": prepared.data, "duration_ms": report.duration_ms, "confirmed": report.confirmed, "at": at,
                "undo": saved.as_ref().and_then(|e| e.undo.as_ref()).map(|u| u.label.clone()),
            }),
        );
        let _ = self.app.emit("activity-changed", ());
    }

    fn stream_text(&self, delta: &str) {
        self.streaming.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.app.emit("chat-stream", delta);
    }

    fn stream_abort(&self) {
        if self.take_streaming() {
            let _ = self.app.emit("chat-stream-abort", ());
        }
    }

    fn confirm(&self, action: &PendingAction) -> bool {
        // La confirmación vive en el chat: si la petición salió de la barra rápida, se pasa al chat para que la vea.
        hide_quick(&self.app);
        show_chat(&self.app);
        let (sender, receiver) = mpsc::channel();
        let waiters = self.app.state::<ConfirmationWaiters>();
        let id = waiters.add(sender);
        let _ = self.app.emit(
            "confirmation-request",
            json!({
                "id": id,
                "tool": action.tool(),
                "preview": action.preview(),
                "risk": risk_name(action.risk()),
                "detailed": action.detailed(),
            }),
        );
        match receiver.recv_timeout(CONFIRMATION_TIMEOUT) {
            Ok(approved) => approved,
            Err(_) => {
                waiters.discard(id);
                say(&self.app, "Se agotó el tiempo para confirmar; cancelé la acción.");
                false
            }
        }
    }

    fn assistant_name(&self) -> String {
        Settings::load().assistant_name
    }

    /// La IA no tiene reloj: sin esto no podría convertir «a las 17:30» o «dentro de un rato» en algo exacto.
    fn context(&self) -> String {
        let now = zen_desktop::LocalNow::now();
        let time = format!("Fecha y hora local del usuario: {}.", now.describe());
        // Cuánto llevas trabajando es un dato de uso: si apagaste los comentarios sobre el equipo, no se usa.
        let worked_minutes = match self.app.try_state::<usage::Activity>() {
            Some(activity) if Settings::load_effective().usage_comments => activity.worked_minutes(),
            _ => 0,
        };
        let mood = zen_desktop::mood::mood_section(zen_desktop::mood::MoodInput {
            hour: now.hour,
            battery: zen_desktop::power::read_battery(),
            worked_minutes,
        });
        // El perfil activo (Trabajo, Descanso…) cambia el tono con el que responde la IA.
        let tone = Settings::load().profile().tone;
        let mood = if tone.is_empty() { mood } else { format!("{mood}\n{tone}") };
        let time = format!("{time}

{mood}");
        match self.app.try_state::<Arc<zen_desktop::Memory>>() {
            Some(memory) if !memory.prompt_section().is_empty() => format!("{time}

{}", memory.prompt_section()),
            _ => time,
        }
    }
}

/// El proveedor de IA elegido, listo para usar. `None` si es un servicio en la nube y falta su API key.
fn make_provider(settings: &Settings) -> Option<Box<dyn zen_core::ai::LlmProvider>> {
    let kind = settings.provider;
    let api_key = if kind.needs_key() { secrets::api_key(kind)? } else { String::new() };
    let local = zen_ai::OllamaSettings { url: settings.ollama_url.clone(), think: settings.ollama_think };
    Some(kind.create(api_key, settings.model(kind), &local))
}

fn handle_key_command(app: &AppHandle, key: &str) {
    let kind = Settings::load().provider;
    if !kind.needs_key() {
        say(app, format!("{} no usa API key: funciona en tu equipo. Si no responde, revisa que Ollama esté abierto y la dirección en Ajustes > IA.", kind.display_name()));
        return;
    }
    if key.is_empty() {
        say(app, format!("Uso: /key TU_API_KEY. Se guarda para {} (la consigues en {}). También puedes usar la pestaña Ajustes.", kind.display_name(), kind.key_url()));
        return;
    }
    match secrets::save_key(kind, key) {
        Ok(()) => say(app, format!("Guardé tu API key de {} en el Administrador de credenciales de Windows. Ya puedes hablar conmigo.", kind.display_name())),
        Err(error) => say(app, format!("No pude guardar la API key: {error}")),
    }
}
/// Comandos de desarrollo: usan el mismo camino de seguridad que la IA (`Assistant::run_tool`).
fn handle_tool_command(app: &AppHandle, rest: &str) {
    let assistant = app.state::<Arc<Assistant>>();
    let rest = rest.trim();
    let (name, json_text) = match rest.split_once(char::is_whitespace) {
        Some((name, json_text)) => (name, json_text.trim()),
        None => (rest, "{}"),
    };
    publish(app, Event::UserMessageReceived { text: format!("/tool {name}") });
    match serde_json::from_str::<Value>(json_text) {
        Ok(args) => {
            let call = ToolInvocation { id: None, name: name.to_string(), args };
            assistant.run_tool(&call, &ChatUi::new(app.clone()));
        }
        Err(error) => {
            say(app, format!("El JSON de los argumentos no es válido: {error}"));
            publish(app, Event::AssistantReplied);
        }
    }
}

fn handle_chat_text(app: &AppHandle, text: &str, images: Vec<zen_core::ai::Image>) {
    let text = text.trim();
    // Con imágenes y sin texto, se pide lo más natural: que las describa.
    let default_text = "Mira esta imagen y dime qué ves.";
    let text = if text.is_empty() && !images.is_empty() { default_text } else { text };

    // La clave nunca se registra ni se publica como evento.
    if text == "/key" {
        return handle_key_command(app, "");
    }
    if let Some(key) = text.strip_prefix("/key ") {
        return handle_key_command(app, key.trim());
    }
    if text == "/forget" {
        let kind = Settings::load().provider;
        match secrets::forget_key(kind) {
            Ok(()) => say(app, format!("Borré tu API key guardada de {}.", kind.display_name())),
            Err(error) => say(app, format!("No pude borrar la API key: {error}")),
        }
        return;
    }

    #[cfg(debug_assertions)]
    println!("recibido: {text}");

    if text == "/new" {
        app.state::<Arc<Assistant>>().clear_history();
        // Lo que se escriba a partir de ahora va a una conversación nueva del historial.
        if let Some(store) = app.try_state::<Arc<zen_desktop::conversations::Conversations>>() {
            store.start_new();
        }
        say(app, "Empecemos de cero: olvidé la conversación anterior.");
        return;
    }
    if text == "/tools" {
        let executor = app.state::<Arc<ToolExecutor>>();
        let lines: Vec<String> = executor
            .specs()
            .iter()
            .map(|spec| format!("• {}: {}\n  {}", spec.name, spec.description, spec.parameters))
            .collect();
        say(app, lines.join("\n"));
        return;
    }
    if let Some(rest) = text.strip_prefix("/tool ") {
        return handle_tool_command(app, rest);
    }

    let settings = Settings::load();
    let kind = settings.provider;
    let Some(provider) = make_provider(&settings) else {
        publish(app, Event::UserMessageReceived { text: text.to_string() });
        say(app, format!("Para hablar conmigo necesito tu API key de {}. Escríbela en la pestaña Ajustes (la consigues en {}); se guarda en el Administrador de credenciales de Windows.", kind.display_name(), kind.key_url()));
        publish(app, Event::AssistantReplied);
        return;
    };
    // Un mensaje que parece llevar una contraseña o una clave no se guarda en el historial.
    let stored = if zen_desktop::memory::looks_sensitive(text) {
        "🔒 Mensaje no guardado en el historial: parecía contener una contraseña o una clave."
    } else {
        text
    };
    let stored = match images.len() {
        0 => stored.to_string(),
        1 => format!("{stored}\n📎 1 imagen adjunta"),
        n => format!("{stored}\n📎 {n} imágenes adjuntas"),
    };
    record_chat(app, zen_desktop::conversations::Role::User, &stored);
    app.state::<Arc<Assistant>>().handle_user_message_with_images(provider.as_ref(), &ChatUi::new(app.clone()), text, images);
}

/// Guarda en el historial lo que se ve en el chat, si el usuario no lo apagó en Ajustes.
fn record_chat(app: &AppHandle, role: zen_desktop::conversations::Role, text: &str) {
    if !Settings::load().save_history {
        return;
    }
    if let Some(store) = app.try_state::<Arc<zen_desktop::conversations::Conversations>>() {
        let _ = store.record(role, text);
    }
}

/// `images`: lo que el usuario adjuntó, como `[{ "mime": "image/png", "data": "<base64>" }]`.
#[tauri::command]
fn submit_message(app: AppHandle, text: String, images: Option<Vec<Value>>) {
    let pairs: Vec<(String, String)> = images
        .unwrap_or_default()
        .iter()
        .map(|item| (item["mime"].as_str().unwrap_or("").to_string(), item["data"].as_str().unwrap_or("").to_string()))
        .collect();
    std::thread::spawn(move || match zen_desktop::attachments::validate_images(&pairs) {
        Ok(images) => handle_chat_text(&app, &text, images),
        Err(error) => say(&app, format!("No pude adjuntar la imagen: {error}")),
    });
}

/// Ventanas que se pueden mirar con el botón «mirar una ventana» (sin terminales, contraseñas ni pagos).
#[tauri::command]
async fn list_capturable_windows() -> Result<Vec<Value>, String> {
    let windows = tauri::async_runtime::spawn_blocking(zen_desktop::capturable_windows).await.map_err(|e| e.to_string())?;
    Ok(windows.into_iter().map(|(title, process)| json!({ "title": title, "process": process })).collect())
}

/// Captura una ventana a petición del usuario y la devuelve para que la vea antes de enviarla. Nada se envía
/// ni se guarda aquí: la imagen solo viaja si el usuario la adjunta a un mensaje.
#[tauri::command]
async fn capture_window_preview(title: Option<String>) -> Result<Value, String> {
    let capture = tauri::async_runtime::spawn_blocking(move || zen_desktop::capture_for_user(title.as_deref()))
        .await
        .map_err(|e| e.to_string())??;
    Ok(json!({ "title": capture.title, "process": capture.process, "width": capture.width, "height": capture.height, "mime": "image/png", "data": capture.base64 }))
}

type Store<'a> = tauri::State<'a, Arc<zen_desktop::conversations::Conversations>>;

fn summary_json(s: &zen_desktop::conversations::Summary) -> Value {
    json!({ "id": s.id, "title": s.title, "updated": s.updated, "count": s.count, "preview": s.preview, "snippet": s.snippet })
}

fn conversation_json(c: &zen_desktop::conversations::Conversation) -> Value {
    use zen_desktop::conversations::Role;
    let messages: Vec<Value> = c
        .messages
        .iter()
        .map(|m| json!({ "role": if m.role == Role::User { "user" } else { "assistant" }, "text": m.text, "at": m.at }))
        .collect();
    json!({ "id": c.id, "title": c.title, "created": c.created, "updated": c.updated, "messages": messages })
}

/// Conversaciones guardadas, las más recientes primero; con `query`, solo donde aparece ese texto.
#[tauri::command]
fn list_conversations(store: Store, query: Option<String>) -> Vec<Value> {
    let found = match query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(query) => store.search(query),
        None => store.list(),
    };
    found.iter().map(summary_json).collect()
}

#[tauri::command]
fn load_conversation(store: Store, id: u64) -> Result<Value, String> {
    store.load(id).map(|c| conversation_json(&c)).ok_or_else(|| "Esa conversación ya no existe.".into())
}

/// Abre una conversación para continuarla: la IA recupera lo hablado y los mensajes nuevos se añaden a ella.
#[tauri::command]
fn resume_conversation(app: AppHandle, store: Store, id: u64) -> Result<Value, String> {
    let conversation = store.resume(id).ok_or("Esa conversación ya no existe.")?;
    let turns = zen_desktop::conversations::restorable(&conversation)
        .into_iter()
        .map(|(role, text)| (role == zen_desktop::conversations::Role::User, text))
        .collect();
    app.state::<Arc<Assistant>>().restore_history(turns);
    Ok(conversation_json(&conversation))
}

#[tauri::command]
fn delete_conversation(store: Store, id: u64) -> bool {
    store.delete(id)
}

#[tauri::command]
fn clear_conversations(store: Store) {
    store.clear();
}

#[tauri::command]
fn set_save_history(value: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.save_history = value;
    settings.save()
}

/// Batería, memoria y disco para la pestaña «Hoy».
#[tauri::command]
fn get_system_status() -> Value {
    zen_desktop::system_status()
}

// ---------------------------------------------------------------------------------------------
// Registro de actividad
// ---------------------------------------------------------------------------------------------

type ActivityStore<'a> = tauri::State<'a, Arc<zen_desktop::activity::Activity>>;

fn activity_json(entry: &zen_desktop::activity::Entry) -> Value {
    json!({
        "id": entry.id, "at": entry.at, "tool": entry.tool, "ok": entry.ok, "summary": entry.summary, "args": entry.args, "data": entry.data,
        "duration_ms": entry.duration_ms, "confirmed": entry.confirmed, "undone": entry.undone,
        "undo": entry.undo.as_ref().map(|u| u.label.clone()),
    })
}

/// Lo que ha hecho el asistente, lo más reciente primero. Antes de terminar el arranque no hay registro: lista vacía.
#[tauri::command]
fn list_activity(app: AppHandle) -> Vec<Value> {
    app.try_state::<Arc<zen_desktop::activity::Activity>>().map(|store| store.list().iter().map(activity_json).collect()).unwrap_or_default()
}

/// Las únicas herramientas con las que se puede deshacer algo. El registro vive en un archivo: aunque alguien lo
/// editara, deshacer nunca podría llamar a una herramienta distinta de estas (y todas piden confirmación como siempre).
const UNDO_TOOLS: [&str; 7] = ["move_file", "delete_file", "undo_organize", "cancel_reminder", "forget", "delete_task", "complete_task"];

/// Deshace una acción del registro llamando a la herramienta contraria por el camino de seguridad normal: la
/// confirmación aparece en el chat. El resultado llega como un mensaje más y la entrada se marca como deshecha.
#[tauri::command]
fn undo_activity(app: AppHandle, store: ActivityStore, id: u64) -> Result<(), String> {
    let entry = store.get(id).ok_or("Esa acción ya no está en el registro.")?;
    if entry.undone {
        return Err("Esa acción ya se deshizo.".into());
    }
    let undo = entry.undo.ok_or("Esa acción no se puede deshacer.")?;
    if !UNDO_TOOLS.contains(&undo.tool.as_str()) {
        return Err("Esa acción no se puede deshacer.".into());
    }
    std::thread::spawn(move || {
        let call = ToolInvocation { id: None, name: undo.tool, args: undo.args };
        let result = app.state::<Arc<Assistant>>().run_tool(&call, &ChatUi::new(app.clone()));
        if matches!(result, zen_core::ai::Message::ToolResult { is_error: false, .. }) {
            if let Some(store) = app.try_state::<Arc<zen_desktop::activity::Activity>>() {
                store.mark_undone(id);
            }
            let _ = app.emit("activity-changed", ());
        }
    });
    Ok(())
}

#[tauri::command]
fn clear_activity(store: ActivityStore) {
    store.clear();
}

/// Muestra en el Explorador algo que hizo el asistente. Solo si está dentro de las carpetas permitidas y existe.
#[tauri::command]
fn show_in_explorer(locations: tauri::State<Arc<Locations>>, path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(path);
    if !path.exists() || !locations.contains(&path) {
        return Err("Eso ya no está ahí.".into());
    }
    zen_desktop::reveal_in_explorer(&path).map_err(|error| format!("No pude abrir el Explorador: {error}"))
}

/// Abre la Papelera de Windows (para restaurar algo que se borró).
#[tauri::command]
fn open_recycle_bin() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("shell:RecycleBinFolder")
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("No pude abrir la Papelera: {error}"))
}

/// Recordatorios pendientes para la pestaña Recordatorios, del que vence antes al que vence después.
#[tauri::command]
fn list_reminders(app: AppHandle) -> Value {
    // Antes de terminar el arranque todavía no hay almacén: se responde con una lista vacía.
    let pending = app.try_state::<Arc<zen_desktop::Reminders>>().map(|reminders| reminders.list()).unwrap_or_default();
    let items: Vec<Value> = pending.iter().map(|r| json!({ "id": r.id, "text": r.text, "due": r.due })).collect();
    json!({ "items": items, "max": zen_desktop::MAX_REMINDERS, "now": zen_desktop::unix_now() })
}

/// Crea un recordatorio: `minutes` (dentro de cuánto) o `at` (hora HH:MM, 24 h), una de las dos.
#[tauri::command]
fn add_reminder(reminders: tauri::State<Arc<zen_desktop::Reminders>>, text: String, minutes: Option<u64>, at: Option<String>) -> Result<String, String> {
    let args = match (minutes, at) {
        (Some(minutes), None) => json!({ "text": text, "delay_seconds": minutes.saturating_mul(60) }),
        (None, Some(at)) => json!({ "text": text, "at": at }),
        _ => return Err("Indica dentro de cuántos minutos o a qué hora.".into()),
    };
    reminders.create(&args).map(|(_, when)| when)
}

#[tauri::command]
fn cancel_reminder(reminders: tauri::State<Arc<zen_desktop::Reminders>>, id: u64) -> bool {
    reminders.cancel(id).is_some()
}

fn tasks_store(app: &AppHandle) -> Result<Arc<zen_desktop::Tasks>, String> {
    app.try_state::<Arc<zen_desktop::Tasks>>().map(|tasks| tasks.inner().clone()).ok_or_else(|| "Las tareas aún no están listas; inténtalo de nuevo.".to_string())
}

/// Lista de tareas para la pestaña Pendientes, en el orden en que se agregaron.
#[tauri::command]
fn list_tasks(app: AppHandle) -> Value {
    let items = app.try_state::<Arc<zen_desktop::Tasks>>().map(|tasks| tasks.list()).unwrap_or_default();
    let items: Vec<Value> = items.iter().map(|t| json!({ "id": t.id, "text": t.text, "done": t.done })).collect();
    json!({ "items": items })
}

#[tauri::command]
fn add_task(app: AppHandle, text: String) -> Result<(), String> {
    tasks_store(&app)?.add(&text).map(|_| ())
}

#[tauri::command]
fn set_task_done(app: AppHandle, id: u64, done: bool) -> Result<(), String> {
    tasks_store(&app)?.set_done(id, done).map(|_| ()).ok_or_else(|| "Esa tarea ya no existe.".to_string())
}

#[tauri::command]
fn delete_task(app: AppHandle, id: u64) -> Result<(), String> {
    tasks_store(&app)?.remove(id).map(|_| ()).ok_or_else(|| "Esa tarea ya no existe.".to_string())
}

/// Quita de la lista todas las tareas ya hechas; devuelve cuántas.
#[tauri::command]
fn clear_done_tasks(app: AppHandle) -> Result<usize, String> {
    Ok(tasks_store(&app)?.clear_done())
}

fn provider_kind(id: &str) -> Result<ProviderKind, String> {
    ProviderKind::from_id(id).ok_or_else(|| format!("Proveedor desconocido: {id}"))
}

/// Estado de los ajustes para la pestaña Ajustes. Las claves nunca se devuelven, solo su origen.
#[tauri::command]
/// Puede llamarse antes de que `setup` termine de registrar los estados (la página carga en paralelo): en ese caso
/// se responde con un error y la interfaz lo reintenta, en vez de hacer fallar la aplicación.
fn get_settings(app: AppHandle) -> Result<Value, String> {
    let (Some(executor), Some(chat_shortcut), Some(quick_shortcut)) =
        (app.try_state::<Arc<ToolExecutor>>(), app.try_state::<ActiveShortcut>(), app.try_state::<ActiveQuickShortcut>())
    else {
        return Err("La aplicación aún está arrancando.".into());
    };
    let settings = Settings::load();
    let providers: Vec<Value> = ProviderKind::ALL
        .iter()
        .map(|&kind| {
            json!({
                "id": kind.id(),
                "name": kind.display_name(),
                "model": settings.custom_model(kind).unwrap_or(""),
                "default_model": kind.default_model(),
                "key_source": secrets::key_source(kind),
                "key_url": kind.key_url(),
                "env_var": kind.env_var(),
                "needs_key": kind.needs_key(),
                "vision": kind.supports_images(),
            })
        })
        .collect();
    let tools: Vec<Value> = executor
        .all_with_risk()
        .iter()
        .map(|(spec, risk)| {
            json!({ "name": spec.name, "description": spec.description, "enabled": settings.is_tool_enabled(&spec.name), "risk": risk_name(*risk) })
        })
        .collect();
    let active_shortcut = chat_shortcut.0.lock().expect("atajo envenenado").as_ref().map(|(_, text)| text.clone());
    Ok(json!({
        "provider": settings.provider.id(),
        "providers": providers,
        "ollama_url": settings.ollama_url,
        "ollama_think": settings.ollama_think,
        "save_history": settings.save_history,
        "profile": settings.profile,
        "profiles": profiles::PROFILES
            .iter()
            .map(|p| json!({ "id": p.id, "name": p.name, "icon": p.icon, "description": p.description, "changes": p.changes() }))
            .collect::<Vec<Value>>(),
        "default_ollama_url": zen_ai::DEFAULT_OLLAMA_URL,
        "shortcut": settings.shortcut,
        "shortcut_active": active_shortcut,
        "quick_shortcut": settings.quick_shortcut,
        "quick_shortcut_active": quick_shortcut.0.lock().expect("atajo envenenado").as_ref().map(|(_, text)| text.clone()),
        "default_shortcut": settings::DEFAULT_SHORTCUT,
        "autostart": app.autolaunch().is_enabled().unwrap_or(false),
        "confirm_all": settings.confirm_all,
        "tools": tools,
        "wander": settings.wander,
        "look_at_cursor": settings.look_at_cursor,
        "character": settings.character,
        "assistant_name": settings.assistant_name,
        "sound_enabled": settings.sound_enabled,
        "sound_volume": settings.sound_volume,
        "battery_alerts": settings.battery_alerts,
        "system_events": settings.system_events,
        "computer_control": settings.computer_control,
        "memory_enabled": settings.memory_enabled,
        "usage_comments": settings.usage_comments,
        "music_reaction": settings.music_reaction,
    }))
}
#[tauri::command]
fn set_provider(app: AppHandle, id: String) -> Result<(), String> {
    let kind = provider_kind(&id)?;
    let mut settings = Settings::load();
    if settings.provider != kind {
        settings.provider = kind;
        settings.save()?;
        // Cada proveedor guarda sus respuestas en un formato propio: se empieza una conversación nueva.
        app.state::<Arc<Assistant>>().clear_history();
    }
    Ok(())
}

#[tauri::command]
fn set_model(provider: String, model: String) -> Result<(), String> {
    let kind = provider_kind(&provider)?;
    let mut settings = Settings::load();
    settings.set_model(kind, &model)?;
    settings.save()
}

/// Guarda la dirección del servidor de Ollama (vacía = la de por defecto) y devuelve la que quedó.
#[tauri::command]
fn set_ollama_url(url: String) -> Result<String, String> {
    let mut settings = Settings::load();
    settings.set_ollama_url(&url)?;
    settings.save()?;
    Ok(settings.ollama_url)
}

#[tauri::command]
fn set_ollama_think(value: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.ollama_think = value;
    settings.save()
}

/// Modelos descargados en Ollama, para sugerirlos en Ajustes. Sirve también de prueba de conexión.
#[tauri::command]
async fn list_ollama_models(url: Option<String>) -> Result<Vec<String>, String> {
    let url = url.filter(|text| !text.trim().is_empty()).unwrap_or_else(|| Settings::load().ollama_url);
    tauri::async_runtime::spawn_blocking(move || zen_ai::list_ollama_models(&url).map_err(|error| error.to_string()))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    if !provider_kind(&provider)?.needs_key() {
        return Err("Este proveedor no usa API key.".into());
    }
    secrets::save_key(provider_kind(&provider)?, &key)
}

#[tauri::command]
fn delete_api_key(provider: String) -> Result<(), String> {
    secrets::forget_key(provider_kind(&provider)?)
}
fn apply_policy(app: &AppHandle, settings: &Settings) {
    app.state::<Arc<ToolExecutor>>().set_policy(settings.policy());
}

#[tauri::command]
fn set_confirm_all(app: AppHandle, value: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.confirm_all = value;
    settings.save()?;
    apply_policy(&app, &settings);
    Ok(())
}

/// Activa o desactiva varias herramientas de una vez (los botones «todas» y «ninguna» de Ajustes).
#[tauri::command]
fn set_tools_enabled(app: AppHandle, names: Vec<String>, enabled: bool) -> Result<(), String> {
    let known: Vec<String> = app.state::<Arc<ToolExecutor>>().all_specs().into_iter().map(|spec| spec.name).collect();
    if let Some(unknown) = names.iter().find(|name| !known.contains(name)) {
        return Err(format!("La herramienta «{unknown}» no existe."));
    }
    let mut settings = Settings::load();
    for name in &names {
        settings.set_tool_enabled(name, enabled);
    }
    settings.save()?;
    apply_policy(&app, &settings);
    Ok(())
}

#[tauri::command]
fn set_tool_enabled(app: AppHandle, name: String, enabled: bool) -> Result<(), String> {
    if !app.state::<Arc<ToolExecutor>>().all_specs().iter().any(|spec| spec.name == name) {
        return Err(format!("La herramienta «{name}» no existe."));
    }
    let mut settings = Settings::load();
    settings.set_tool_enabled(&name, enabled);
    settings.save()?;
    apply_policy(&app, &settings);
    Ok(())
}

/// Interruptores del comportamiento autónomo: "wander" (pasear) y "look_at_cursor" (mirar el cursor).
#[tauri::command]
fn set_behavior(app: AppHandle, name: String, enabled: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    match name.as_str() {
        "wander" => settings.wander = enabled,
        "look_at_cursor" => settings.look_at_cursor = enabled,
        "music_reaction" => settings.music_reaction = enabled,
        _ => return Err(format!("Comportamiento desconocido: {name}")),
    }
    settings.save()?;
    sync_flags(&app, &settings);
    Ok(())
}

/// Lo que la mascota hace de verdad es lo que el usuario configuró con el perfil activo por encima.
fn sync_flags(app: &AppHandle, settings: &Settings) {
    let live = settings.effective();
    let flags = app.state::<Arc<BehaviorFlags>>();
    flags.wander.store(live.wander, std::sync::atomic::Ordering::Relaxed);
    flags.look_at_cursor.store(live.look_at_cursor, std::sync::atomic::Ordering::Relaxed);
}

/// Cambia de perfil (Normal, Trabajo, Descanso, Concentración). Se aplica al instante y no toca los ajustes del usuario.
#[tauri::command]
fn set_profile(app: AppHandle, id: String) -> Result<(), String> {
    let profile = profiles::find(&id).ok_or_else(|| format!("Perfil desconocido: {id}"))?;
    let mut settings = Settings::load();
    settings.profile = profile.id.to_string();
    settings.save()?;
    sync_flags(&app, &settings);
    let _ = app.emit("profile-changed", profile.id);
    // Un gesto de la mascota para que se note el cambio.
    publish(&app, Event::RandomAnimation);
    Ok(())
}

/// Sonidos y avisos: `enabled`/`volume` (0-100) de los sonidos, `battery_alerts` y `system_events`. Cada campo es opcional.
#[tauri::command]
fn set_sound(enabled: Option<bool>, volume: Option<u32>, battery_alerts: Option<bool>, system_events: Option<bool>) -> Result<(), String> {
    let mut settings = Settings::load();
    if let Some(enabled) = enabled {
        settings.sound_enabled = enabled;
    }
    if let Some(volume) = volume {
        settings.sound_volume = zen_desktop::sound::clamp_volume(volume);
    }
    if let Some(alerts) = battery_alerts {
        settings.battery_alerts = alerts;
    }
    if let Some(events) = system_events {
        settings.system_events = events;
    }
    settings.save()
}

/// Recuerdos guardados, para mostrarlos en Ajustes.
#[tauri::command]
fn list_memories(app: AppHandle) -> Vec<Value> {
    app.state::<Arc<zen_desktop::Memory>>().list().iter().map(|fact| json!({ "id": fact.id, "text": fact.text })).collect()
}

#[tauri::command]
fn delete_memory(app: AppHandle, id: u64) -> Result<(), String> {
    app.state::<Arc<zen_desktop::Memory>>().remove(id).map(|_| ()).ok_or_else(|| "Ese recuerdo ya no existe.".to_string())
}

#[tauri::command]
fn clear_memories(app: AppHandle) {
    app.state::<Arc<zen_desktop::Memory>>().clear();
}

/// Activa o desactiva la memoria (si se apaga, los recuerdos siguen guardados pero la IA no los ve ni añade otros).
#[tauri::command]
fn set_memory_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.memory_enabled = enabled;
    settings.save()?;
    app.state::<Arc<zen_desktop::Memory>>().set_enabled(enabled);
    Ok(())
}

#[tauri::command]
fn set_usage_comments(enabled: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.usage_comments = enabled;
    settings.save()
}

/// Interruptor general del control de otras aplicaciones (escribir, teclas, pulsar controles). Apagado por defecto.
#[tauri::command]
fn set_computer_control(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.computer_control = enabled;
    settings.save()?;
    apply_policy(&app, &settings);
    Ok(())
}

/// Suena un aviso de muestra con el volumen que se está eligiendo (aunque aún no esté guardado).
#[tauri::command]
fn test_sound(volume: u32) {
    zen_desktop::sound::play(zen_desktop::sound::SoundKind::Notify, zen_desktop::sound::clamp_volume(volume));
}

const TRAY_ID: &str = "ryan";

#[tauri::command]
fn get_assistant_name() -> String {
    Settings::load().assistant_name
}

/// Cambia el nombre del asistente: se guarda, se avisa a las ventanas y la IA lo usa desde su próxima respuesta.
#[tauri::command]
fn set_assistant_name(app: AppHandle, name: String) -> Result<(), String> {
    let name = zen_core::assistant::clean_assistant_name(&name)?;
    let mut settings = Settings::load();
    settings.assistant_name = name.clone();
    settings.save()?;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(Some(name.as_str()));
    }
    let _ = app.emit("assistant-name-changed", name);
    Ok(())
}

/// Personaje elegido; la ventana de la mascota lo pide al arrancar.
#[tauri::command]
fn get_character() -> String {
    Settings::load().character
}

/// Cambia el personaje: se guarda y se avisa a las ventanas para que lo apliquen al momento.
#[tauri::command]
fn set_character(app: AppHandle, id: String) -> Result<(), String> {
    if !characters::is_known(&id) {
        return Err(format!("Personaje desconocido: {id}"));
    }
    let mut settings = Settings::load();
    settings.character = id.clone();
    settings.save()?;
    let _ = app.emit("character-changed", id);
    Ok(())
}

/// Abre en el navegador un enlace que el usuario pulsó en el chat (fuentes de una búsqueda, por ejemplo).
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    zen_desktop::open_url(&url)
}

/// Guarda en los ajustes el estado actual de la lista de aplicaciones.
fn save_apps(catalog: &zen_desktop::AppCatalog) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.disabled_apps = catalog.disabled();
    settings.custom_apps = catalog.custom();
    settings.save()
}

/// Aplicaciones para la sección «Aplicaciones que puede abrir» de Ajustes. Con `refresh` (o si aún no se
/// leyó) consulta el menú Inicio, que tarda un par de segundos: por eso va fuera del hilo de la ventana.
#[tauri::command]
async fn list_apps(app: AppHandle, refresh: bool) -> Result<Value, String> {
    let catalog = app.state::<Arc<zen_desktop::AppCatalog>>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut error = None;
        if refresh || catalog.selectable().is_none() {
            if let Err(reason) = catalog.refresh() {
                error = Some(reason);
            }
        }
        let enabled: Vec<String> = catalog.custom().into_iter().map(|app| app.app_id).collect();
        let builtin: Vec<Value> = catalog
            .builtin_status()
            .into_iter()
            .map(|app| json!({ "key": app.key, "name": app.name, "enabled": app.enabled, "installed": app.installed }))
            .collect();
        let (apps, hidden_tools) = catalog.selectable().unwrap_or_default();
        let apps: Vec<Value> = apps
            .into_iter()
            .map(|app| json!({ "enabled": enabled.contains(&app.app_id), "name": app.name, "app_id": app.app_id }))
            .collect();
        json!({ "builtin": builtin, "apps": apps, "hidden_tools": hidden_tools, "error": error })
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_builtin_app(app: AppHandle, key: String, enabled: bool) -> Result<(), String> {
    let catalog = app.state::<Arc<zen_desktop::AppCatalog>>();
    catalog.set_builtin_enabled(&key, enabled)?;
    save_apps(&catalog)
}

/// Permite o quita varias aplicaciones del menú Inicio de una vez. Las que no se pueden permitir se saltan y se cuentan.
#[tauri::command]
fn set_custom_apps(app: AppHandle, app_ids: Vec<String>, enabled: bool) -> Result<String, String> {
    let catalog = app.state::<Arc<zen_desktop::AppCatalog>>();
    let mut skipped = 0;
    let mut first_error = None;
    for app_id in &app_ids {
        if let Err(error) = catalog.set_custom_enabled(app_id, enabled) {
            skipped += 1;
            first_error.get_or_insert(error);
        }
    }
    save_apps(&catalog)?;
    match (skipped, first_error) {
        (0, _) => Ok(String::new()),
        (n, Some(error)) if n == app_ids.len() => Err(error),
        (n, _) => Ok(format!("{n} aplicaciones no se pudieron cambiar.")),
    }
}

#[tauri::command]
fn set_custom_app(app: AppHandle, app_id: String, enabled: bool) -> Result<(), String> {
    let catalog = app.state::<Arc<zen_desktop::AppCatalog>>();
    catalog.set_custom_enabled(&app_id, enabled)?;
    save_apps(&catalog)
}

/// El usuario empezó a arrastrar la mascota: cancela cualquier paseo en curso.
#[tauri::command]
fn pet_dragged(app: AppHandle) {
    publish(&app, Event::CharacterDragged);
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let result = if enabled { launcher.enable() } else { launcher.disable() };
    result.map_err(|error| format!("No se pudo cambiar el inicio con Windows: {error}"))
}

fn parse_shortcut(text: &str) -> Result<Shortcut, String> {
    settings::validate_shortcut(text)?;
    text.parse::<Shortcut>().map_err(|error| format!("Atajo no reconocido: {error}"))
}

/// Cambia el atajo global: registra el nuevo antes de soltar el anterior, así nunca se queda sin ninguno.
#[tauri::command]
fn set_shortcut(app: AppHandle, shortcut: String) -> Result<(), String> {
    let new = parse_shortcut(&shortcut)?;
    if app.state::<ActiveQuickShortcut>().0.lock().expect("atajo envenenado").as_ref().is_some_and(|(quick, _)| *quick == new) {
        return Err("Ese atajo ya abre la barra rápida: elige otro distinto.".into());
    }
    let state = app.state::<ActiveShortcut>();
    let mut active = state.0.lock().expect("atajo envenenado");
    if active.as_ref().is_some_and(|(current, _)| *current == new) {
        let mut settings = Settings::load();
        settings.shortcut = shortcut.clone();
        settings.save()?;
        *active = Some((new, shortcut));
        return Ok(());
    }
    app.global_shortcut()
        .register(new.clone())
        .map_err(|_| "No se pudo registrar ese atajo: probablemente Windows u otra aplicación ya lo usa.".to_string())?;
    if let Some((old, _)) = active.take() {
        let _ = app.global_shortcut().unregister(old);
    }
    *active = Some((new, shortcut.clone()));
    let mut settings = Settings::load();
    settings.shortcut = shortcut;
    settings.save()
}

#[tauri::command]
fn resolve_confirmation(app: AppHandle, id: u64, approve: bool) {
    app.state::<ConfirmationWaiters>().resolve(id, approve);
}
/// Registra el atajo guardado; si no se puede (otra app lo usa), el de respaldo.
fn register_shortcut(app: &AppHandle, wanted: &str) {
    let active = app.state::<ActiveShortcut>();
    for candidate in [wanted, settings::FALLBACK_SHORTCUT] {
        let Ok(shortcut) = parse_shortcut(candidate) else { continue };
        if app.global_shortcut().register(shortcut.clone()).is_ok() {
            if candidate != wanted {
                eprintln!("El atajo {wanted} no está disponible; usando {candidate}");
            }
            #[cfg(debug_assertions)]
            println!("Atajo global: {candidate}");
            *active.0.lock().expect("atajo envenenado") = Some((shortcut, candidate.to_string()));
            return;
        }
    }
    eprintln!("No se pudo registrar ningún atajo global");
}
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir chat", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(Settings::load().assistant_name)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_chat(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_chat(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(music::MusicFlag::default())
        // Debe ser el primer plugin: si ya hay una copia en marcha, esta se cierra y abre el chat de la otra.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_chat(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let is_quick = app
                        .try_state::<ActiveQuickShortcut>()
                        .is_some_and(|state| state.0.lock().is_ok_and(|active| active.as_ref().is_some_and(|(quick, _)| quick == shortcut)));
                    if is_quick {
                        toggle_quick(app);
                    } else {
                        toggle_chat(app);
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            toggle_chat_cmd,
            hide_chat,
            submit_message,
            set_profile,
            list_activity,
            undo_activity,
            clear_activity,
            show_in_explorer,
            open_recycle_bin,
            hide_quick_cmd,
            resize_quick,
            quick_submit,
            open_chat_from_quick,
            set_quick_shortcut,
            set_tools_enabled,
            set_custom_apps,
            list_capturable_windows,
            capture_window_preview,
            list_conversations,
            load_conversation,
            resume_conversation,
            delete_conversation,
            clear_conversations,
            set_save_history,
            get_system_status,
            list_reminders,
            add_reminder,
            cancel_reminder,
            list_tasks,
            add_task,
            set_task_done,
            delete_task,
            clear_done_tasks,
            resolve_confirmation,
            get_settings,
            set_provider,
            set_model,
            save_api_key,
            delete_api_key,
            set_ollama_url,
            set_ollama_think,
            list_ollama_models,
            set_confirm_all,
            set_tool_enabled,
            set_autostart,
            set_shortcut,
            set_behavior,
            get_assistant_name,
            set_assistant_name,
            set_sound,
            test_sound,
            set_computer_control,
            list_memories,
            delete_memory,
            clear_memories,
            set_memory_enabled,
            set_usage_comments,
            get_character,
            set_character,
            open_link,
            music::is_music_playing,
            media::get_media,
            media::media_control,
            list_apps,
            set_builtin_app,
            set_custom_app,
            pet_dragged
        ])
        .setup(|app| {
            let pet = app.get_webview_window("main").expect("ventana main");
            // Windows exige un ancho mínimo (136 px) al crear la ventana; se relaja y se fija después.
            pet.set_min_size(Some(tauri::LogicalSize::new(1.0, 1.0)))?;
            pet.set_size(tauri::LogicalSize::new(PET_SIZE, PET_SIZE))?;
            place_in_corner(&pet)?;
            let bus = Arc::new(EventBus::new());
            let settings = Settings::load();
            let live = settings.effective();
            let flags = Arc::new(BehaviorFlags::new(live.wander, live.look_at_cursor));
            behavior::spawn(app.handle().clone(), bus.clone(), flags.clone());
            app.manage(flags);

            let locations = Locations::from_system().ok_or("no se pudieron localizar las carpetas del usuario")?;
            let mut registry = ToolRegistry::new();
            let data_dir = Settings::data_dir().ok_or("no se encontró la carpeta de configuración")?;
            // La búsqueda web usa el proveedor y la clave que el usuario tenga elegidos en cada momento.
            let provider_for_search: zen_core::websearch::ProviderFactory = Arc::new(|| {
                let settings = Settings::load();
                make_provider(&settings).ok_or_else(|| format!("Falta la API key de {}.", settings.provider.display_name()))
            });
            registry.register(zen_core::websearch::WebSearch::new(provider_for_search));
            let reminders = Arc::new(zen_desktop::Reminders::open(data_dir.clone()));
            let apps = Arc::new(zen_desktop::AppCatalog::new());
            apps.configure(settings.disabled_apps.clone(), settings.custom_apps.clone());
            app.manage(Arc::new(zen_desktop::conversations::Conversations::open(data_dir.clone())));
            let memory = Arc::new(zen_desktop::Memory::open(data_dir.clone()));
            memory.set_enabled(settings.memory_enabled);
            let tasks = Arc::new(zen_desktop::Tasks::open(data_dir.clone()));
            let locations = Arc::new(locations);
            app.manage(locations.clone());
            app.manage(Arc::new(zen_desktop::activity::Activity::open(data_dir.clone())));
            zen_desktop::register_default_tools(&mut registry, locations, data_dir, reminders.clone(), apps.clone(), memory.clone(), tasks.clone());
            app.manage(tasks);
            app.manage(memory);
            // Se lee el menú Inicio una vez al arrancar: así, cuando el usuario abre Ajustes, la lista ya está.
            #[cfg(debug_assertions)]
            println!("apps activadas: {}", apps.enabled_keys().join(", "));
            let warm_up = apps.clone();
            std::thread::spawn(move || {
                let result = warm_up.refresh();
                #[cfg(debug_assertions)]
                println!("menú Inicio leído: {}", result.as_ref().map(|list| list.len().to_string()).unwrap_or_else(|e| e.clone()));
                let _ = result;
            });
            app.manage(apps);
            let executor = Arc::new(ToolExecutor::new(registry, settings.policy(), bus.clone()));
            app.manage(Arc::new(Assistant::new(executor.clone(), bus.clone())));
            app.manage(executor);
            app.manage(ConfirmationWaiters::default());
            app.manage(bus);
            app.manage(ActiveShortcut::default());
            register_shortcut(app.handle(), &settings.shortcut);
            app.manage(ActiveQuickShortcut::default());
            register_quick_shortcut(app.handle(), &settings.quick_shortcut);
            build_tray(app.handle())?;
            // Al final: los avisos usan el bus de eventos, que ya debe estar registrado.
            // La pestaña Recordatorios de la interfaz lee y cambia este mismo almacén.
            app.manage(reminders.clone());
            system_events::spawn(app.handle().clone(), reminders.clone());
            alarms::spawn(app.handle().clone(), reminders);
            media::spawn(app.handle().clone());
            music::spawn(app.handle().clone());
            notify::spawn(app.handle().clone());
            power::spawn(app.handle().clone());
            usage::spawn(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al iniciar Ryan");
}

#[cfg(test)]
mod tests {
    use tauri_plugin_global_shortcut::{Code, Modifiers};

    use super::*;

    #[test]
    fn el_atajo_por_defecto_y_el_de_respaldo_se_entienden() {
        assert_eq!(
            parse_shortcut(settings::DEFAULT_SHORTCUT).unwrap(),
            Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyK)
        );
        assert_eq!(
            parse_shortcut(settings::FALLBACK_SHORTCUT).unwrap(),
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyK)
        );
    }

    /// Todo lo que acepta `validate_shortcut` debe poder registrarse: si no, la pantalla de ajustes
    /// ofrecería atajos que luego fallan.
    #[test]
    fn toda_tecla_valida_se_convierte_en_un_atajo_registrable() {
        let keys = ('A'..='Z').map(String::from).chain(('0'..='9').map(String::from)).chain((1..=12).map(|n| format!("F{n}")));
        for key in keys {
            for modifiers in ["Ctrl", "Alt", "Super", "Ctrl+Shift", "Ctrl+Alt+Super+Shift"] {
                let text = format!("{modifiers}+{key}");
                assert!(parse_shortcut(&text).is_ok(), "{text}");
            }
        }
    }
}
