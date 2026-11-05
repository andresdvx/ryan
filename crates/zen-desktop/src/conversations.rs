//! Historial de conversaciones: cada chat se guarda en su propio archivo JSON dentro de la carpeta de datos
//! (`conversations/<id>.json`), solo en este equipo. Se pueden listar, buscar, reanudar y borrar. Lo que se
//! guarda es lo que se ve en el chat (lo que escribe el usuario y lo que responde el asistente), no los
//! detalles internos de la conversación con la IA ni imágenes.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub const MAX_CONVERSATIONS: usize = 200;
pub const MAX_MESSAGES: usize = 500;
const MAX_TEXT_CHARS: usize = 20_000;
const TITLE_CHARS: usize = 60;
const PREVIEW_CHARS: usize = 100;
const SNIPPET_RADIUS: usize = 45;
/// Cuántos mensajes se le devuelven a la IA al reanudar una conversación.
const RESTORE_MESSAGES: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub role: Role,
    pub text: String,
    /// Segundos desde 1970.
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub id: u64,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub messages: Vec<StoredMessage>,
}

/// Lo que se muestra en la lista (y en los resultados de una búsqueda).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: u64,
    pub title: String,
    pub updated: u64,
    pub count: usize,
    /// Último mensaje del asistente, recortado.
    pub preview: String,
    /// En una búsqueda, el trozo donde aparece lo buscado.
    pub snippet: Option<String>,
}

pub struct Conversations {
    dir: PathBuf,
    /// La conversación a la que se añaden los mensajes nuevos (`None`: la siguiente empieza una nueva).
    current: Mutex<Option<u64>>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Texto limpio para guardar: sin espacios sobrantes y con un tope de tamaño.
fn clean(text: &str) -> String {
    text.trim().chars().take(MAX_TEXT_CHARS).collect()
}

fn make_title(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let short: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        format!("{}…", short.trim_end())
    } else {
        short
    }
}

/// Minúsculas y sin tildes, un carácter por carácter (así los índices coinciden con el texto original).
fn fold(text: &str) -> Vec<char> {
    text.chars()
        .map(|c| {
            let lower = c.to_lowercase().next().unwrap_or(c);
            match lower {
                'á' | 'à' | 'ä' | 'â' => 'a',
                'é' | 'è' | 'ë' | 'ê' => 'e',
                'í' | 'ì' | 'ï' | 'î' => 'i',
                'ó' | 'ò' | 'ö' | 'ô' => 'o',
                'ú' | 'ù' | 'ü' | 'û' => 'u',
                'ñ' => 'n',
                other => other,
            }
        })
        .collect()
}

/// Posición (en caracteres) de la primera aparición de `needle` en `haystack`.
fn find_chars(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn snippet(text: &str, at: usize, needle_len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(SNIPPET_RADIUS);
    let end = (at + needle_len + SNIPPET_RADIUS).min(chars.len());
    let body: String = chars[start..end].iter().map(|c| if c.is_control() { ' ' } else { *c }).collect();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, strip_markdown(body.trim()), if end < chars.len() { "…" } else { "" })
}

/// ¿Es un aviso del sistema (resultado de una herramienta, un error) y no algo que dijo la IA?
/// Los avisos se muestran en el historial, pero no se le devuelven a la IA al reanudar.
pub fn is_notice(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('✔')
        || text.starts_with('✖')
        || text.starts_with("No pude consultar a la IA")
        || text.starts_with("Sigo con la tarea anterior")
        || text.starts_with("Me detuve")
        || text.starts_with("Se agotó el tiempo")
}

/// Los últimos mensajes de la conversación que se le devuelven a la IA al reanudarla: solo lo que dijeron
/// el usuario y la IA (sin avisos), empezando siempre por un mensaje del usuario.
pub fn restorable(conversation: &Conversation) -> Vec<(Role, String)> {
    let mut kept: Vec<(Role, String)> = conversation
        .messages
        .iter()
        .filter(|m| m.role == Role::User || !is_notice(&m.text))
        .map(|m| (m.role, m.text.clone()))
        .collect();
    if kept.len() > RESTORE_MESSAGES {
        kept.drain(..kept.len() - RESTORE_MESSAGES);
    }
    let first_user = kept.iter().position(|(role, _)| *role == Role::User).unwrap_or(kept.len());
    kept.drain(..first_user);
    kept
}

impl Conversations {
    pub fn open(data_dir: PathBuf) -> Self {
        Self { dir: data_dir.join("conversations"), current: Mutex::new(None) }
    }

    fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn read(&self, id: u64) -> Option<Conversation> {
        let value: Value = serde_json::from_str(&fs::read_to_string(self.path(id)).ok()?).ok()?;
        let messages: Vec<StoredMessage> = value["messages"]
            .as_array()?
            .iter()
            .filter_map(|m| Some(StoredMessage { role: Role::parse(m["role"].as_str()?)?, text: clean(m["text"].as_str()?), at: m["at"].as_u64().unwrap_or(0) }))
            .take(MAX_MESSAGES)
            .collect();
        Some(Conversation {
            id,
            title: make_title(value["title"].as_str().unwrap_or("")),
            created: value["created"].as_u64().unwrap_or(0),
            updated: value["updated"].as_u64().unwrap_or(0),
            messages,
        })
    }

    fn write(&self, conversation: &Conversation) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("no se pudo crear la carpeta del historial: {e}"))?;
        let messages: Vec<Value> = conversation.messages.iter().map(|m| json!({ "role": m.role.as_str(), "text": m.text, "at": m.at })).collect();
        let body = serde_json::to_string(&json!({
            "title": conversation.title, "created": conversation.created, "updated": conversation.updated, "messages": messages,
        }))
        .map_err(|e| e.to_string())?;
        // Se escribe a un archivo temporal y se renombra, para no dejar un JSON a medias.
        let path = self.path(conversation.id);
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, body).map_err(|e| format!("no se pudo guardar la conversación: {e}"))?;
        fs::rename(&temp, &path).map_err(|e| format!("no se pudo guardar la conversación: {e}"))
    }

    fn ids(&self) -> Vec<u64> {
        fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.path().file_stem()?.to_str()?.parse::<u64>().ok().filter(|_| entry.path().extension().is_some_and(|e| e == "json")))
            .collect()
    }

    /// Añade un mensaje a la conversación en curso (creándola si hace falta) y devuelve su identificador.
    pub fn record(&self, role: Role, text: &str) -> Result<u64, String> {
        self.record_at(role, text, now_ms())
    }

    pub fn record_at(&self, role: Role, text: &str, now_ms: u64) -> Result<u64, String> {
        let text = clean(text);
        if text.is_empty() {
            return Err("el mensaje está vacío".into());
        }
        let now = now_ms / 1000;
        let mut current = self.current.lock().expect("historial envenenado");
        let mut conversation = match current.and_then(|id| self.read(id)) {
            Some(existing) => existing,
            None => {
                // Los identificadores son milisegundos; si ya existe uno igual, se pasa al siguiente.
                let mut id = now_ms;
                while self.path(id).exists() {
                    id += 1;
                }
                Conversation { id, title: String::new(), created: now, updated: now, messages: Vec::new() }
            }
        };
        if conversation.title.is_empty() && role == Role::User {
            conversation.title = make_title(&text);
        }
        if conversation.messages.len() < MAX_MESSAGES {
            conversation.messages.push(StoredMessage { role, text, at: now });
            conversation.updated = now;
            if conversation.title.is_empty() {
                conversation.title = "Conversación".into();
            }
            self.write(&conversation)?;
        }
        let is_new = current.is_none_or(|id| id != conversation.id);
        *current = Some(conversation.id);
        drop(current);
        if is_new {
            self.prune();
        }
        Ok(conversation.id)
    }

    /// Deja solo las `MAX_CONVERSATIONS` más recientes.
    fn prune(&self) {
        let mut all = self.summaries();
        if all.len() <= MAX_CONVERSATIONS {
            return;
        }
        all.sort_by_key(|s| std::cmp::Reverse(s.updated));
        for old in all.into_iter().skip(MAX_CONVERSATIONS) {
            let _ = fs::remove_file(self.path(old.id));
        }
    }

    /// La siguiente conversación que se escriba empezará una nueva (por ejemplo, tras «/new»).
    pub fn start_new(&self) {
        *self.current.lock().expect("historial envenenado") = None;
    }

    pub fn current_id(&self) -> Option<u64> {
        *self.current.lock().expect("historial envenenado")
    }

    fn summary_of(conversation: &Conversation, snippet: Option<String>) -> Summary {
        let preview = conversation
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant && !is_notice(&m.text))
            .or_else(|| conversation.messages.last())
            .map(|m| make_preview(&m.text))
            .unwrap_or_default();
        Summary { id: conversation.id, title: conversation.title.clone(), updated: conversation.updated, count: conversation.messages.len(), preview, snippet }
    }

    fn summaries(&self) -> Vec<Summary> {
        self.ids().into_iter().filter_map(|id| self.read(id)).map(|c| Self::summary_of(&c, None)).collect()
    }

    /// Más recientes primero.
    pub fn list(&self) -> Vec<Summary> {
        let mut all = self.summaries();
        all.sort_by_key(|s| std::cmp::Reverse((s.updated, s.id)));
        all
    }

    /// Conversaciones donde aparece el texto (sin distinguir mayúsculas ni tildes), con el trozo donde sale.
    pub fn search(&self, query: &str) -> Vec<Summary> {
        let needle = fold(query.trim());
        if needle.is_empty() {
            return self.list();
        }
        let mut found: Vec<Summary> = self
            .ids()
            .into_iter()
            .filter_map(|id| self.read(id))
            .filter_map(|conversation| {
                let in_title = find_chars(&fold(&conversation.title), &needle).is_some();
                let hit = conversation.messages.iter().find_map(|m| find_chars(&fold(&m.text), &needle).map(|at| snippet(&m.text, at, needle.len())));
                (in_title || hit.is_some()).then(|| Self::summary_of(&conversation, hit))
            })
            .collect();
        found.sort_by_key(|s| std::cmp::Reverse((s.updated, s.id)));
        found
    }

    pub fn load(&self, id: u64) -> Option<Conversation> {
        self.read(id)
    }

    /// Como `load`, y además los mensajes nuevos se seguirán añadiendo a esa conversación.
    pub fn resume(&self, id: u64) -> Option<Conversation> {
        let conversation = self.read(id)?;
        *self.current.lock().expect("historial envenenado") = Some(id);
        Some(conversation)
    }

    pub fn delete(&self, id: u64) -> bool {
        let removed = fs::remove_file(self.path(id)).is_ok();
        let mut current = self.current.lock().expect("historial envenenado");
        if *current == Some(id) {
            *current = None;
        }
        removed
    }

    /// Borra todo el historial.
    pub fn clear(&self) {
        for id in self.ids() {
            let _ = fs::remove_file(self.path(id));
        }
        self.start_new();
    }
}

/// Quita las marcas de formato del chat (negritas, código, títulos) para mostrar el texto en una lista.
fn strip_markdown(text: &str) -> String {
    let without_marks = text.replace("**", "").replace("__", "").replace('`', "");
    without_marks
        .lines()
        .map(|line| line.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}

fn make_preview(text: &str) -> String {
    let flat: String = strip_markdown(text).split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = flat.chars().take(PREVIEW_CHARS).collect();
    if flat.chars().count() > PREVIEW_CHARS {
        format!("{}…", short.trim_end())
    } else {
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{temp_locations, TempLocations};

    fn store(t: &TempLocations) -> Conversations {
        Conversations::open(t.locations_root("data"))
    }

    #[test]
    fn guarda_una_conversacion_y_la_sigue_ampliando() {
        let t = temp_locations();
        let s = store(&t);
        let id = s.record_at(Role::User, "  Pon música de Arctic Monkeys\npor favor ", 1_000_000).unwrap();
        assert_eq!(s.record_at(Role::Assistant, "¡Listo!", 1_005_000).unwrap(), id, "los mensajes siguientes van a la misma conversación");
        let c = s.load(id).unwrap();
        assert_eq!(c.title, "Pon música de Arctic Monkeys", "el título es la primera línea del primer mensaje");
        assert_eq!((c.created, c.updated, c.messages.len()), (1_000, 1_005, 2));
        assert_eq!((c.messages[0].role, c.messages[1].text.as_str()), (Role::User, "¡Listo!"));
    }

    #[test]
    fn sobrevive_a_cerrar_y_abrir_y_start_new_empieza_otra() {
        let t = temp_locations();
        let first = store(&t).record_at(Role::User, "hola", 1_000).unwrap();
        let s = store(&t); // «reinicio»: la conversación en curso no se recuerda, pero el archivo sí
        assert_eq!(s.current_id(), None);
        assert_eq!(s.load(first).unwrap().messages.len(), 1);
        let second = s.record_at(Role::User, "otra cosa", 2_000).unwrap();
        s.start_new();
        let third = s.record_at(Role::User, "tercera", 3_000).unwrap();
        assert_eq!(s.list().iter().map(|c| c.id).collect::<Vec<_>>(), [third, second, first], "más recientes primero");
    }

    #[test]
    fn dos_conversaciones_en_el_mismo_milisegundo_no_se_pisan() {
        let t = temp_locations();
        let s = store(&t);
        let a = s.record_at(Role::User, "uno", 5_000).unwrap();
        s.start_new();
        let b = s.record_at(Role::User, "dos", 5_000).unwrap();
        assert_ne!(a, b);
        assert_eq!((s.load(a).unwrap().messages[0].text.as_str(), s.load(b).unwrap().messages[0].text.as_str()), ("uno", "dos"));
    }

    #[test]
    fn rechaza_mensajes_vacios_y_recorta_los_enormes() {
        let t = temp_locations();
        let s = store(&t);
        assert!(s.record_at(Role::User, "   \n ", 1_000).is_err());
        assert!(s.list().is_empty(), "un mensaje vacío no crea nada");
        let id = s.record_at(Role::User, &"a".repeat(MAX_TEXT_CHARS + 500), 1_000).unwrap();
        assert_eq!(s.load(id).unwrap().messages[0].text.chars().count(), MAX_TEXT_CHARS);
        let long_title = s.record_at(Role::User, &"palabra ".repeat(40), 2_000).unwrap();
        assert!(s.load(long_title).is_some_and(|c| c.title.chars().count() <= TITLE_CHARS + 1 && c.title.ends_with('…')));
    }

    #[test]
    fn una_conversacion_no_pasa_del_maximo_de_mensajes() {
        let t = temp_locations();
        let s = store(&t);
        let mut id = 0;
        for i in 0..MAX_MESSAGES + 20 {
            id = s.record_at(Role::User, &format!("mensaje {i}"), 1_000 + i as u64).unwrap();
        }
        assert_eq!(s.load(id).unwrap().messages.len(), MAX_MESSAGES);
    }

    #[test]
    fn solo_se_conservan_las_conversaciones_mas_recientes() {
        let t = temp_locations();
        let s = store(&t);
        let mut ids = Vec::new();
        for i in 0..MAX_CONVERSATIONS + 3 {
            s.start_new();
            ids.push(s.record_at(Role::User, &format!("c{i}"), 1_000_000 + (i as u64) * 1_000).unwrap());
        }
        let kept: Vec<u64> = s.list().iter().map(|c| c.id).collect();
        assert_eq!(kept.len(), MAX_CONVERSATIONS);
        assert!(!kept.contains(&ids[0]) && kept.contains(ids.last().unwrap()), "se borran las más antiguas");
    }

    #[test]
    fn busca_sin_distinguir_mayusculas_ni_tildes_y_enseña_donde_sale() {
        let t = temp_locations();
        let s = store(&t);
        let a = s.record_at(Role::User, "¿Qué ocupa más espacio en Descargas?", 1_000).unwrap();
        s.record_at(Role::Assistant, "Los vídeos de la carpeta Películas ocupan 12 GB.", 2_000).unwrap();
        s.start_new();
        let b = s.record_at(Role::User, "Recuérdame llamar al dentista", 3_000).unwrap();
        let hits = s.search("PELICULAS");
        assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), [a]);
        assert!(hits[0].snippet.as_deref().is_some_and(|s| s.contains("Películas")), "{:?}", hits[0].snippet);
        assert_eq!(s.search("qué").iter().map(|h| h.id).collect::<Vec<_>>(), [a], "con tilde o sin ella");
        assert_eq!(s.search("dentista").iter().map(|h| h.id).collect::<Vec<_>>(), [b]);
        assert!(s.search("zzz-no-existe").is_empty());
        assert_eq!(s.search("   ").len(), 2, "una búsqueda vacía lista todo");
    }

    #[test]
    fn el_resumen_muestra_la_ultima_respuesta_de_la_ia_y_no_los_avisos() {
        let t = temp_locations();
        let s = store(&t);
        s.record_at(Role::User, "borra Pruebas", 1_000).unwrap();
        s.record_at(Role::Assistant, "Listo, la envié a la Papelera.", 2_000).unwrap();
        let id = s.record_at(Role::Assistant, "✔ Envié a la Papelera: Pruebas", 3_000).unwrap();
        let summary = s.list().into_iter().find(|c| c.id == id).unwrap();
        assert_eq!((summary.count, summary.preview.as_str()), (3, "Listo, la envié a la Papelera."));
    }

    #[test]
    fn las_vistas_previas_no_llevan_marcas_de_formato() {
        assert_eq!(make_preview("# Título\nClaro: **Arabella** y `código`"), "Título Claro: Arabella y código");
        let t = temp_locations();
        let s = store(&t);
        s.record_at(Role::User, "dime canciones", 1_000).unwrap();
        s.record_at(Role::Assistant, "Te recomiendo **Arabella** de Arctic Monkeys", 2_000).unwrap();
        assert_eq!(s.list()[0].preview, "Te recomiendo Arabella de Arctic Monkeys");
        assert_eq!(s.search("arabella")[0].snippet.as_deref(), Some("Te recomiendo Arabella de Arctic Monkeys"));
    }

    #[test]
    fn reanudar_continua_en_la_misma_conversacion() {
        let t = temp_locations();
        let s = store(&t);
        let id = s.record_at(Role::User, "hola", 1_000).unwrap();
        s.start_new();
        assert!(s.resume(id).is_some() && s.current_id() == Some(id));
        s.record_at(Role::Assistant, "de nuevo por aquí", 2_000).unwrap();
        assert_eq!(s.load(id).unwrap().messages.len(), 2);
        assert!(s.resume(999).is_none());
    }

    #[test]
    fn borra_una_o_todas() {
        let t = temp_locations();
        let s = store(&t);
        let a = s.record_at(Role::User, "a", 1_000).unwrap();
        s.start_new();
        let b = s.record_at(Role::User, "b", 2_000).unwrap();
        assert!(s.delete(b) && s.current_id().is_none() && !s.delete(b), "borrar la actual deja de grabar en ella");
        assert_eq!(s.list().iter().map(|c| c.id).collect::<Vec<_>>(), [a]);
        s.clear();
        assert!(s.list().is_empty());
    }

    #[test]
    fn archivos_estropeados_o_con_datos_raros_no_rompen_nada() {
        let t = temp_locations();
        let s = store(&t);
        let id = s.record_at(Role::User, "bueno", 1_000).unwrap();
        let dir = t.locations_root("data").join("conversations");
        fs::write(dir.join("123.json"), "esto no es json").unwrap();
        fs::write(dir.join("456.json"), r#"{"title":5,"messages":[{"role":"root","text":"x"},{"role":"user","text":"vale"},7]}"#).unwrap();
        fs::write(dir.join("nota.txt"), "ignorada").unwrap();
        fs::write(dir.join("abc.json"), "{}").unwrap();
        let ids: Vec<u64> = s.list().iter().map(|c| c.id).collect();
        assert!(ids.contains(&id) && ids.contains(&456) && !ids.contains(&123), "{ids:?}");
        assert_eq!(s.load(456).unwrap().messages.len(), 1, "los mensajes con rol desconocido se descartan");
    }

    #[test]
    fn al_reanudar_a_la_ia_solo_se_le_devuelve_lo_que_dijeron_el_usuario_y_ella() {
        let said = |role, text: &str| StoredMessage { role, text: text.into(), at: 0 };
        let conversation = Conversation {
            id: 1,
            title: "t".into(),
            created: 0,
            updated: 0,
            messages: vec![
                said(Role::Assistant, "Un aviso suelto antes de empezar"),
                said(Role::User, "borra Pruebas"),
                said(Role::Assistant, "Vale, voy a ello."),
                said(Role::Assistant, "✔ Envié a la Papelera: Pruebas"),
                said(Role::Assistant, "No pude consultar a la IA: sin red"),
                said(Role::User, "gracias"),
            ],
        };
        let kept = restorable(&conversation);
        assert_eq!(
            kept,
            vec![(Role::User, "borra Pruebas".to_string()), (Role::Assistant, "Vale, voy a ello.".to_string()), (Role::User, "gracias".to_string())]
        );
        // Con una conversación muy larga se queda con el final y siempre empieza por el usuario.
        let long = Conversation {
            messages: (0..100).map(|i| said(if i % 2 == 0 { Role::User } else { Role::Assistant }, &format!("m{i}"))).collect(),
            ..conversation
        };
        let kept = restorable(&long);
        assert!(kept.len() <= RESTORE_MESSAGES && kept[0].0 == Role::User, "{kept:?}");
        assert_eq!(kept.last().unwrap().1, "m99");
    }

    #[test]
    fn distingue_avisos_de_respuestas() {
        for notice in ["✔ Listo", "✖ Falló", "  ✔ con espacios", "No pude consultar a la IA: x", "Se agotó el tiempo para confirmar"] {
            assert!(is_notice(notice), "{notice}");
        }
        for reply in ["Hola", "Hecho ✔", "Claro, te lo explico"] {
            assert!(!is_notice(reply), "{reply}");
        }
    }
}
