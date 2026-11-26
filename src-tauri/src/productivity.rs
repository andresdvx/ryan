//! Puente nativo: temporizador independiente del WebView y rutinas que pasan por Assistant/ToolExecutor.
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use tauri::{AppHandle, Emitter, Manager};
use serde_json::{json, Value};
use zen_core::{ai::ToolInvocation, assistant::Assistant, events::Event};
use zen_desktop::{unix_now, productivity::{FocusTimer, Phase, RoutineStep, Routines}};

#[derive(Default)]
pub struct RoutineRun {
    active: Mutex<Option<(u64, String)>>,
    cancelled: AtomicBool,
}

#[tauri::command]
pub fn get_productivity(app: AppHandle) -> Result<Value, String> {
    let timer = app.try_state::<FocusTimer>().ok_or("El temporizador se está preparando.")?;
    let routines = app.try_state::<Routines>().ok_or("Las rutinas se están preparando.")?;
    let run = app.state::<RoutineRun>();
    let active = run.active.lock().map_err(|_| "No pude consultar la rutina.")?.clone();
    Ok(json!({ "focus": timer.snapshot(unix_now()), "routines": routines.list(), "active_routine": active.map(|(id,name)| json!({"id":id,"name":name})) }))
}

#[tauri::command]
pub fn focus_action(app: AppHandle, action: String, work_minutes: u64, break_minutes: u64) -> Result<Value, String> {
    let timer = app.try_state::<FocusTimer>().ok_or("El temporizador se está preparando.")?;
    let state = timer.action(&action, work_minutes, break_minutes, unix_now())?;
    let _ = app.emit("productivity-changed", ());
    serde_json::to_value(state).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_routine(app: AppHandle, id: Option<u64>, name: String, steps: Vec<Value>) -> Result<Value, String> {
    let steps: Vec<RoutineStep> = steps.into_iter().map(|v| serde_json::from_value(v).map_err(|e| format!("Paso inválido: {e}"))).collect::<Result<_,_>>()?;
    let store = app.try_state::<Routines>().ok_or("Las rutinas se están preparando.")?;
    let routine = store.put(id, &name, steps)?;
    let _ = app.emit("productivity-changed", ());
    serde_json::to_value(routine).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_routine(app: AppHandle, id: u64) -> Result<(), String> {
    let run = app.state::<RoutineRun>();
    let active = run.active.lock().map_err(|_| "No pude consultar la rutina.")?;
    if active.as_ref().is_some_and(|(active,_)| *active == id) { return Err("Detén la rutina antes de borrarla.".into()); }
    app.state::<Routines>().delete(id)?;
    let _ = app.emit("productivity-changed", ());
    Ok(())
}

#[tauri::command]
pub fn cancel_routine(app: AppHandle) {
    app.state::<RoutineRun>().cancelled.store(true, Ordering::SeqCst);
}

#[tauri::command]
pub fn run_routine(app: AppHandle, id: u64, expected: Value) -> Result<(), String> {
    let routine = app.try_state::<Routines>().ok_or("Las rutinas se están preparando.")?.get(id)?;
    if expected != json!({ "name": routine.name, "steps": routine.steps }) {
        return Err("La rutina cambió desde la revisión. Vuelve a revisar sus pasos antes de ejecutarla.".into());
    }
    let run = app.state::<RoutineRun>();
    {
        let mut active = run.active.lock().map_err(|_| "No pude iniciar la rutina.")?;
        if active.is_some() { return Err("Ya hay una rutina en curso.".into()); }
        run.cancelled.store(false, Ordering::SeqCst);
        *active = Some((routine.id, routine.name.clone()));
    }
    let _ = app.emit("productivity-changed", ());
    std::thread::spawn(move || {
        let run = app.state::<RoutineRun>();
        let calls: Vec<_> = routine.steps.into_iter().map(|step| ToolInvocation { id: None, name: step.tool, args: step.args }).collect();
        let ui = crate::ChatUi::new(app.clone());
        crate::say(&app, format!("Rutina «{}»: {} paso(s).", routine.name, calls.len()));
        let result = app.state::<Arc<Assistant>>().run_sequence(&calls, &ui, &run.cancelled);
        let (ok, message) = match result {
            Ok(count) => (true, format!("Rutina «{}» completada: {count} paso(s).", routine.name)),
            Err(error) => (false, format!("Rutina «{}»: {error}", routine.name)),
        };
        crate::say(&app, &message);
        *run.active.lock().expect("rutina envenenada") = None;
        let _ = app.emit("routine-finished", json!({ "ok": ok, "message": message }));
        let _ = app.emit("productivity-changed", ());
    });
    Ok(())
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        match app.state::<FocusTimer>().tick(unix_now()) {
            Ok(Some(phase)) => {
                let text = if phase == Phase::Focus {
                    "Terminó tu bloque de enfoque. Tómate un descanso; inícialo cuando quieras."
                } else {
                    "Terminó el descanso. Puedes iniciar otro bloque de enfoque."
                };
                let _ = app.emit("focus-finished", text);
                let _ = app.emit("productivity-changed", ());
                let _ = app.emit("reminder", text);
                crate::show_chat_with(&app, false);
                crate::publish(&app, Event::ReminderDue { text: text.into() });
            }
            Err(error) => {
                // No se avisa de un fin que no pudo guardarse; se reintenta sin perder la sesión.
                #[cfg(debug_assertions)]
                eprintln!("temporizador: {error}");
                let _ = error;
            }
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    });
}
