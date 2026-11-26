//! Recordatorios y temporizadores. Se guardan en disco, así que sobreviven a cerrar la aplicación;
//! quien los dispara (un hilo en la capa de la aplicación) solo llama a `take_due`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use chrono::{Datelike, Days, Local, TimeZone};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, required_str};
use crate::clock::{describe_due, seconds_until, unix_now, LocalNow};

pub const MAX_PENDING: usize = 50;
const MAX_TEXT_CHARS: usize = 200;
const MIN_DELAY_SECS: u64 = 5;
const MAX_DELAY_SECS: u64 = 7 * 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: u64,
    /// Instante de vencimiento, en segundos desde 1970 (UTC).
    pub due: u64,
    pub text: String,
    pub recurrence: Option<Recurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Recurrence {
    Interval { seconds: u64 },
    Daily { hour: u8, minute: u8 },
    Weekdays { hour: u8, minute: u8 },
    Weekly { hour: u8, minute: u8, weekday: u8 },
}

impl Recurrence {
    fn valid(&self) -> bool {
        match *self {
            Self::Interval { seconds } => (60..=MAX_DELAY_SECS).contains(&seconds),
            Self::Daily { hour, minute } | Self::Weekdays { hour, minute } => hour < 24 && minute < 60,
            Self::Weekly { hour, minute, weekday } => hour < 24 && minute < 60 && (1..=7).contains(&weekday),
        }
    }

    /// Primera ocurrencia estrictamente posterior a now. No reproduce los intervalos perdidos.
    fn next_after(&self, previous: u64, now: u64) -> Option<u64> {
        if !self.valid() { return None; }
        if let Self::Interval { seconds } = *self {
            let jumps = now.saturating_sub(previous) / seconds + 1;
            return previous.checked_add(jumps.checked_mul(seconds)?);
        }
        let local = Local.timestamp_opt(i64::try_from(now).ok()?, 0).single()?;
        let (hour, minute) = match *self {
            Self::Daily { hour, minute } | Self::Weekdays { hour, minute } | Self::Weekly { hour, minute, .. } => (hour, minute),
            _ => unreachable!(),
        };
        for offset in 0..=15 {
            let date = local.date_naive().checked_add_days(Days::new(offset))?;
            let weekday = date.weekday().number_from_monday() as u8;
            if matches!(self, Self::Weekdays { .. }) && weekday > 5 { continue; }
            if let Self::Weekly { weekday: target, .. } = *self {
                if weekday != target { continue; }
            }
            let naive = date.and_hms_opt(hour as u32, minute as u32, 0)?;
            // En una hora repetida se usa la primera ocurrencia; una hora inexistente se omite.
            if let Some(next) = Local.from_local_datetime(&naive).earliest() {
                if next.timestamp() > now as i64 { return u64::try_from(next.timestamp()).ok(); }
            }
        }
        None
    }

    pub fn label(&self) -> String {
        match *self {
            Self::Interval { seconds } => format!("Cada {}", crate::describe_span(seconds)),
            Self::Daily { .. } => "Todos los días".into(),
            Self::Weekdays { .. } => "De lunes a viernes".into(),
            Self::Weekly { weekday, .. } => format!("Cada {}", ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"][weekday.saturating_sub(1) as usize % 7]),
        }
    }
}

#[derive(Default, Clone)]
struct State {
    next_id: u64,
    items: Vec<Reminder>,
}

pub struct Reminders {
    path: PathBuf,
    state: Mutex<State>,
}

/// Texto limpio de un recordatorio, o el motivo por el que no vale.
fn clean_text(text: &str) -> Result<String, String> {
    let text = text.trim();
    let length = text.chars().count();
    if length == 0 || length > MAX_TEXT_CHARS {
        return Err(format!("el texto debe tener entre 1 y {MAX_TEXT_CHARS} caracteres"));
    }
    if text.chars().any(char::is_control) {
        return Err("el texto no puede tener saltos de línea ni caracteres de control".into());
    }
    Ok(text.to_string())
}

impl Reminders {
    /// Abre (o crea) el almacén en `dir`. Un archivo ilegible se trata como vacío.
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join("reminders.json");
        let state = Self::load(&path).unwrap_or_default();
        Self { path, state: Mutex::new(state) }
    }

    fn load(path: &PathBuf) -> Option<State> {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        let items: Vec<Reminder> = value["items"]
            .as_array()?
            .iter()
            .filter_map(|item| {
                let text = clean_text(item["text"].as_str()?).ok()?;
                let recurrence = if item["recurrence"].is_null() { None } else {
                    let rule: Recurrence = serde_json::from_value(item["recurrence"].clone()).ok()?;
                    if !rule.valid() { return None; }
                    Some(rule)
                };
                Some(Reminder { id: item["id"].as_u64()?, due: item["due"].as_u64()?, text, recurrence })
            })
            .take(MAX_PENDING)
            .collect();
        let floor = items.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Some(State { next_id: value["next_id"].as_u64().unwrap_or(1).max(floor), items })
    }

    fn save(&self, state: &State) -> Result<(), String> {
        let items: Vec<Value> = state.items.iter().map(|r| json!({ "id": r.id, "due": r.due, "text": r.text, "recurrence": r.recurrence })).collect();
        let body = serde_json::to_string_pretty(&json!({ "next_id": state.next_id, "items": items })).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear la carpeta de datos: {e}"))?;
        }
        // Se escribe a un archivo temporal y se renombra, para no dejar un JSON a medias si se corta la luz.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, body).map_err(|e| format!("no se pudieron guardar los recordatorios: {e}"))?;
        std::fs::rename(&temp, &self.path).map_err(|e| format!("no se pudieron guardar los recordatorios: {e}"))
    }

    pub fn add(&self, text: &str, due: u64) -> Result<Reminder, String> {
        self.add_recurring(text, due, None)
    }

    fn add_recurring(&self, text: &str, due: u64, recurrence: Option<Recurrence>) -> Result<Reminder, String> {
        let text = clean_text(text)?;
        let mut state = self.state.lock().expect("recordatorios envenenado");
        if state.items.len() >= MAX_PENDING {
            return Err(format!("ya hay {MAX_PENDING} recordatorios pendientes; cancela alguno antes de crear más"));
        }
        state.next_id = state.next_id.max(1);
        let reminder = Reminder { id: state.next_id, due, text, recurrence };
        state.next_id += 1;
        state.items.push(reminder.clone());
        if let Err(error) = self.save(&state) {
            state.items.pop();
            return Err(error);
        }
        Ok(reminder)
    }

    /// Pendientes, del que vence antes al que vence después.
    pub fn list(&self) -> Vec<Reminder> {
        let mut items = self.state.lock().expect("recordatorios envenenado").items.clone();
        items.sort_by_key(|r| (r.due, r.id));
        items
    }

    pub fn cancel(&self, id: u64) -> Option<Reminder> {
        let mut state = self.state.lock().expect("recordatorios envenenado");
        let position = state.items.iter().position(|r| r.id == id)?;
        let removed = state.items.remove(position);
        if self.save(&state).is_err() {
            state.items.insert(position, removed);
            return None;
        }
        Some(removed)
    }

    /// Crea un recordatorio con las mismas reglas que la herramienta (`text` y `delay_seconds` o `at`), para
    /// la interfaz. Devuelve el recordatorio y cuándo sonará, en palabras.
    pub fn create(&self, args: &Value) -> Result<(Reminder, String), String> {
        let (text, when) = parse_set(args).map_err(|error| error.to_string())?;
        let now = LocalNow::now();
        let delay = delay_for(&when, &now);
        let rule = parse_recurrence(args, &when).map_err(|e| e.to_string())?;
        let timestamp = unix_now();
        let due = match &rule {
            Some(rule) if !matches!(rule, Recurrence::Interval { .. }) => rule.next_after(timestamp, timestamp).ok_or("No pude calcular la próxima fecha local.")?,
            _ => timestamp + delay,
        };
        let reminder = self.add_recurring(&text, due, rule)?;
        let mut human = describe_reminder_due(&now, timestamp, due, reminder.recurrence.as_ref());
        if let Some(rule) = &reminder.recurrence { human.push_str(&format!(" · {}", rule.label())); }
        Ok((reminder, human))
    }

    /// Saca (y deja de recordar) los que ya vencieron a las `now`.
    pub fn take_due(&self, now: u64) -> Vec<Reminder> {
        let mut state = self.state.lock().expect("recordatorios envenenado");
        if !state.items.iter().any(|r| r.due <= now) {
            return Vec::new();
        }
        let original = state.clone();
        let (mut due, mut pending): (Vec<_>, Vec<_>) = std::mem::take(&mut state.items).into_iter().partition(|r| r.due <= now);
        for item in &due {
            if let Some(next) = item.recurrence.as_ref().and_then(|rule| rule.next_after(item.due, now)) {
                let mut next_item = item.clone();
                next_item.due = next;
                pending.push(next_item);
            }
        }
        state.items = pending;
        due.sort_by_key(|r| (r.due, r.id));
        if self.save(&state).is_err() {
            *state = original;
            return Vec::new();
        }
        due
    }
}

// ---------------------------------------------------------------------------------------------
// Herramientas
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum When {
    InSeconds(u64),
    /// Próxima vez que el reloj marque esa hora (formato 24 h).
    At { hour: u8, minute: u8 },
}

fn parse_clock(text: &str) -> Option<(u8, u8)> {
    let (hour, minute) = text.split_once(':')?;
    let digits = |s: &str, max_len: usize| (1..=max_len).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(hour, 2) || !digits(minute, 2) || minute.len() != 2 {
        return None;
    }
    let (hour, minute): (u8, u8) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

fn parse_set(args: &Value) -> Result<(String, When), ToolError> {
    check_keys(args, &["text", "delay_seconds", "at", "recurrence"])?;
    let text = clean_text(required_str(args, "text")?).map_err(invalid)?;
    let delay = args.get("delay_seconds").filter(|v| !v.is_null());
    let at = args.get("at").filter(|v| !v.is_null());
    let when = match (delay, at) {
        (Some(delay), None) => match delay.as_u64() {
            Some(secs) if (MIN_DELAY_SECS..=MAX_DELAY_SECS).contains(&secs) => When::InSeconds(secs),
            _ => return Err(invalid(format!("«delay_seconds» debe ser un entero entre {MIN_DELAY_SECS} y {MAX_DELAY_SECS} (7 días)"))),
        },
        (None, Some(at)) => {
            let (hour, minute) = at.as_str().and_then(parse_clock).ok_or_else(|| invalid("«at» debe ser una hora 24 h con el formato HH:MM, por ejemplo 17:30"))?;
            When::At { hour, minute }
        }
        _ => return Err(invalid("indica exactamente uno: «delay_seconds» (dentro de cuánto) o «at» (a qué hora)")),
    };
    parse_recurrence(args, &when)?;
    Ok((text, when))
}

fn parse_recurrence(args: &Value, when: &When) -> Result<Option<Recurrence>, ToolError> {
    let Some(rule) = args.get("recurrence").filter(|r| !r.is_null()) else { return Ok(None) };
    check_keys(rule, &["kind", "seconds", "weekday"])?;
    let kind = required_str(rule, "kind")?;
    if kind == "interval" {
        let seconds = rule["seconds"].as_u64().ok_or_else(|| invalid("Indica los segundos del intervalo."))?;
        let recurrence = Recurrence::Interval { seconds };
        if !recurrence.valid() || !matches!(when, When::InSeconds(_)) || rule.get("weekday").is_some() {
            return Err(invalid("Un intervalo requiere delay_seconds y seconds entre 60 y 604800, sin weekday."));
        }
        return Ok(Some(recurrence));
    }
    let When::At { hour, minute } = *when else { return Err(invalid("Para repetir por días indica at (HH:MM).")); };
    if rule.get("seconds").is_some() { return Err(invalid("seconds solo vale con interval.")); }
    let recurrence = match kind {
        "daily" if rule.get("weekday").is_none() => Recurrence::Daily { hour, minute },
        "weekdays" if rule.get("weekday").is_none() => Recurrence::Weekdays { hour, minute },
        "weekly" => Recurrence::Weekly { hour, minute, weekday: rule["weekday"].as_u64().filter(|d| (1..=7).contains(d)).ok_or_else(|| invalid("weekday debe ser 1 (lunes) a 7 (domingo)."))? as u8 },
        _ => return Err(invalid("recurrence.kind debe ser interval, daily, weekdays o weekly.")),
    };
    Ok(Some(recurrence))
}

fn delay_for(when: &When, now: &LocalNow) -> u64 {
    match *when {
        When::InSeconds(secs) => secs,
        When::At { hour, minute } => seconds_until(now, hour, minute),
    }
}

fn describe_reminder_due(now: &LocalNow, timestamp: u64, due: u64, rule: Option<&Recurrence>) -> String {
    if rule.is_some_and(|r| !matches!(r, Recurrence::Interval { .. })) {
        if let Some(local) = i64::try_from(due).ok().and_then(|time| Local.timestamp_opt(time, 0).single()) {
            return format!("el {} (en {})", local.format("%d/%m a las %H:%M"), crate::describe_span(due.saturating_sub(timestamp)));
        }
    }
    describe_due(now, due.saturating_sub(timestamp))
}

pub struct SetReminder {
    reminders: Arc<Reminders>,
}

impl SetReminder {
    pub fn new(reminders: Arc<Reminders>) -> Self {
        Self { reminders }
    }
}

impl Tool for SetReminder {
    fn name(&self) -> &'static str {
        "set_reminder"
    }

    fn description(&self) -> &'static str {
        "Programa un recordatorio o temporizador: la mascota avisará con un sonido, un mensaje en el chat y \
         moviéndose cuando llegue el momento. Usa «delay_seconds» para «dentro de X» (convierte minutos u \
         horas a segundos; de 5 s a 7 días) o «at» (HH:MM, 24 h) para una hora concreta; si esa hora ya \
         pasó hoy, será mañana. Para repetir usa recurrence: interval con seconds (60 a 604800) y delay_seconds; \
         daily o weekdays con at; weekly con at y weekday (1 lunes a 7 domingo). «text» es lo que se recordará."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Qué recordar, por ejemplo «Hacer un descanso»." },
                "delay_seconds": { "type": "integer", "minimum": MIN_DELAY_SECS, "maximum": MAX_DELAY_SECS, "description": "Segundos desde ahora." },
                "at": { "type": "string", "description": "Hora local en formato HH:MM de 24 horas, por ejemplo 17:30." },
                "recurrence": { "type": "object", "properties": {
                    "kind": { "type": "string", "enum": ["interval", "daily", "weekdays", "weekly"] },
                    "seconds": { "type": "integer", "minimum": 60, "maximum": MAX_DELAY_SECS },
                    "weekday": { "type": "integer", "minimum": 1, "maximum": 7 }
                }, "required": ["kind"], "additionalProperties": false }
            },
            "required": ["text"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_set(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_set(args) {
            Ok((text, when)) => {
                let now = LocalNow::now();
                let timestamp = unix_now();
                let rule = parse_recurrence(args, &when).ok().flatten();
                let due = match &rule {
                    Some(rule) if !matches!(rule, Recurrence::Interval { .. }) => rule.next_after(timestamp, timestamp).unwrap_or(timestamp),
                    _ => timestamp + delay_for(&when, &now),
                };
                let repeat = rule.as_ref().map(|r| format!(" · {}", r.label())).unwrap_or_default();
                format!("Recordarte «{text}» {}{repeat}", describe_reminder_due(&now, timestamp, due, rule.as_ref()))
            },
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (reminder, human) = self.reminders.create(args).map_err(ToolError::Failed)?;
        Ok(ToolOutput {
            summary: format!("Listo, te lo recordaré {human}: «{}»", reminder.text),
            data: json!({ "id": reminder.id, "due_unix": reminder.due, "when": human, "text": reminder.text, "recurrence": reminder.recurrence }),
        })
    }
}

pub struct ListReminders {
    reminders: Arc<Reminders>,
}

impl ListReminders {
    pub fn new(reminders: Arc<Reminders>) -> Self {
        Self { reminders }
    }
}

impl Tool for ListReminders {
    fn name(&self) -> &'static str {
        "list_reminders"
    }

    fn description(&self) -> &'static str {
        "Lista los recordatorios y temporizadores pendientes, con su número, para poder cancelarlos."
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
        "Consultar los recordatorios pendientes".into()
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        check_keys(args, &[])?;
        let items = self.reminders.list();
        if items.is_empty() {
            return Ok(ToolOutput { summary: "No tienes recordatorios pendientes.".into(), data: json!({ "reminders": [] }) });
        }
        let (now, now_unix) = (LocalNow::now(), unix_now());
        let mut summary = format!("Tienes {} recordatorio(s) pendiente(s):", items.len());
        let mut data = Vec::new();
        for item in &items {
            let when = describe_reminder_due(&now, now_unix, item.due, item.recurrence.as_ref());
            let repeat = item.recurrence.as_ref().map(|r| format!(" · {}", r.label())).unwrap_or_default();
            summary.push_str(&format!("\n• #{} {when}{repeat}: {}", item.id, item.text));
            data.push(json!({ "id": item.id, "when": when, "due_unix": item.due, "text": item.text, "recurrence": item.recurrence }));
        }
        Ok(ToolOutput { summary, data: json!({ "reminders": data }) })
    }
}

pub struct CancelReminder {
    reminders: Arc<Reminders>,
}

impl CancelReminder {
    pub fn new(reminders: Arc<Reminders>) -> Self {
        Self { reminders }
    }
}

fn parse_cancel(args: &Value) -> Result<u64, ToolError> {
    check_keys(args, &["id"])?;
    match args.get("id").and_then(Value::as_u64) {
        Some(id) if id >= 1 => Ok(id),
        _ => Err(invalid("«id» debe ser el número del recordatorio (un entero positivo)")),
    }
}

impl Tool for CancelReminder {
    fn name(&self) -> &'static str {
        "cancel_reminder"
    }

    fn description(&self) -> &'static str {
        "Cancela un recordatorio pendiente por su número, incluidas sus próximas repeticiones si es recurrente (consúltalo con list_reminders)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "integer", "minimum": 1, "description": "Número del recordatorio." } },
            "required": ["id"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_cancel(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_cancel(args) {
            Ok(id) => format!("Cancelar el recordatorio #{id}"),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let id = parse_cancel(args)?;
        let removed = self.reminders.cancel(id).ok_or_else(|| ToolError::Failed(format!("No existe el recordatorio #{id}.")))?;
        Ok(ToolOutput {
            summary: format!("Cancelé el recordatorio #{id}: «{}»", removed.text),
            data: json!({ "id": id, "text": removed.text }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn store(t: &TempLocations) -> Arc<Reminders> {
        Arc::new(Reminders::open(t.locations_root("data")))
    }

    #[test]
    fn recurrencia_intervalo_salta_atrasos_persiste_y_se_cancela_completa() {
        let t = temp_locations(); let r = store(&t);
        let item = r.add_recurring("Agua", 100, Some(Recurrence::Interval { seconds: 120 })).unwrap();
        assert_eq!(r.take_due(10000).len(), 1);
        let next = r.list()[0].clone();
        assert_eq!(next.id, item.id);
        assert!(next.due > 10000 && next.due <= 10120);
        assert!(r.take_due(10000).is_empty());
        assert_eq!(store(&t).list()[0], next);
        r.cancel(item.id).unwrap();
        assert!(store(&t).take_due(100000).is_empty());
    }

    #[test]
    fn recurrencias_de_calendario_respetan_hora_y_dia_local() {
        use chrono::Timelike;
        let friday = Local.with_ymd_and_hms(2026, 11, 27, 10, 0, 0).earliest().unwrap().timestamp() as u64;
        let weekdays = Recurrence::Weekdays { hour: 9, minute: 30 };
        let next = Local.timestamp_opt(weekdays.next_after(friday, friday).unwrap() as i64, 0).single().unwrap();
        assert_eq!((next.weekday().number_from_monday(), next.hour(), next.minute()), (1, 9, 30));
        let weekly = Recurrence::Weekly { hour: 9, minute: 0, weekday: 5 };
        let next = Local.timestamp_opt(weekly.next_after(friday, friday).unwrap() as i64, 0).single().unwrap();
        assert_eq!((next.day(), next.weekday().number_from_monday(), next.hour()), (4, 5, 9));
        assert!(weekly.next_after(friday, friday + 40 * 86400).unwrap() > friday + 40 * 86400);
    }

    #[test]
    fn valida_recurrencia_tanto_para_ia_como_interfaz() {
        let t = temp_locations(); let r = store(&t);
        for rule in [json!({"kind":"daily"}), json!({"kind":"weekdays"}), json!({"kind":"weekly","weekday":5})] {
            assert!(r.create(&json!({"text":"Revisar","at":"09:00","recurrence":rule})).unwrap().0.recurrence.is_some());
        }
        for bad in [
            json!({"text":"x","at":"09:00","recurrence":{"kind":"interval","seconds":120}}),
            json!({"text":"x","delay_seconds":120,"recurrence":{"kind":"daily"}}),
            json!({"text":"x","at":"09:00","recurrence":{"kind":"weekly","weekday":8}}),
            json!({"text":"x","delay_seconds":120,"recurrence":{"kind":"interval","seconds":0}}),
            json!({"text":"x","at":"09:00","recurrence":{"kind":"daily","command":"bad"}}),
        ] { assert!(r.create(&bad).is_err(), "{bad}"); }
        assert_eq!(r.list().len(), 3);
    }

    #[test]
    fn si_falla_el_guardado_no_se_pierden_avisos_ni_cancelaciones() {
        let t = temp_locations(); let r = store(&t);
        let reminder = r.add("x", 100).unwrap();
        std::fs::create_dir(r.path.with_extension("json.tmp")).unwrap();
        assert!(r.take_due(100).is_empty());
        assert_eq!(r.list()[0].id, reminder.id);
        assert!(r.cancel(reminder.id).is_none());
        assert_eq!(r.list().len(), 1);
    }

    #[test]
    fn crear_desde_la_interfaz_sigue_las_mismas_reglas_que_la_herramienta() {
        let t = temp_locations();
        let r = store(&t);
        let (reminder, when) = r.create(&json!({ "text": "  Estirar  ", "delay_seconds": 1800 })).unwrap();
        assert_eq!(reminder.text, "Estirar");
        assert!(!when.is_empty() && r.list().len() == 1);
        assert!(r.create(&json!({ "text": "Hora", "at": "17:30" })).is_ok());
        for bad in [
            json!({ "text": "", "delay_seconds": 60 }),
            json!({ "text": "x", "delay_seconds": 1 }),
            json!({ "text": "x", "delay_seconds": 8 * 86_400 }),
            json!({ "text": "x", "at": "25:00" }),
            json!({ "text": "x" }),
            json!({ "text": "x", "delay_seconds": 60, "at": "10:00" }),
            json!({ "text": "x", "delay_seconds": 60, "otro": true }),
        ] {
            assert!(r.create(&bad).is_err(), "{bad}");
        }
        assert_eq!(r.list().len(), 2, "lo inválido no se guarda");
    }

    #[test]
    fn guarda_ordena_y_numera_sin_repetir_ids() {
        let t = temp_locations();
        let r = store(&t);
        let late = r.add("tarde", 2_000).unwrap();
        let early = r.add("pronto", 1_000).unwrap();
        assert_eq!((late.id, early.id), (1, 2));
        assert_eq!(r.list().iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["pronto", "tarde"]);
        r.cancel(2).unwrap();
        assert_eq!(r.add("nuevo", 3_000).unwrap().id, 3, "un id cancelado no se reutiliza");
    }

    #[test]
    fn sobrevive_a_cerrar_y_abrir() {
        let t = temp_locations();
        {
            let r = store(&t);
            r.add("uno", 500).unwrap();
            r.add("dos con tilde ñ ✔", 600).unwrap();
            r.cancel(1).unwrap();
        }
        let reopened = store(&t);
        let items = reopened.list();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].id, items[0].text.as_str(), items[0].due), (2, "dos con tilde ñ ✔", 600));
        assert_eq!(reopened.add("tres", 700).unwrap().id, 3);
    }

    #[test]
    fn entrega_solo_lo_vencido_una_unica_vez() {
        let t = temp_locations();
        let r = store(&t);
        r.add("a", 100).unwrap();
        r.add("b", 200).unwrap();
        r.add("c", 300).unwrap();
        assert!(r.take_due(99).is_empty());
        let due = r.take_due(200);
        assert_eq!(due.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(r.take_due(200).is_empty(), "no se entregan dos veces");
        assert_eq!(r.list().len(), 1);
        // También deja de estar en disco: tras reabrir no vuelve a avisar.
        assert_eq!(store(&t).list().len(), 1);
        assert_eq!(store(&t).take_due(1_000).len(), 1);
    }

    #[test]
    fn un_recordatorio_vencido_con_la_app_cerrada_se_entrega_al_abrir() {
        let t = temp_locations();
        store(&t).add("tomar la pastilla", 1_000).unwrap();
        let due = store(&t).take_due(1_000_000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].text, "tomar la pastilla");
    }

    #[test]
    fn respeta_el_limite_y_valida_el_texto() {
        let t = temp_locations();
        let r = store(&t);
        for i in 0..MAX_PENDING {
            r.add(&format!("r{i}"), 1_000 + i as u64).unwrap();
        }
        assert!(r.add("uno más", 5).unwrap_err().contains("cancela alguno"));
        assert_eq!(r.list().len(), MAX_PENDING);
        let empty = store(&temp_locations());
        for bad in ["", "   ", "a\nb", "x\u{7}y", &"x".repeat(MAX_TEXT_CHARS + 1)] {
            assert!(empty.add(bad, 1).is_err(), "{bad:?}");
        }
        assert_eq!(empty.add("  con espacios  ", 1).unwrap().text, "con espacios");
    }

    #[test]
    fn un_archivo_corrupto_o_manipulado_no_rompe_nada() {
        let t = temp_locations();
        let path = t.locations_root("data").join("reminders.json");
        std::fs::write(&path, "esto no es json").unwrap();
        assert!(store(&t).list().is_empty());
        let weird = json!({ "next_id": 1, "items": [
            { "id": 7, "due": 10, "text": "bueno" }, { "id": 8, "due": "x", "text": "sin fecha" },
            { "id": 9, "due": 10, "text": "con\nsalto" }, { "id": 10, "due": 10 }
        ]});
        std::fs::write(&path, weird.to_string()).unwrap();
        let r = store(&t);
        assert_eq!(r.list().len(), 1);
        assert_eq!(r.add("nuevo", 1).unwrap().id, 8, "el siguiente id supera a los existentes aunque next_id sea menor");
    }

    #[test]
    fn valida_los_argumentos_de_set_reminder() {
        let ok = parse_set(&json!({ "text": "descanso", "delay_seconds": 1200 })).unwrap();
        assert_eq!(ok, ("descanso".to_string(), When::InSeconds(1200)));
        assert_eq!(parse_set(&json!({ "text": "x", "at": "17:30" })).unwrap().1, When::At { hour: 17, minute: 30 });
        assert_eq!(parse_set(&json!({ "text": "x", "at": "7:05" })).unwrap().1, When::At { hour: 7, minute: 5 });
        for bad in [
            json!({ "text": "x" }), json!({ "delay_seconds": 60 }), json!({ "text": "x", "delay_seconds": 60, "at": "10:00" }),
            json!({ "text": "x", "delay_seconds": 4 }), json!({ "text": "x", "delay_seconds": MAX_DELAY_SECS + 1 }),
            json!({ "text": "x", "delay_seconds": -5 }), json!({ "text": "x", "delay_seconds": "60" }), json!({ "text": "x", "delay_seconds": 1.5 }),
            json!({ "text": "x", "at": "24:00" }), json!({ "text": "x", "at": "12:60" }), json!({ "text": "x", "at": "12:5" }),
            json!({ "text": "x", "at": "1230" }), json!({ "text": "x", "at": "12:30:15" }), json!({ "text": "x", "at": "las 5" }),
            json!({ "text": "x", "at": 1730 }), json!({ "text": "", "delay_seconds": 60 }), json!({ "text": "x", "delay_seconds": 60, "otro": 1 }),
        ] {
            assert!(parse_set(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn las_herramientas_crean_listan_y_cancelan() {
        let t = temp_locations();
        let r = store(&t);
        let (set, list, cancel) = (SetReminder::new(r.clone()), ListReminders::new(r.clone()), CancelReminder::new(r.clone()));

        assert_eq!(list.execute(&json!({})).unwrap().summary, "No tienes recordatorios pendientes.");
        let before = unix_now();
        let created = set.execute(&json!({ "text": "Hacer un descanso", "delay_seconds": 1200 })).unwrap();
        let due = created.data["due_unix"].as_u64().unwrap();
        assert!((before + 1200..=unix_now() + 1200).contains(&due), "vence en 20 minutos");
        assert!(created.summary.contains("«Hacer un descanso»") && created.summary.contains("(en 20 min)"), "{}", created.summary);

        let at = set.execute(&json!({ "text": "Reunión", "at": "09:15" })).unwrap();
        let wait = at.data["due_unix"].as_u64().unwrap() - unix_now();
        assert!((1..=86_400).contains(&wait), "la próxima vez que sean las 09:15 queda a menos de 24 h");

        let listing = list.execute(&json!({})).unwrap();
        assert!(listing.summary.contains("2 recordatorio(s)") && listing.summary.contains("#1") && listing.summary.contains("Reunión"));
        assert_eq!(listing.data["reminders"].as_array().unwrap().len(), 2);

        assert!(cancel.execute(&json!({ "id": 1 })).unwrap().summary.contains("Cancelé el recordatorio #1"));
        assert!(matches!(cancel.execute(&json!({ "id": 1 })), Err(ToolError::Failed(m)) if m.contains("No existe")));
        for bad in [json!({}), json!({ "id": 0 }), json!({ "id": -1 }), json!({ "id": "1" }), json!({ "id": 1, "x": 1 })] {
            assert!(cancel.validate(&bad).is_err(), "{bad}");
        }
        assert_eq!(r.list().len(), 1);
    }

    #[test]
    fn son_seguras_y_la_vista_previa_dice_cuando() {
        let t = temp_locations();
        let r = store(&t);
        for risk in [
            SetReminder::new(r.clone()).risk(&json!({})), ListReminders::new(r.clone()).risk(&json!({})), CancelReminder::new(r.clone()).risk(&json!({})),
        ] {
            assert_eq!(risk, RiskLevel::Safe);
        }
        let preview = SetReminder::new(r).preview(&json!({ "text": "Agua", "delay_seconds": 600 }));
        assert!(preview.starts_with("Recordarte «Agua» ") && preview.contains("(en 10 min)"), "{preview}");
    }
}
