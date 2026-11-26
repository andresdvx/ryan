//! Temporizador y rutinas locales. Ningún paso se ejecuta aquí: la aplicación usa ToolExecutor.
use std::{path::PathBuf, sync::Mutex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn save(path: &PathBuf, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("Ruta de datos inválida.")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&temporary, body).map_err(|e| format!("No pude guardar los cambios: {e}"))?;
    std::fs::rename(temporary, path).map_err(|e| format!("No pude guardar los cambios: {e}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase { Focus, Break }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerStatus { Ready, Running, Paused }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusState {
    pub phase: Phase,
    pub status: TimerStatus,
    pub work_minutes: u64,
    pub break_minutes: u64,
    pub remaining_seconds: u64,
    pub deadline: Option<u64>,
    pub completed: u64,
}
impl Default for FocusState {
    fn default() -> Self {
        Self { phase: Phase::Focus, status: TimerStatus::Ready, work_minutes: 25, break_minutes: 5, remaining_seconds: 1500, deadline: None, completed: 0 }
    }
}
impl FocusState {
    fn valid(&self) -> bool {
        (1..=180).contains(&self.work_minutes) && (1..=60).contains(&self.break_minutes)
            && self.remaining_seconds <= 180 * 60
            && (self.status == TimerStatus::Running) == self.deadline.is_some()
    }
    fn remaining(&self, now: u64) -> u64 {
        self.deadline.map(|d| d.saturating_sub(now)).unwrap_or(self.remaining_seconds)
    }
}
pub struct FocusTimer {
    path: PathBuf,
    state: Mutex<FocusState>,
}
impl FocusTimer {
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join("focus.json");
        let state = std::fs::read(&path).ok().and_then(|v| serde_json::from_slice::<FocusState>(&v).ok()).filter(FocusState::valid).unwrap_or_default();
        Self { path, state: Mutex::new(state) }
    }
    pub fn snapshot(&self, now: u64) -> FocusState {
        let mut state = self.state.lock().expect("temporizador envenenado").clone();
        state.remaining_seconds = state.remaining(now);
        state
    }
    pub fn action(&self, action: &str, work: u64, rest: u64, now: u64) -> Result<FocusState, String> {
        if !(1..=180).contains(&work) || !(1..=60).contains(&rest) {
            return Err("El enfoque debe durar de 1 a 180 min y el descanso de 1 a 60 min.".into());
        }
        let mut locked = self.state.lock().expect("temporizador envenenado");
        let mut next = locked.clone();
        match action {
            "start" if next.status == TimerStatus::Ready => {
                next.work_minutes = work;
                next.break_minutes = rest;
                next.remaining_seconds = if next.phase == Phase::Focus { work * 60 } else { rest * 60 };
                next.deadline = Some(now.saturating_add(next.remaining_seconds));
                next.status = TimerStatus::Running;
            }
            "pause" if next.status == TimerStatus::Running => {
                next.remaining_seconds = next.remaining(now);
                if next.remaining_seconds == 0 { return Err("La sesión terminó; espera a que se actualice.".into()); }
                next.deadline = None;
                next.status = TimerStatus::Paused;
            }
            "resume" if next.status == TimerStatus::Paused => {
                next.deadline = Some(now.saturating_add(next.remaining_seconds));
                next.status = TimerStatus::Running;
            }
            "stop" => {
                next = FocusState { completed: next.completed, work_minutes: work, break_minutes: rest, remaining_seconds: work * 60, ..FocusState::default() };
            }
            _ => return Err("Esa acción no está disponible en el estado actual del temporizador.".into()),
        }
        save(&self.path, &next)?;
        *locked = next.clone();
        Ok(next)
    }
    /// Devuelve la fase que terminó. La siguiente requiere Inicio: no encadena alarmas al reabrir.
    pub fn tick(&self, now: u64) -> Result<Option<Phase>, String> {
        let mut locked = self.state.lock().expect("temporizador envenenado");
        if locked.status != TimerStatus::Running || locked.remaining(now) > 0 { return Ok(None); }
        let ended = locked.phase.clone();
        let mut next = locked.clone();
        next.phase = if ended == Phase::Focus { Phase::Break } else { Phase::Focus };
        if ended == Phase::Focus { next.completed = next.completed.saturating_add(1); }
        next.remaining_seconds = if next.phase == Phase::Focus { next.work_minutes * 60 } else { next.break_minutes * 60 };
        next.status = TimerStatus::Ready;
        next.deadline = None;
        save(&self.path, &next)?;
        *locked = next;
        Ok(Some(ended))
    }
}

pub const MAX_ROUTINES: usize = 20;
pub const MAX_STEPS: usize = 8;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineStep { pub tool: String, pub args: Value }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routine { pub id: u64, pub name: String, pub steps: Vec<RoutineStep> }
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineData { next_id: u64, items: Vec<Routine> }
pub struct Routines { path: PathBuf, state: Mutex<RoutineData> }

fn validate(name: &str, steps: &[RoutineStep]) -> Result<(), String> {
    if name.trim().is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("El nombre debe tener de 1 a 60 caracteres, sin saltos de línea.".into());
    }
    if steps.is_empty() || steps.len() > MAX_STEPS { return Err(format!("Una rutina necesita de 1 a {MAX_STEPS} pasos.")); }
    for step in steps {
        let allowed: &[&str] = match step.tool.as_str() {
            "open_application" => &["application"],
            "open_website" => &["url"],
            "play_music" => &["query", "service"],
            "set_reminder" => &["text", "delay_seconds"],
            "list_tasks" | "list_reminders" | "system_info" => &[],
            _ => return Err("Esa herramienta no está disponible para rutinas. No se admiten comandos ni acciones destructivas.".into()),
        };
        let args = step.args.as_object().ok_or("Los argumentos del paso deben ser un objeto.")?;
        if args.keys().any(|k| !allowed.contains(&k.as_str())) { return Err("El paso contiene argumentos no permitidos.".into()); }
        let text = |key: &str, max: usize| -> Result<&str, String> {
            args.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty() && v.chars().count() <= max && !v.chars().any(char::is_control))
                .ok_or_else(|| format!("Revisa el campo {key} del paso."))
        };
        match step.tool.as_str() {
            "open_application" => {
                let app = text("application", 200)?;
                if crate::is_blocked_app(app, app) { return Err("No se admiten terminales ni herramientas de administración.".into()); }
            }
            "open_website" => {
                let parsed = url::Url::parse(text("url", 2048)?).map_err(|_| "La dirección web no es válida.")?;
                if parsed.scheme() != "https" || parsed.host_str().is_none() || !parsed.username().is_empty() || parsed.password().is_some() {
                    return Err("Usa una dirección https sin credenciales.".into());
                }
            }
            "play_music" => {
                text("query", 100)?;
                if args.get("service").and_then(Value::as_str) != Some("spotify") { return Err("Las rutinas de música usan Spotify.".into()); }
            }
            "set_reminder" => {
                text("text", 200)?;
                if !args.get("delay_seconds").and_then(Value::as_u64).is_some_and(|n| (60..=604800).contains(&n)) { return Err("El recordatorio debe vencer entre 1 min y 7 días.".into()); }
            }
            _ => {}
        }
    }
    Ok(())
}
impl Routines {
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join("routines.json");
        let mut data = std::fs::read(&path).ok().and_then(|v| serde_json::from_slice::<RoutineData>(&v).ok()).unwrap_or_default();
        data.items.retain(|r| r.id > 0 && validate(&r.name, &r.steps).is_ok());
        data.items.truncate(MAX_ROUTINES);
        data.next_id = data.next_id.max(data.items.iter().map(|r| r.id).max().unwrap_or(0).saturating_add(1));
        Self { path, state: Mutex::new(data) }
    }
    pub fn list(&self) -> Vec<Routine> { self.state.lock().expect("rutinas envenenado").items.clone() }
    pub fn get(&self, id: u64) -> Result<Routine, String> {
        self.list().into_iter().find(|r| r.id == id).ok_or_else(|| "La rutina ya no existe.".into())
    }
    pub fn put(&self, id: Option<u64>, name: &str, steps: Vec<RoutineStep>) -> Result<Routine, String> {
        validate(name, &steps)?;
        let name = name.trim();
        let mut locked = self.state.lock().expect("rutinas envenenado");
        let mut next = locked.clone();
        if next.items.iter().any(|r| Some(r.id) != id && r.name.to_lowercase() == name.to_lowercase()) {
            return Err("Ya existe una rutina con ese nombre.".into());
        }
        let index = match id {
            Some(id) => Some(next.items.iter().position(|r| r.id == id).ok_or("La rutina ya no existe.")?),
            None if next.items.len() >= MAX_ROUTINES => return Err(format!("Puedes guardar hasta {MAX_ROUTINES} rutinas.")),
            None => None,
        };
        let routine = Routine { id: id.unwrap_or(next.next_id), name: name.to_string(), steps };
        if let Some(index) = index { next.items[index] = routine.clone(); } else {
            next.next_id = next.next_id.checked_add(1).ok_or("No quedan identificadores.")?;
            next.items.push(routine.clone());
        }
        save(&self.path, &next)?;
        *locked = next;
        Ok(routine)
    }
    pub fn delete(&self, id: u64) -> Result<(), String> {
        let mut locked = self.state.lock().expect("rutinas envenenado");
        let mut next = locked.clone();
        let index = next.items.iter().position(|r| r.id == id).ok_or("La rutina ya no existe.")?;
        next.items.remove(index);
        save(&self.path, &next)?;
        *locked = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_locations;
    use serde_json::json;
    #[test]
    fn timer_pausa_reanuda_y_no_depende_del_chat() {
        let t = temp_locations(); let dir = t.locations_root("data");
        let timer = FocusTimer::open(dir.clone());
        timer.action("start", 25, 5, 100).unwrap();
        assert_eq!(timer.snapshot(130).remaining_seconds, 1470);
        timer.action("pause", 25, 5, 130).unwrap();
        let reopened = FocusTimer::open(dir);
        assert_eq!(reopened.snapshot(1000).remaining_seconds, 1470);
        reopened.action("resume", 25, 5, 1000).unwrap();
        assert_eq!(reopened.tick(2470).unwrap(), Some(Phase::Focus));
        assert_eq!(reopened.tick(99999).unwrap(), None);
        assert_eq!(reopened.snapshot(99999).phase, Phase::Break);
        assert_eq!(reopened.snapshot(99999).completed, 1);
        reopened.action("start", 25, 5, 100000).unwrap();
        assert_eq!(reopened.tick(100300).unwrap(), Some(Phase::Break));
    }
    #[test]
    fn timer_reabierto_vence_una_vez_y_rechaza_transiciones_invalidas() {
        let t = temp_locations(); let dir = t.locations_root("data");
        FocusTimer::open(dir.clone()).action("start", 1, 1, 100).unwrap();
        let timer = FocusTimer::open(dir);
        assert_eq!(timer.tick(9999).unwrap(), Some(Phase::Focus));
        assert_eq!(timer.tick(10000).unwrap(), None);
        assert!(timer.action("resume", 25, 5, 1).is_err());
        assert!(timer.action("start", 0, 5, 1).is_err());
    }
    fn step() -> RoutineStep { RoutineStep { tool: "list_tasks".into(), args: json!({}) } }
    #[test]
    fn rutinas_guardan_editan_y_borran_sin_reutilizar_ids() {
        let t = temp_locations(); let dir = t.locations_root("data");
        let store = Routines::open(dir.clone());
        let r = store.put(None, "Mi jornada", vec![step()]).unwrap();
        store.put(Some(r.id), "Trabajo", vec![step(), step()]).unwrap();
        let reopened = Routines::open(dir);
        assert_eq!(reopened.get(r.id).unwrap().steps.len(), 2);
        assert!(reopened.put(None, " trabajo ", vec![step()]).is_err());
        reopened.delete(r.id).unwrap();
        assert!(reopened.put(None, "Nueva", vec![step()]).unwrap().id > r.id);
    }
    #[test]
    fn rutinas_rechazan_terminal_borrados_inyecciones_y_limites() {
        for tool in ["shell", "delete_file", "type_text", "open_path"] {
            assert!(validate("x", &[RoutineStep { tool: tool.into(), args: json!({}) }]).is_err());
        }
        assert!(validate("x", &[]).is_err());
        assert!(validate("x", &vec![step(); MAX_STEPS + 1]).is_err());
        assert!(validate("x", &[RoutineStep { tool: "list_tasks".into(), args: json!({"command":"bad"}) }]).is_err());
        assert!(validate("x", &[RoutineStep { tool: "open_website".into(), args: json!({"url":"javascript:alert(1)"}) }]).is_err());
    }
}
