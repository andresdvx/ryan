//! Lista de tareas pendientes («R38838 - ESE», «llamar al dentista»). Se guarda en disco, sobrevive a cerrar la
//! aplicación y se maneja con cuatro herramientas: agregar, listar, marcar (como hecha, como **parcial** —empezada pero
//! sin terminar, con una nota de lo que falta— o de nuevo como pendiente) y borrar.
//! A diferencia de un recordatorio no tiene hora: es una lista para ir tachando.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, optional_str, optional_u64, required_str};

pub const MAX_TASKS: usize = 100;
const MAX_TEXT_CHARS: usize = 200;

/// En qué punto está una tarea. Una parcial sigue contando como pendiente: aún queda trabajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Partial,
    Done,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Partial => "partial",
            Status::Done => "done",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "pending" => Some(Status::Pending),
            "partial" => Some(Status::Partial),
            "done" => Some(Status::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: u64,
    pub text: String,
    pub done: bool,
    /// Empezada pero sin terminar (nunca a la vez que `done`).
    pub partial: bool,
    /// Qué falta (o qué se hizo), solo en las parciales.
    pub note: Option<String>,
}

impl Task {
    pub fn status(&self) -> Status {
        if self.done {
            Status::Done
        } else if self.partial {
            Status::Partial
        } else {
            Status::Pending
        }
    }
}

const MAX_NOTE_CHARS: usize = 200;

/// Nota de una tarea parcial: texto de una línea o nada.
fn clean_note(note: &str) -> Result<Option<String>, String> {
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    if note.is_empty() {
        return Ok(None);
    }
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("la nota debe tener como mucho {MAX_NOTE_CHARS} caracteres"));
    }
    if note.chars().any(char::is_control) {
        return Err("la nota no puede tener caracteres de control".into());
    }
    Ok(Some(note))
}

#[derive(Default)]
struct State {
    next_id: u64,
    items: Vec<Task>,
}

pub struct Tasks {
    path: PathBuf,
    state: Mutex<State>,
}

fn clean_text(text: &str) -> Result<String, String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = text.chars().count();
    if length == 0 || length > MAX_TEXT_CHARS {
        return Err(format!("la tarea debe tener entre 1 y {MAX_TEXT_CHARS} caracteres"));
    }
    if text.chars().any(char::is_control) {
        return Err("la tarea no puede tener caracteres de control".into());
    }
    Ok(text)
}

impl Tasks {
    /// Abre (o crea) el almacén en `dir`. Un archivo ilegible se trata como vacío.
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join("tasks.json");
        let state = Self::load(&path).unwrap_or_default();
        Self { path, state: Mutex::new(state) }
    }

    fn load(path: &PathBuf) -> Option<State> {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let items: Vec<Task> = value["items"]
            .as_array()?
            .iter()
            .filter_map(|item| {
                let text = clean_text(item["text"].as_str()?).ok()?;
                let done = item["done"].as_bool().unwrap_or(false);
                let partial = !done && item["partial"].as_bool().unwrap_or(false);
                let note = if partial { item["note"].as_str().and_then(|note| clean_note(note).ok().flatten()) } else { None };
                Some(Task { id: item["id"].as_u64()?, text, done, partial, note })
            })
            .take(MAX_TASKS)
            .collect();
        let floor = items.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        Some(State { next_id: value["next_id"].as_u64().unwrap_or(1).max(floor), items })
    }

    fn save(&self, state: &State) -> Result<(), String> {
        let items: Vec<Value> = state.items.iter().map(|t| json!({ "id": t.id, "text": t.text, "done": t.done, "partial": t.partial, "note": t.note })).collect();
        let body = serde_json::to_string_pretty(&json!({ "next_id": state.next_id, "items": items })).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear la carpeta de datos: {e}"))?;
        }
        // Se escribe a un archivo temporal y se renombra, para no dejar un JSON a medias si se corta la luz.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, body).map_err(|e| format!("no se pudieron guardar las tareas: {e}"))?;
        std::fs::rename(&temp, &self.path).map_err(|e| format!("no se pudieron guardar las tareas: {e}"))
    }

    /// Agrega una tarea pendiente. Si la lista está llena se descartan primero las ya hechas, las más antiguas antes.
    pub fn add(&self, text: &str) -> Result<Task, String> {
        let text = clean_text(text)?;
        let mut state = self.state.lock().expect("tareas envenenado");
        if state.items.iter().any(|t| !t.done && t.text.to_lowercase() == text.to_lowercase()) {
            return Err(format!("ya tienes pendiente «{text}»"));
        }
        if state.items.len() >= MAX_TASKS {
            let Some(oldest_done) = state.items.iter().position(|t| t.done) else {
                return Err(format!("ya hay {MAX_TASKS} tareas pendientes; borra o completa alguna antes de agregar más"));
            };
            state.items.remove(oldest_done);
        }
        state.next_id = state.next_id.max(1);
        let task = Task { id: state.next_id, text, done: false, partial: false, note: None };
        state.next_id += 1;
        state.items.push(task.clone());
        self.save(&state)?;
        Ok(task)
    }

    /// Todas, en el orden en que se agregaron.
    pub fn list(&self) -> Vec<Task> {
        self.state.lock().expect("tareas envenenado").items.clone()
    }

    /// Lo que aún queda por hacer: las pendientes y las parciales.
    pub fn pending_count(&self) -> usize {
        self.list().iter().filter(|t| !t.done).count()
    }

    pub fn partial_count(&self) -> usize {
        self.list().iter().filter(|t| t.partial).count()
    }

    /// Marca (o desmarca) una tarea como hecha. Devuelve la tarea ya cambiada.
    pub fn set_done(&self, id: u64, done: bool) -> Option<Task> {
        self.set_status(id, if done { Status::Done } else { Status::Pending }, None).ok().flatten()
    }

    /// Cambia el estado de una tarea. `note` solo vale para una parcial (qué falta): sin nota se conserva la que ya
    /// tuviera; al pasar a hecha o a pendiente la nota se borra. `Ok(None)` si la tarea no existe.
    pub fn set_status(&self, id: u64, status: Status, note: Option<&str>) -> Result<Option<Task>, String> {
        let note = note.map(clean_note).transpose()?.flatten();
        let mut state = self.state.lock().expect("tareas envenenado");
        let Some(task) = state.items.iter_mut().find(|t| t.id == id) else { return Ok(None) };
        task.done = status == Status::Done;
        task.partial = status == Status::Partial;
        task.note = if task.partial { note.or_else(|| task.note.take()) } else { None };
        let task = task.clone();
        let _ = self.save(&state);
        Ok(Some(task))
    }

    pub fn remove(&self, id: u64) -> Option<Task> {
        let mut state = self.state.lock().expect("tareas envenenado");
        let position = state.items.iter().position(|t| t.id == id)?;
        let removed = state.items.remove(position);
        let _ = self.save(&state);
        Some(removed)
    }

    /// Quita todas las ya hechas; devuelve cuántas.
    pub fn clear_done(&self) -> usize {
        let mut state = self.state.lock().expect("tareas envenenado");
        let before = state.items.len();
        state.items.retain(|t| !t.done);
        let removed = before - state.items.len();
        if removed > 0 {
            let _ = self.save(&state);
        }
        removed
    }

    /// Busca una tarea por su número o por parte de su texto (sin distinguir mayúsculas). Con el texto solo mira las
    /// que cumplen `wanted`; error si no hay ninguna o hay varias que encajan.
    fn find(&self, id: Option<u64>, text: Option<&str>, wanted: impl Fn(&Task) -> bool) -> Result<Task, String> {
        let items = self.list();
        match (id, text) {
            (Some(id), None) => items.into_iter().find(|t| t.id == id).ok_or_else(|| format!("no hay ninguna tarea con el número {id}")),
            (None, Some(text)) => {
                let query = text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
                if query.is_empty() {
                    return Err("indica qué tarea".into());
                }
                let candidates: Vec<&Task> = items.iter().filter(|t| wanted(t)).collect();
                // Una coincidencia exacta gana a las parciales: «R38» no confunde «R38838 - ESE» con «R38999».
                let exact: Vec<&&Task> = candidates.iter().filter(|t| t.text.to_lowercase() == query).collect();
                let matches: Vec<&Task> = match exact.as_slice() {
                    [one] => vec![**one],
                    _ => candidates.iter().copied().filter(|t| t.text.to_lowercase().contains(&query)).collect(),
                };
                match matches.as_slice() {
                    [] => Err(format!("no encuentro ninguna tarea que contenga «{text}»")),
                    [one] => Ok((*one).clone()),
                    many => Err(format!(
                        "varias tareas coinciden con «{text}»: {}. Usa el número.",
                        many.iter().map(|t| format!("#{} {}", t.id, t.text)).collect::<Vec<_>>().join("; ")
                    )),
                }
            }
            _ => Err("indica «id» o «text» (solo uno de los dos)".into()),
        }
    }
}

fn target_args(args: &Value, extra: &[&str]) -> Result<(Option<u64>, Option<String>), ToolError> {
    let mut keys = vec!["id", "text"];
    keys.extend_from_slice(extra);
    check_keys(args, &keys)?;
    let id = optional_u64(args, "id", 1, u64::MAX / 2)?;
    let text = optional_str(args, "text")?.map(str::to_string);
    if id.is_some() == text.is_some() {
        return Err(invalid("indica «id» o «text» (solo uno de los dos)"));
    }
    Ok((id, text))
}

/// A qué estado se pasa la tarea: «status» (hecha por defecto) o, por compatibilidad, «done» (false = pendiente).
fn status_arg(args: &Value) -> Result<Status, ToolError> {
    let status = optional_str(args, "status")?;
    let done = match args.get("done") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(done)) => Some(*done),
        Some(_) => return Err(invalid("«done» debe ser verdadero o falso")),
    };
    match (status, done) {
        (Some(_), Some(_)) => Err(invalid("usa «status» o «done», no los dos")),
        (Some(key), None) => Status::from_key(key).ok_or_else(|| invalid("«status» debe ser done, partial o pending")),
        (None, Some(false)) => Ok(Status::Pending),
        (None, _) => Ok(Status::Done),
    }
}

/// La nota que acompaña al estado: solo tiene sentido en una parcial.
fn note_arg(args: &Value, status: Status) -> Result<Option<String>, ToolError> {
    let Some(note) = optional_str(args, "note")? else { return Ok(None) };
    if status != Status::Partial {
        return Err(invalid("«note» solo se usa con status partial"));
    }
    clean_note(note).map_err(invalid)
}

fn describe(task: &Task) -> String {
    let mark = match task.status() {
        Status::Done => "✔",
        Status::Partial => "◐",
        Status::Pending => "○",
    };
    match &task.note {
        Some(note) => format!("{mark} #{} {} — {note}", task.id, task.text),
        None => format!("{mark} #{} {}", task.id, task.text),
    }
}

// ---------------------------------------------------------------------------------------------
// Herramientas
// ---------------------------------------------------------------------------------------------

pub struct AddTask(pub Arc<Tasks>);

impl Tool for AddTask {
    fn name(&self) -> &'static str {
        "add_task"
    }

    fn description(&self) -> &'static str {
        "Agrega una tarea a la lista de tareas pendientes del usuario («agrega X a mi lista de tareas»). Guarda \
         el texto tal cual lo dijo (por ejemplo «R38838 - ESE»). No es un recordatorio: no tiene hora; si el \
         usuario quiere que se le avise, usa set_reminder."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": format!("La tarea, tal cual (hasta {MAX_TEXT_CHARS} caracteres).") } },
            "required": ["text"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &["text"])?;
        clean_text(required_str(args, "text")?).map(|_| ()).map_err(invalid)
    }

    fn preview(&self, args: &Value) -> String {
        match args["text"].as_str().map(clean_text) {
            Some(Ok(text)) => format!("Agregar a tus tareas: «{text}»"),
            _ => "Agregar una tarea".into(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        check_keys(args, &["text"])?;
        let task = self.0.add(required_str(args, "text")?).map_err(ToolError::Failed)?;
        let pending = self.0.pending_count();
        Ok(ToolOutput {
            summary: format!("Agregué a tus tareas: «{}» (tienes {pending} pendiente(s))", task.text),
            data: json!({ "id": task.id, "text": task.text, "pending": pending }),
        })
    }
}

pub struct ListTasks(pub Arc<Tasks>);

impl Tool for ListTasks {
    fn name(&self) -> &'static str {
        "list_tasks"
    }

    fn description(&self) -> &'static str {
        "Lista las tareas del usuario con su número y su estado: hechas (✔), parciales (◐, empezadas pero sin \
         terminar, con una nota de lo que falta) o pendientes (○). Por defecto muestra todas; con «only_pending» \
         solo lo que aún queda (pendientes y parciales). Úsala antes de marcar o borrar una tarea si no estás seguro \
         de cuál es."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "only_pending": { "type": "boolean", "description": "Solo las pendientes (por defecto, todas)." } }
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &["only_pending"])?;
        match args.get("only_pending") {
            None | Some(Value::Null | Value::Bool(_)) => Ok(()),
            Some(_) => Err(invalid("«only_pending» debe ser verdadero o falso")),
        }
    }

    fn preview(&self, _args: &Value) -> String {
        "Consultar tu lista de tareas".into()
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        self.validate(args)?;
        let only_pending = args.get("only_pending").and_then(Value::as_bool).unwrap_or(false);
        let all = self.0.list();
        let items: Vec<&Task> = all.iter().filter(|t| !only_pending || !t.done).collect();
        let pending = all.iter().filter(|t| !t.done).count();
        if items.is_empty() {
            let summary = if all.is_empty() { "No tienes tareas." } else { "No tienes tareas pendientes." };
            return Ok(ToolOutput { summary: summary.into(), data: json!({ "tasks": [], "pending": pending }) });
        }
        let partial = all.iter().filter(|t| t.partial).count();
        let extra = if partial > 0 { format!(", {partial} parcial(es)") } else { String::new() };
        let mut summary = format!("Tienes {pending} tarea(s) pendiente(s) de {}{extra}:", all.len());
        for task in &items {
            summary.push_str(&format!("\n{}", describe(task)));
        }
        let data: Vec<Value> = items.iter().map(|t| json!({ "id": t.id, "text": t.text, "done": t.done, "partial": t.partial, "status": t.status().key(), "note": t.note })).collect();
        Ok(ToolOutput { summary, data: json!({ "tasks": data, "pending": pending }) })
    }
}

pub struct CompleteTask(pub Arc<Tasks>);

impl Tool for CompleteTask {
    fn name(&self) -> &'static str {
        "complete_task"
    }

    fn description(&self) -> &'static str {
        "Cambia el estado de una tarea: hecha (status=done, por defecto), PARCIAL (status=partial: se avanzó pero no \
         está terminada; añade en «note» qué falta o qué se hizo) o de nuevo pendiente (status=pending). Indica su \
         número («id», el que muestra list_tasks) o parte de su texto («text», por ejemplo «R38838»). Si el texto \
         coincide con varias, pide al usuario cuál o usa el número."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "minimum": 1, "description": "Número de la tarea." },
                "text": { "type": "string", "description": "Parte del texto de la tarea (alternativa al número)." },
                "status": { "type": "string", "enum": ["done", "partial", "pending"], "description": "done (por defecto): terminada; partial: terminada solo en parte; pending: sin empezar." },
                "note": { "type": "string", "description": format!("Solo con partial: qué falta o qué se hizo (hasta {MAX_NOTE_CHARS} caracteres).") },
                "done": { "type": "boolean", "description": "Alternativa antigua a status: false la vuelve a dejar pendiente." }
            }
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        target_args(args, &["status", "note", "done"])?;
        let status = status_arg(args)?;
        note_arg(args, status).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        let status = status_arg(args).unwrap_or(Status::Done);
        let action = match status {
            Status::Done => "Marcar como hecha",
            Status::Partial => "Marcar como parcial",
            Status::Pending => "Volver a dejar pendiente",
        };
        match self.resolve(args) {
            Ok(task) => format!("{action} «{}»", task.text),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let status = status_arg(args)?;
        let note = note_arg(args, status)?;
        let task = self.resolve(args)?;
        let previous = task.status();
        let changed = self
            .0
            .set_status(task.id, status, note.as_deref())
            .map_err(ToolError::Failed)?
            .ok_or_else(|| ToolError::Failed(format!("No existe la tarea #{}.", task.id)))?;
        let pending = self.0.pending_count();
        let summary = match status {
            Status::Done => format!("Marqué como hecha «{}». Te quedan {pending} pendiente(s).", changed.text),
            Status::Partial => match &changed.note {
                Some(note) => format!("Marqué como parcial «{}» (falta: {note}). Tienes {pending} pendiente(s).", changed.text),
                None => format!("Marqué como parcial «{}». Tienes {pending} pendiente(s).", changed.text),
            },
            Status::Pending => format!("Volví a dejar pendiente «{}». Tienes {pending} pendiente(s).", changed.text),
        };
        Ok(ToolOutput {
            summary,
            data: json!({
                "id": changed.id, "text": changed.text, "done": changed.done, "partial": changed.partial,
                "status": status.key(), "previous": previous.key(), "note": changed.note, "pending": pending
            }),
        })
    }
}

impl CompleteTask {
    fn resolve(&self, args: &Value) -> Result<Task, ToolError> {
        let (id, text) = target_args(args, &["status", "note", "done"])?;
        // Al marcar como hecha o como parcial se busca entre lo que aún queda; al volver a pendiente, entre lo ya
        // avanzado (hechas y parciales).
        let status = status_arg(args)?;
        self.0.find(id, text.as_deref(), |task| if status == Status::Pending { task.done || task.partial } else { !task.done }).map_err(ToolError::Failed)
    }
}

pub struct DeleteTask(pub Arc<Tasks>);

impl DeleteTask {
    fn resolve(&self, args: &Value) -> Result<Task, ToolError> {
        let (id, text) = target_args(args, &[])?;
        self.0.find(id, text.as_deref(), |_| true).map_err(ToolError::Failed)
    }
}

impl Tool for DeleteTask {
    fn name(&self) -> &'static str {
        "delete_task"
    }

    fn description(&self) -> &'static str {
        "Borra una tarea de la lista (hecha o pendiente) por su número o por parte de su texto. Para tacharla como \
         terminada usa complete_task; esta es para quitarla del todo."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer", "minimum": 1, "description": "Número de la tarea." },
                "text": { "type": "string", "description": "Parte del texto de la tarea (alternativa al número)." }
            }
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        target_args(args, &[]).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.resolve(args) {
            Ok(task) => format!("Borrar la tarea «{}»", task.text),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let task = self.resolve(args)?;
        self.0.remove(task.id);
        Ok(ToolOutput { summary: format!("Borré la tarea «{}».", task.text), data: json!({ "id": task.id, "text": task.text }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn store(t: &TempLocations) -> Arc<Tasks> {
        Arc::new(Tasks::open(t.locations_root("data")))
    }

    #[test]
    fn agrega_numera_y_sobrevive_a_cerrar_y_abrir() {
        let t = temp_locations();
        {
            let tasks = store(&t);
            assert_eq!(tasks.add("  R38838 -   ESE ").unwrap().text, "R38838 - ESE", "se normalizan los espacios");
            tasks.add("Llamar al dentista ñ ✔").unwrap();
            tasks.set_done(1, true).unwrap();
        }
        let reopened = store(&t);
        let items = reopened.list();
        assert_eq!(items.iter().map(|x| (x.id, x.text.as_str(), x.done)).collect::<Vec<_>>(), [(1, "R38838 - ESE", true), (2, "Llamar al dentista ñ ✔", false)]);
        assert_eq!(reopened.add("otra").unwrap().id, 3);
        reopened.remove(3).unwrap();
        assert_eq!(reopened.add("más").unwrap().id, 4, "un número borrado no se reutiliza");
    }

    #[test]
    fn rechaza_vacias_largas_y_repetidas_pendientes() {
        let t = temp_locations();
        let tasks = store(&t);
        for bad in ["", "   ", "a\u{7}b", &"x".repeat(MAX_TEXT_CHARS + 1)] {
            assert!(tasks.add(bad).is_err(), "{bad:?}");
        }
        tasks.add("Pagar luz").unwrap();
        assert!(tasks.add("pagar LUZ").unwrap_err().contains("ya tienes pendiente"));
        tasks.set_done(1, true).unwrap();
        assert!(tasks.add("pagar luz").is_ok(), "una ya hecha no impide repetirla");
    }

    #[test]
    fn con_la_lista_llena_descarta_las_hechas_mas_antiguas() {
        let t = temp_locations();
        let tasks = store(&t);
        for i in 0..MAX_TASKS {
            tasks.add(&format!("t{i}")).unwrap();
        }
        assert!(tasks.add("una más").unwrap_err().contains("borra o completa"));
        tasks.set_done(3, true).unwrap();
        tasks.set_done(5, true).unwrap();
        tasks.add("una más").unwrap();
        let ids: Vec<u64> = tasks.list().iter().map(|x| x.id).collect();
        assert_eq!(ids.len(), MAX_TASKS);
        assert!(!ids.contains(&3) && ids.contains(&5), "se fue la hecha más antigua");
    }

    #[test]
    fn marcar_por_texto_exige_una_unica_coincidencia() {
        let t = temp_locations();
        let tasks = store(&t);
        tasks.add("R38838 - ESE").unwrap();
        tasks.add("R38999 - OTRO").unwrap();
        tasks.add("R38").unwrap();
        assert_eq!(tasks.find(None, Some("r38838"), |_| true).unwrap().id, 1);
        assert_eq!(tasks.find(None, Some("R38"), |_| true).unwrap().id, 3, "la coincidencia exacta gana a las parciales");
        assert!(tasks.find(None, Some("R3"), |_| true).unwrap_err().contains("varias"));
        assert!(tasks.find(None, Some("zzz"), |_| true).unwrap_err().contains("no encuentro"));
        assert!(tasks.find(None, Some("  "), |_| true).is_err());
        assert!(tasks.find(Some(9), None, |_| true).is_err());
        assert!(tasks.find(Some(1), Some("x"), |_| true).is_err());
    }

    #[test]
    fn las_herramientas_agregan_listan_completan_y_borran() {
        let t = temp_locations();
        let tasks = store(&t);
        let (add, list, complete, delete) = (AddTask(tasks.clone()), ListTasks(tasks.clone()), CompleteTask(tasks.clone()), DeleteTask(tasks.clone()));

        assert_eq!(list.execute(&json!({})).unwrap().summary, "No tienes tareas.");
        let added = add.execute(&json!({ "text": "R38838 - ESE" })).unwrap();
        assert_eq!(added.data["id"], 1);
        add.execute(&json!({ "text": "Enviar informe" })).unwrap();

        let listing = list.execute(&json!({})).unwrap();
        assert!(listing.summary.contains("2 tarea(s) pendiente(s) de 2") && listing.summary.contains("○ #1 R38838 - ESE"), "{}", listing.summary);

        // Por texto…
        let done = complete.execute(&json!({ "text": "R38838" })).unwrap();
        assert!(done.summary.contains("Marqué como hecha «R38838 - ESE»") && done.summary.contains("1 pendiente"), "{}", done.summary);
        assert_eq!(done.data["done"], true);
        // …no se vuelve a marcar una ya hecha (no está entre las pendientes)…
        assert!(matches!(complete.execute(&json!({ "text": "R38838" })), Err(ToolError::Failed(m)) if m.contains("no encuentro")));
        // …pero sí se puede reabrir.
        assert!(complete.execute(&json!({ "id": 1, "done": false })).unwrap().summary.contains("Volví a dejar pendiente"));
        complete.execute(&json!({ "id": 1 })).unwrap();

        let pending = list.execute(&json!({ "only_pending": true })).unwrap();
        assert!(pending.summary.contains("Enviar informe") && !pending.summary.contains("R38838"), "{}", pending.summary);
        assert_eq!(pending.data["tasks"].as_array().unwrap().len(), 1);

        assert!(delete.execute(&json!({ "text": "informe" })).unwrap().summary.contains("Borré la tarea «Enviar informe»"));
        assert_eq!(tasks.list().len(), 1);
        assert_eq!(tasks.clear_done(), 1);
        assert_eq!(list.execute(&json!({})).unwrap().summary, "No tienes tareas.");
    }

    #[test]
    fn valida_argumentos_y_riesgos() {
        let t = temp_locations();
        let tasks = store(&t);
        let (add, list, complete, delete) = (AddTask(tasks.clone()), ListTasks(tasks.clone()), CompleteTask(tasks.clone()), DeleteTask(tasks.clone()));
        for bad in [json!({}), json!({ "text": "" }), json!({ "text": 5 }), json!({ "text": "x", "otro": 1 })] {
            assert!(add.validate(&bad).is_err(), "{bad}");
        }
        for bad in [json!({}), json!({ "id": 1, "text": "x" }), json!({ "id": 0 }), json!({ "id": "1" }), json!({ "id": 1, "done": "sí" }), json!({ "id": 1, "x": 1 }),
            json!({ "id": 1, "status": "mitad" }), json!({ "id": 1, "status": "done", "note": "falta algo" }), json!({ "id": 1, "status": "partial", "done": false }),
            json!({ "id": 1, "status": "partial", "note": 5 }), json!({ "id": 1, "status": "partial", "note": "x".repeat(MAX_NOTE_CHARS + 1) }),
        ] {
            assert!(complete.validate(&bad).is_err(), "{bad}");
        }
        assert!(delete.validate(&json!({})).is_err() && delete.validate(&json!({ "id": 1, "text": "x" })).is_err());
        assert!(list.validate(&json!({ "only_pending": "sí" })).is_err());
        assert_eq!((add.risk(&json!({})), list.risk(&json!({})), complete.risk(&json!({}))), (RiskLevel::Safe, RiskLevel::Safe, RiskLevel::Safe));
        assert_eq!(delete.risk(&json!({})), RiskLevel::Confirm, "borrar pide confirmación");
        assert!(add.preview(&json!({ "text": "R38838 - ESE" })).contains("«R38838 - ESE»"));
    }

    #[test]
    fn un_archivo_corrupto_o_manipulado_no_rompe_nada() {
        let t = temp_locations();
        let path = t.locations_root("data").join("tasks.json");
        std::fs::write(&path, "esto no es json").unwrap();
        assert!(store(&t).list().is_empty());
        let weird = json!({ "next_id": 1, "items": [
            { "id": 7, "text": "buena", "done": true }, { "id": "x", "text": "sin id" },
            { "id": 9, "text": "con\u{7}control" }, { "id": 10 }, { "id": 11, "text": "sin done" }
        ]});
        std::fs::write(&path, weird.to_string()).unwrap();
        let tasks = store(&t);
        assert_eq!(tasks.list().iter().map(|x| (x.id, x.done)).collect::<Vec<_>>(), [(7, true), (11, false)]);
        assert_eq!(tasks.add("nueva").unwrap().id, 12, "el siguiente número supera a los existentes");
    }

    #[test]
    fn una_tarea_parcial_sigue_pendiente_y_guarda_su_nota() {
        let t = temp_locations();
        let tasks = store(&t);
        tasks.add("R38838 - ESE").unwrap();
        tasks.add("Enviar informe").unwrap();
        tasks.add("Llamar al dentista").unwrap();

        let partial = tasks.set_status(1, Status::Partial, Some("  falta   la firma ")).unwrap().unwrap();
        assert_eq!((partial.done, partial.partial, partial.note.as_deref(), partial.status()), (false, true, Some("falta la firma"), Status::Partial));
        assert_eq!((tasks.pending_count(), tasks.partial_count()), (3, 1), "una parcial aún cuenta como pendiente");
        assert!(tasks.add("r38838 - ese").unwrap_err().contains("ya tienes pendiente"), "una parcial no se puede repetir");

        // Sin nota nueva se conserva la anterior; con nota nueva se reemplaza.
        assert_eq!(tasks.set_status(1, Status::Partial, None).unwrap().unwrap().note.as_deref(), Some("falta la firma"));
        assert_eq!(tasks.set_status(1, Status::Partial, Some("falta enviarla")).unwrap().unwrap().note.as_deref(), Some("falta enviarla"));
        // «Quitar las hechas» no toca las parciales.
        tasks.set_done(2, true).unwrap();
        assert_eq!(tasks.clear_done(), 1);
        assert!(tasks.list().iter().any(|x| x.id == 1 && x.partial));

        // Terminarla o reabrirla borra la nota.
        let done = tasks.set_status(1, Status::Done, None).unwrap().unwrap();
        assert_eq!((done.done, done.partial, done.note), (true, false, None));
        tasks.set_status(1, Status::Partial, Some("otra vez")).unwrap();
        let pending = tasks.set_status(1, Status::Pending, None).unwrap().unwrap();
        assert_eq!((pending.done, pending.partial, pending.note), (false, false, None));
        assert!(tasks.set_status(99, Status::Partial, None).unwrap().is_none());
        assert!(tasks.set_status(1, Status::Partial, Some(&"x".repeat(MAX_NOTE_CHARS + 1))).is_err());
        assert!(tasks.set_status(1, Status::Partial, Some("con\u{7}control")).is_err());
    }

    #[test]
    fn el_estado_parcial_sobrevive_a_cerrar_y_abrir_y_los_archivos_antiguos_se_leen() {
        let t = temp_locations();
        {
            let tasks = store(&t);
            tasks.add("Informe").unwrap();
            tasks.add("Factura").unwrap();
            tasks.set_status(1, Status::Partial, Some("falta el anexo")).unwrap();
            tasks.set_done(2, true).unwrap();
        }
        let reopened = store(&t);
        let items = reopened.list();
        assert_eq!((items[0].status(), items[0].note.as_deref()), (Status::Partial, Some("falta el anexo")));
        assert_eq!((items[1].status(), items[1].note.as_deref()), (Status::Done, None));
        // Un archivo guardado antes de existir «parcial» (sin esos campos) sigue funcionando, y los datos raros se sanean.
        let path = t.locations_root("data").join("tasks.json");
        let old = json!({ "next_id": 5, "items": [
            { "id": 1, "text": "antigua", "done": false },
            { "id": 2, "text": "hecha y parcial a la vez", "done": true, "partial": true, "note": "no debe quedar" },
            { "id": 3, "text": "nota rara", "done": false, "partial": true, "note": 7 },
            { "id": 4, "text": "nota en pendiente", "done": false, "partial": false, "note": "se descarta" }
        ]});
        std::fs::write(&path, old.to_string()).unwrap();
        let tasks = store(&t);
        let loaded = tasks.list();
        assert_eq!(loaded.iter().map(Task::status).collect::<Vec<_>>(), [Status::Pending, Status::Done, Status::Partial, Status::Pending]);
        assert!(loaded.iter().all(|x| x.note.is_none()), "la nota solo vive en las parciales y debe ser texto");
    }

    #[test]
    fn la_herramienta_marca_parcial_lista_y_deja_deshacer() {
        let t = temp_locations();
        let tasks = store(&t);
        let (add, list, complete) = (AddTask(tasks.clone()), ListTasks(tasks.clone()), CompleteTask(tasks.clone()));
        add.execute(&json!({ "text": "R38838 - ESE" })).unwrap();
        add.execute(&json!({ "text": "Enviar informe" })).unwrap();

        let partial = complete.execute(&json!({ "text": "R38838", "status": "partial", "note": "falta la firma" })).unwrap();
        assert!(partial.summary.contains("Marqué como parcial «R38838 - ESE» (falta: falta la firma)") && partial.summary.contains("2 pendiente"), "{}", partial.summary);
        assert_eq!((partial.data["status"].as_str(), partial.data["previous"].as_str(), partial.data["partial"].as_bool()), (Some("partial"), Some("pending"), Some(true)));
        assert!(complete.preview(&json!({ "id": 2, "status": "partial" })).starts_with("Marcar como parcial «Enviar informe»"));

        let listing = list.execute(&json!({})).unwrap();
        assert!(listing.summary.contains("2 tarea(s) pendiente(s) de 2, 1 parcial(es)"), "{}", listing.summary);
        assert!(listing.summary.contains("◐ #1 R38838 - ESE — falta la firma") && listing.summary.contains("○ #2 Enviar informe"), "{}", listing.summary);
        let row = &listing.data["tasks"][0];
        assert_eq!((row["status"].as_str(), row["note"].as_str(), row["partial"].as_bool()), (Some("partial"), Some("falta la firma"), Some(true)));
        assert_eq!(list.execute(&json!({ "only_pending": true })).unwrap().data["tasks"].as_array().unwrap().len(), 2, "las parciales salen entre lo pendiente");

        // Una parcial se puede terminar (por texto: sigue entre lo que queda) y el cambio dice de dónde venía.
        let done = complete.execute(&json!({ "text": "R38838" })).unwrap();
        assert_eq!((done.data["status"].as_str(), done.data["previous"].as_str(), done.data["note"].is_null()), (Some("done"), Some("partial"), true));
        // Y volver atrás: de hecha a parcial, y de parcial a pendiente (se busca entre lo ya avanzado).
        complete.execute(&json!({ "id": 1, "status": "partial", "note": "otra vez" })).unwrap();
        assert!(complete.execute(&json!({ "text": "R38838", "status": "pending" })).unwrap().summary.contains("Volví a dejar pendiente"));
        assert!(matches!(complete.execute(&json!({ "text": "Enviar", "status": "pending" })), Err(ToolError::Failed(m)) if m.contains("no encuentro")), "una sin empezar no se «reabre»");
        // Lo antiguo sigue valiendo: done=false deja la tarea pendiente.
        complete.execute(&json!({ "id": 2, "status": "partial" })).unwrap();
        assert_eq!(complete.execute(&json!({ "id": 2, "done": false })).unwrap().data["status"], "pending");
    }
}
