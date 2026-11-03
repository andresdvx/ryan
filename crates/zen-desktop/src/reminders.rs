//! Recordatorios y temporizadores. Se guardan en disco, así que sobreviven a cerrar la aplicación;
//! quien los dispara (un hilo en la capa de la aplicación) solo llama a `take_due`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
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
}

#[derive(Default)]
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
                Some(Reminder { id: item["id"].as_u64()?, due: item["due"].as_u64()?, text })
            })
            .take(MAX_PENDING)
            .collect();
        let floor = items.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Some(State { next_id: value["next_id"].as_u64().unwrap_or(1).max(floor), items })
    }

    fn save(&self, state: &State) -> Result<(), String> {
        let items: Vec<Value> = state.items.iter().map(|r| json!({ "id": r.id, "due": r.due, "text": r.text })).collect();
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
        let text = clean_text(text)?;
        let mut state = self.state.lock().expect("recordatorios envenenado");
        if state.items.len() >= MAX_PENDING {
            return Err(format!("ya hay {MAX_PENDING} recordatorios pendientes; cancela alguno antes de crear más"));
        }
        state.next_id = state.next_id.max(1);
        let reminder = Reminder { id: state.next_id, due, text };
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
        let _ = self.save(&state);
        Some(removed)
    }

    /// Saca (y deja de recordar) los que ya vencieron a las `now`.
    pub fn take_due(&self, now: u64) -> Vec<Reminder> {
        let mut state = self.state.lock().expect("recordatorios envenenado");
        if !state.items.iter().any(|r| r.due <= now) {
            return Vec::new();
        }
        let (mut due, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut state.items).into_iter().partition(|r| r.due <= now);
        state.items = pending;
        due.sort_by_key(|r| (r.due, r.id));
        let _ = self.save(&state);
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
    check_keys(args, &["text", "delay_seconds", "at"])?;
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
    Ok((text, when))
}

fn delay_for(when: &When, now: &LocalNow) -> u64 {
    match *when {
        When::InSeconds(secs) => secs,
        When::At { hour, minute } => seconds_until(now, hour, minute),
    }
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
         pasó hoy, será mañana. «text» es lo que se le recordará al usuario."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Qué recordar, por ejemplo «Hacer un descanso»." },
                "delay_seconds": { "type": "integer", "minimum": MIN_DELAY_SECS, "maximum": MAX_DELAY_SECS, "description": "Segundos desde ahora." },
                "at": { "type": "string", "description": "Hora local en formato HH:MM de 24 horas, por ejemplo 17:30." }
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
            Ok((text, when)) => format!("Recordarte «{text}» {}", describe_due(&LocalNow::now(), delay_for(&when, &LocalNow::now()))),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (text, when) = parse_set(args)?;
        let now = LocalNow::now();
        let delay = delay_for(&when, &now);
        let reminder = self.reminders.add(&text, unix_now() + delay).map_err(ToolError::Failed)?;
        let human = describe_due(&now, delay);
        Ok(ToolOutput {
            summary: format!("Listo, te lo recordaré {human}: «{}»", reminder.text),
            data: json!({ "id": reminder.id, "due_unix": reminder.due, "when": human, "text": reminder.text }),
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
            let when = describe_due(&now, item.due.saturating_sub(now_unix));
            summary.push_str(&format!("\n• #{} {when}: {}", item.id, item.text));
            data.push(json!({ "id": item.id, "when": when, "due_unix": item.due, "text": item.text }));
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
        "Cancela un recordatorio pendiente por su número (consúltalo con list_reminders)."
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
