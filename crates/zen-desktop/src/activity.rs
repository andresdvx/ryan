//! Registro de actividad: lo que ha hecho el asistente (qué herramienta, con qué datos, si salió bien) y, cuando se
//! puede, cómo deshacerlo. Se guarda en este equipo. Lo que podría contener contenido privado (texto escrito en
//! otra app, el portapapeles, el contenido de un archivo leído) se oculta antes de guardarlo.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use zen_core::assistant::ToolReport;

use crate::locations::{Locations, Placed};

pub const MAX_ENTRIES: usize = 500;
const MAX_STRING_CHARS: usize = 300;
const MAX_ITEMS: usize = 40;
const MAX_KEYS: usize = 30;
const MAX_DEPTH: usize = 4;
const MAX_SUMMARY_CHARS: usize = 600;
pub const HIDDEN: &str = "(oculto)";

/// Herramientas cuyos argumentos son texto que el usuario escribió o pegó: no se guardan.
const HIDE_ARGS: &[&str] = &["type_text", "write_clipboard", "press_keys"];
/// Herramientas cuyo resultado es contenido ajeno (un archivo, el portapapeles, una página): no se guarda.
const HIDE_DATA: &[&str] = &["read_file", "read_clipboard", "web_search", "list_controls", "list_windows", "look_at_screen"];

#[derive(Debug, Clone, PartialEq)]
pub struct Undo {
    pub tool: String,
    pub args: Value,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u64,
    /// Segundos desde 1970.
    pub at: u64,
    pub tool: String,
    pub ok: bool,
    pub summary: String,
    pub args: Value,
    pub data: Value,
    pub duration_ms: u64,
    pub confirmed: bool,
    pub undo: Option<Undo>,
    pub undone: bool,
}

/// El informe de una herramienta tal como se puede enseñar y guardar: sin contenido privado y de tamaño razonable.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub summary: String,
    pub args: Value,
    pub data: Value,
}

fn truncate(text: &str, max: usize) -> String {
    let short: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        format!("{}…", short.trim_end())
    } else {
        short
    }
}

/// Copia acotada de un valor: textos, listas, claves y profundidad limitados, para que un resultado enorme
/// (una búsqueda con cientos de archivos) no llene el chat ni el disco.
pub fn compact(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(text) => Value::String(truncate(text, MAX_STRING_CHARS)),
        Value::Array(items) if depth >= MAX_DEPTH => json!(format!("({} elementos)", items.len())),
        Value::Array(items) => {
            let mut shown: Vec<Value> = items.iter().take(MAX_ITEMS).map(|item| compact(item, depth + 1)).collect();
            if items.len() > MAX_ITEMS {
                shown.push(json!(format!("… y {} más", items.len() - MAX_ITEMS)));
            }
            Value::Array(shown)
        }
        Value::Object(_) if depth >= MAX_DEPTH => json!("(datos)"),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, item) in map.iter().take(MAX_KEYS) {
                out.insert(key.clone(), compact(item, depth + 1));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Reemplaza todos los textos de un valor por «(oculto)» (conserva números, booleanos y la forma).
fn hide_texts(value: &Value) -> Value {
    match value {
        Value::String(_) => json!(HIDDEN),
        Value::Array(items) => Value::Array(items.iter().map(hide_texts).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), hide_texts(v))).collect()),
        other => other.clone(),
    }
}

pub fn prepare(report: &ToolReport) -> Prepared {
    let hide_args = HIDE_ARGS.contains(&report.tool.as_str());
    let hide_data = HIDE_DATA.contains(&report.tool.as_str());
    let args = if hide_args { hide_texts(&report.args) } else { report.args.clone() };
    let data = if hide_data { Value::Null } else { report.data.clone() };
    // En lo que se oculta, también el resumen puede traer el contenido: se deja solo la primera línea.
    let summary = if hide_args || hide_data {
        truncate(report.summary.lines().next().unwrap_or(""), 120)
    } else {
        truncate(&report.summary, MAX_SUMMARY_CHARS)
    };
    Prepared { summary, args: compact(&args, 0), data: compact(&data, 0) }
}

// ---------------------------------------------------------------------------------------------
// Deshacer
// ---------------------------------------------------------------------------------------------

fn placed_args(placed: &Placed) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("location".into(), json!(placed.location.key()));
    if let Some(subfolder) = &placed.subfolder {
        args.insert("subfolder".into(), json!(subfolder));
    }
    args.insert("name".into(), json!(placed.name));
    args
}

fn path_in(value: &Value, key: &str) -> Option<PathBuf> {
    value[key].as_str().filter(|text| !text.is_empty()).map(PathBuf::from)
}

/// Cómo deshacer lo que hizo una herramienta, si se puede. Siempre es OTRA llamada a una herramienta normal, que
/// pasa por los mismos permisos y confirmaciones: deshacer nunca se salta la seguridad.
pub fn undo_for(locations: &Locations, report: &ToolReport) -> Option<Undo> {
    if !report.ok {
        return None;
    }
    let to_trash = |path: Option<PathBuf>, label: &str| -> Option<Undo> {
        let placed = locations.locate(&path?)?;
        Some(Undo { tool: "delete_file".into(), args: Value::Object(placed_args(&placed)), label: label.into() })
    };
    match report.tool.as_str() {
        "move_file" => {
            let (from, to) = (locations.locate(&path_in(&report.data, "from")?)?, locations.locate(&path_in(&report.data, "to")?)?);
            // Devolver lo movido: de donde está ahora a donde estaba, con su nombre de antes.
            let mut args = placed_args(&to);
            args.insert("to_location".into(), json!(from.location.key()));
            if let Some(subfolder) = &from.subfolder {
                args.insert("to_subfolder".into(), json!(subfolder));
            }
            args.insert("new_name".into(), json!(from.name));
            Some(Undo { tool: "move_file".into(), args: Value::Object(args), label: format!("Devolver «{}» a su sitio", from.name) })
        }
        "copy_file" => to_trash(path_in(&report.data, "to"), "Enviar la copia a la Papelera"),
        "zip_files" => to_trash(path_in(&report.data, "zip"), "Enviar el zip a la Papelera"),
        "unzip_file" => to_trash(path_in(&report.data, "folder"), "Enviar la carpeta extraída a la Papelera"),
        "create_file" | "create_folder" => to_trash(path_in(&report.data, "path"), "Enviar lo creado a la Papelera"),
        "organize_folder" if report.data["undo_available"].as_bool() == Some(true) => {
            Some(Undo { tool: "undo_organize".into(), args: json!({}), label: "Deshacer la organización".into() })
        }
        "set_reminder" => Some(Undo { tool: "cancel_reminder".into(), args: json!({ "id": report.data["id"].as_u64()? }), label: "Cancelar este recordatorio".into() }),
        "remember" => Some(Undo { tool: "forget".into(), args: json!({ "id": report.data["id"].as_u64()? }), label: "Olvidar este recuerdo".into() }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Almacén
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct State {
    next_id: u64,
    items: Vec<Entry>,
}

pub struct Activity {
    path: PathBuf,
    state: Mutex<State>,
}

fn entry_to_json(entry: &Entry) -> Value {
    json!({
        "id": entry.id, "at": entry.at, "tool": entry.tool, "ok": entry.ok, "summary": entry.summary, "args": entry.args, "data": entry.data,
        "duration_ms": entry.duration_ms, "confirmed": entry.confirmed, "undone": entry.undone,
        "undo": entry.undo.as_ref().map(|u| json!({ "tool": u.tool, "args": u.args, "label": u.label })),
    })
}

fn entry_from_json(value: &Value) -> Option<Entry> {
    let undo = value["undo"].as_object().and_then(|u| {
        Some(Undo { tool: u.get("tool")?.as_str()?.to_string(), args: u.get("args")?.clone(), label: u.get("label")?.as_str()?.to_string() })
    });
    Some(Entry {
        id: value["id"].as_u64()?,
        at: value["at"].as_u64().unwrap_or(0),
        tool: value["tool"].as_str()?.to_string(),
        ok: value["ok"].as_bool().unwrap_or(false),
        summary: truncate(value["summary"].as_str().unwrap_or(""), MAX_SUMMARY_CHARS),
        args: compact(&value["args"], 0),
        data: compact(&value["data"], 0),
        duration_ms: value["duration_ms"].as_u64().unwrap_or(0),
        confirmed: value["confirmed"].as_bool().unwrap_or(false),
        undo,
        undone: value["undone"].as_bool().unwrap_or(false),
    })
}

impl Activity {
    /// Abre (o crea) el registro en `dir`. Un archivo ilegible se trata como vacío.
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join("activity.json");
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| {
                let items: Vec<Entry> = value["items"].as_array()?.iter().filter_map(entry_from_json).take(MAX_ENTRIES).collect();
                let floor = items.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                Some(State { next_id: value["next_id"].as_u64().unwrap_or(1).max(floor), items })
            })
            .unwrap_or_default();
        Self { path, state: Mutex::new(state) }
    }

    fn save(&self, state: &State) -> Result<(), String> {
        let items: Vec<Value> = state.items.iter().map(entry_to_json).collect();
        let body = serde_json::to_string(&json!({ "next_id": state.next_id, "items": items })).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear la carpeta de datos: {e}"))?;
        }
        // Se escribe a un archivo temporal y se renombra, para no dejar un JSON a medias.
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, body).map_err(|e| format!("no se pudo guardar la actividad: {e}"))?;
        std::fs::rename(&temp, &self.path).map_err(|e| format!("no se pudo guardar la actividad: {e}"))
    }

    /// Anota lo que hizo una herramienta (ya sin contenido privado) y devuelve la entrada.
    pub fn record(&self, report: &ToolReport, undo: Option<Undo>, at: u64) -> Result<Entry, String> {
        let prepared = prepare(report);
        let mut state = self.state.lock().expect("actividad envenenada");
        state.next_id = state.next_id.max(1);
        let entry = Entry {
            id: state.next_id,
            at,
            tool: report.tool.clone(),
            ok: report.ok,
            summary: prepared.summary,
            args: prepared.args,
            data: prepared.data,
            duration_ms: report.duration_ms,
            confirmed: report.confirmed,
            undo,
            undone: false,
        };
        state.next_id += 1;
        state.items.push(entry.clone());
        if state.items.len() > MAX_ENTRIES {
            let excess = state.items.len() - MAX_ENTRIES;
            state.items.drain(..excess);
        }
        self.save(&state)?;
        Ok(entry)
    }

    /// Las más recientes primero.
    pub fn list(&self) -> Vec<Entry> {
        let mut items = self.state.lock().expect("actividad envenenada").items.clone();
        items.reverse();
        items
    }

    pub fn get(&self, id: u64) -> Option<Entry> {
        self.state.lock().expect("actividad envenenada").items.iter().find(|e| e.id == id).cloned()
    }

    /// Marca una entrada como deshecha (para no ofrecer deshacerla dos veces).
    pub fn mark_undone(&self, id: u64) -> bool {
        let mut state = self.state.lock().expect("actividad envenenada");
        let Some(entry) = state.items.iter_mut().find(|e| e.id == id) else { return false };
        entry.undone = true;
        let _ = self.save(&state);
        true
    }

    pub fn clear(&self) {
        let mut state = self.state.lock().expect("actividad envenenada");
        state.items.clear();
        let _ = self.save(&state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn report(tool: &str, args: Value, data: Value) -> ToolReport {
        ToolReport { tool: tool.into(), args, ok: true, summary: format!("hecho {tool}"), data, duration_ms: 12, confirmed: true }
    }

    fn store(t: &TempLocations) -> Activity {
        Activity::open(t.locations_root("data"))
    }

    #[test]
    fn guarda_ordena_y_sobrevive_a_cerrar_y_abrir() {
        let t = temp_locations();
        let a = store(&t);
        let first = a.record(&report("create_folder", json!({ "name": "A" }), json!({ "path": "x" })), None, 1_000).unwrap();
        let second = a.record(&report("move_file", json!({}), json!({})), None, 2_000).unwrap();
        assert_eq!((first.id, second.id), (1, 2));
        assert_eq!(a.list().iter().map(|e| e.id).collect::<Vec<_>>(), [2, 1], "las más recientes primero");
        let reopened = store(&t);
        assert_eq!(reopened.list().len(), 2);
        assert_eq!(reopened.record(&report("t", json!({}), json!({})), None, 3_000).unwrap().id, 3, "los identificadores no se repiten");
        let back = reopened.get(1).unwrap();
        assert_eq!((back.tool.as_str(), back.ok, back.duration_ms, back.confirmed), ("create_folder", true, 12, true));
    }

    #[test]
    fn se_queda_con_las_ultimas_y_se_puede_borrar_todo() {
        let t = temp_locations();
        let a = store(&t);
        for i in 0..MAX_ENTRIES + 5 {
            a.record(&report("t", json!({ "i": i }), json!({})), None, i as u64).unwrap();
        }
        let list = a.list();
        assert_eq!(list.len(), MAX_ENTRIES);
        assert_eq!(list[0].args["i"], MAX_ENTRIES + 4, "se conserva lo más reciente");
        a.clear();
        assert!(a.list().is_empty() && store(&t).list().is_empty());
    }

    #[test]
    fn oculta_lo_que_el_usuario_escribio_y_el_contenido_ajeno() {
        let typed = prepare(&report("type_text", json!({ "window": "Bloc de notas", "text": "mi contraseña es 1234" }), json!({ "characters": 21 })));
        assert_eq!(typed.args, json!({ "window": HIDDEN, "text": HIDDEN }));
        assert!(!typed.summary.contains("1234"));
        let clip = prepare(&ToolReport { summary: "Portapapeles:\nsecreto".into(), ..report("read_clipboard", json!({}), json!({ "text": "secreto" })) });
        assert_eq!((clip.data, clip.summary.as_str()), (Value::Null, "Portapapeles:"));
        let file = prepare(&report("read_file", json!({ "name": "notas.txt" }), json!({ "text": "contenido privado" })));
        assert!(file.data.is_null() && file.args == json!({ "name": "notas.txt" }), "el nombre del archivo sí se guarda");
        // Lo demás se guarda tal cual.
        assert_eq!(prepare(&report("move_file", json!({ "name": "a.txt" }), json!({ "to": "C:\\x" }))).args, json!({ "name": "a.txt" }));
    }

    #[test]
    fn acota_textos_listas_y_profundidad() {
        let big = json!({ "lista": (0..100).collect::<Vec<_>>(), "texto": "x".repeat(1000), "hondo": { "a": { "b": { "c": { "d": { "e": 1 } } } } } });
        let small = compact(&big, 0);
        assert_eq!(small["lista"].as_array().unwrap().len(), MAX_ITEMS + 1);
        assert_eq!(small["lista"].as_array().unwrap().last().unwrap(), &json!("… y 60 más"));
        assert!(small["texto"].as_str().unwrap().chars().count() <= MAX_STRING_CHARS + 1);
        assert_eq!(small["hondo"]["a"]["b"]["c"], json!("(datos)"));
        let long_summary = prepare(&ToolReport { summary: "y".repeat(5000), ..report("t", json!({}), json!({})) });
        assert!(long_summary.summary.chars().count() <= MAX_SUMMARY_CHARS + 1);
    }

    #[test]
    fn archivos_estropeados_no_rompen_el_registro() {
        let t = temp_locations();
        let dir = t.locations_root("data");
        std::fs::write(dir.join("activity.json"), "no es json").unwrap();
        assert!(store(&t).list().is_empty());
        std::fs::write(dir.join("activity.json"), r#"{"next_id":1,"items":[7,{"id":5,"tool":"t","ok":true},{"sin":"id"}]}"#).unwrap();
        let a = store(&t);
        assert_eq!(a.list().iter().map(|e| e.id).collect::<Vec<_>>(), [5]);
        assert_eq!(a.record(&report("t", json!({}), json!({})), None, 1).unwrap().id, 6, "el siguiente id va por encima del mayor");
    }

    #[test]
    fn marca_lo_deshecho() {
        let t = temp_locations();
        let a = store(&t);
        let undo = Some(Undo { tool: "cancel_reminder".into(), args: json!({ "id": 3 }), label: "Cancelar".into() });
        let id = a.record(&report("set_reminder", json!({}), json!({ "id": 3 })), undo.clone(), 1).unwrap().id;
        assert!(!a.get(id).unwrap().undone && a.get(id).unwrap().undo == undo);
        assert!(a.mark_undone(id) && !a.mark_undone(999));
        assert!(store(&t).get(id).unwrap().undone, "queda guardado");
    }

    #[test]
    fn calcula_como_deshacer_cada_accion() {
        let t = temp_locations();
        let l = &t.locations;
        let root = |key: &str| t.locations_root(key).canonicalize().unwrap();
        let shown = |path: PathBuf| path.display().to_string().trim_start_matches(r"\\?\").to_string();

        // Mover: se devuelve a su sitio, con su nombre de antes.
        let moved = report("move_file", json!({}), json!({ "from": shown(root("desktop").join("a.txt")), "to": shown(root("documents").join("Proyecto").join("b.txt")) }));
        let undo = undo_for(l, &moved).unwrap();
        assert_eq!(undo.tool, "move_file");
        assert_eq!(
            undo.args,
            json!({ "location": "documents", "subfolder": "Proyecto", "name": "b.txt", "to_location": "desktop", "new_name": "a.txt" })
        );
        // Copiar, comprimir, descomprimir y crear: lo creado va a la Papelera (con confirmación, como todo borrado).
        let copy = report("copy_file", json!({}), json!({ "from": "x", "to": shown(root("downloads").join("informe - copia.docx")) }));
        assert_eq!(undo_for(l, &copy).unwrap().args, json!({ "location": "downloads", "name": "informe - copia.docx" }));
        assert_eq!(undo_for(l, &copy).unwrap().tool, "delete_file");
        let zip = report("zip_files", json!({}), json!({ "zip": shown(root("desktop").join("Fotos.zip")) }));
        assert_eq!(undo_for(l, &zip).unwrap().args, json!({ "location": "desktop", "name": "Fotos.zip" }));
        let unzip = report("unzip_file", json!({}), json!({ "folder": shown(root("desktop").join("Fotos")) }));
        assert_eq!(undo_for(l, &unzip).unwrap().args["name"], "Fotos");
        let folder = report("create_folder", json!({}), json!({ "path": shown(root("desktop").join("Nueva")), "location": "desktop" }));
        assert_eq!(undo_for(l, &folder).unwrap().tool, "delete_file");
        // Otras.
        assert_eq!(undo_for(l, &report("organize_folder", json!({}), json!({ "undo_available": true }))).unwrap().tool, "undo_organize");
        assert!(undo_for(l, &report("organize_folder", json!({}), json!({ "undo_available": false }))).is_none());
        assert_eq!(undo_for(l, &report("set_reminder", json!({}), json!({ "id": 7 }))).unwrap().args, json!({ "id": 7 }));
        assert_eq!(undo_for(l, &report("remember", json!({}), json!({ "id": 4 }))).unwrap().tool, "forget");
    }

    #[test]
    fn no_ofrece_deshacer_lo_que_no_se_puede_ni_lo_que_fallo_ni_lo_que_cae_fuera() {
        let t = temp_locations();
        let l = &t.locations;
        for tool in ["delete_file", "empty_recycle_bin", "play_music", "web_search", "type_text", "set_brightness"] {
            assert!(undo_for(l, &report(tool, json!({}), json!({}))).is_none(), "{tool}");
        }
        let failed = ToolReport { ok: false, ..report("copy_file", json!({}), json!({ "to": "x" })) };
        assert!(undo_for(l, &failed).is_none());
        // Una ruta fuera de las ubicaciones permitidas nunca produce una acción.
        let outside = report("zip_files", json!({}), json!({ "zip": "C:\\Windows\\System32\\config.zip" }));
        assert!(undo_for(l, &outside).is_none());
        assert!(undo_for(l, &report("zip_files", json!({}), json!({}))).is_none(), "sin ruta no hay nada que deshacer");
        let sneaky = report("copy_file", json!({}), json!({ "to": format!("{}\\..\\..\\fuera.txt", t.locations_root("desktop").display()) }));
        assert!(undo_for(l, &sneaky).is_none());
    }
}
