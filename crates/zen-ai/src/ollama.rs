//! Proveedor Ollama: modelos locales (Qwen, Llama, DeepSeek...) sin API key.
//! https://github.com/ollama/ollama/blob/main/docs/api.md

use serde_json::{json, Value};
use zen_core::ai::{LlmError, LlmProvider, LlmResponse, Message, ToolInvocation};
use zen_core::tools::ToolSpec;

use crate::http::{output_text, HttpClient};

pub const DEFAULT_MODEL: &str = "qwen3:8b";
pub const DEFAULT_URL: &str = "http://localhost:11434";
/// Ollama usa 4096 de contexto por defecto, que se queda corto con las instrucciones y las herramientas.
const CONTEXT_TOKENS: u32 = 8192;
/// La primera respuesta incluye cargar el modelo en memoria, que en equipos modestos tarda.
const TIMEOUT_SECS: u64 = 300;

/// Opciones del servidor local que no son el modelo: dónde está y si el modelo razona antes de responder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaSettings {
    pub url: String,
    /// Los modelos con razonamiento (Qwen3, DeepSeek-R1…) piensan en silencio antes de escribir, y eso puede
    /// tardar decenas de segundos. Apagado, responden casi al instante.
    pub think: bool,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self { url: DEFAULT_URL.to_string(), think: false }
    }
}

pub struct OllamaProvider {
    base_url: String,
    model: String,
    think: bool,
    http: HttpClient,
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: String) -> Self {
        let base_url = normalize_base_url(base_url).unwrap_or_else(|_| DEFAULT_URL.to_string());
        Self { base_url, model, think: false, http: HttpClient::with_timeout(TIMEOUT_SECS) }
    }

    pub fn with_think(mut self, think: bool) -> Self {
        self.think = think;
        self
    }

    /// Lanza la petición indicando si se quiere razonamiento. Los modelos sin esa capacidad rechazan el parámetro:
    /// en ese caso se repite una vez sin él.
    fn attempt<T>(&self, mut run: impl FnMut(Option<bool>) -> Result<T, LlmError>) -> Result<T, LlmError> {
        match run(Some(self.think)) {
            Err(error) if rejects_think(&error) => run(None),
            other => other,
        }
    }
}

fn rejects_think(error: &LlmError) -> bool {
    matches!(error, LlmError::Unsupported(text) if text.contains("support thinking"))
}

impl LlmProvider for OllamaProvider {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError> {
        self.attempt(|think| {
            let body = build_request_with(&self.model, system, history, tools, false, think);
            let (status, value) = self
                .http
                .post_json(&format!("{}/api/chat", self.base_url), &[], &body)
                .map_err(|error| unreachable_hint(error, &self.base_url))?;
            parse_response(status, &value, &self.model)
        })
    }

    /// Ollama envía la respuesta en trozos (una línea JSON por trozo): el texto se reenvía según llega,
    /// sin el razonamiento, y las llamadas a herramientas se juntan para el final.
    fn generate_stream(
        &self,
        system: &str,
        history: &[Message],
        tools: &[ToolSpec],
        on_text: &mut dyn FnMut(&str),
    ) -> Result<LlmResponse, LlmError> {
        self.attempt(|think| {
        let body = build_request_with(&self.model, system, history, tools, true, think);
        let (mut raw, mut filter, mut tool_calls) = (String::new(), ThinkFilter::default(), Vec::new());
        let (status, error_body) = self
            .http
            .post_lines(&format!("{}/api/chat", self.base_url), &[], &body, &mut |line| {
                let chunk: Value = serde_json::from_str(line).map_err(|_| LlmError::BadResponse("Ollama envió una línea que no es JSON".into()))?;
                // Un error a mitad de respuesta llega como una línea más: `{ "error": "..." }`.
                if chunk.get("error").is_some() {
                    return Err(map_api_error(500, &chunk, &self.model));
                }
                let message = &chunk["message"];
                if let Some(text) = message["content"].as_str().filter(|text| !text.is_empty()) {
                    raw.push_str(text);
                    let visible = filter.feed(text);
                    if !visible.is_empty() {
                        on_text(&visible);
                    }
                }
                tool_calls.extend(parse_tool_calls(&message["tool_calls"])?);
                Ok(())
            })
            .map_err(|error| unreachable_hint(error, &self.base_url))?;
        if status >= 400 {
            return Err(map_api_error(status, &error_body.unwrap_or(Value::Null), &self.model));
        }
        let rest = filter.finish();
        if !rest.is_empty() {
            on_text(&rest);
        }
        Ok(LlmResponse { text: strip_thinking(&raw), tool_calls, raw: None })
        })
    }

    fn supports_images(&self) -> bool {
        true
    }
}

/// Valida la dirección del servidor y le quita la barra final. Solo http(s), sin usuario ni parámetros.
pub fn normalize_base_url(text: &str) -> Result<String, String> {
    let url = text.trim().trim_end_matches('/');
    let rest = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"));
    let valid = rest.is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
        && url.len() <= 200
        && !url.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '@' | '\\'));
    if valid {
        Ok(url.to_string())
    } else {
        Err("La dirección de Ollama debe empezar por http:// o https://, por ejemplo http://localhost:11434.".into())
    }
}

/// Modelos descargados en ese servidor de Ollama (`ollama list`).
pub fn list_models(base_url: &str) -> Result<Vec<String>, LlmError> {
    let base_url = normalize_base_url(base_url).map_err(LlmError::Unsupported)?;
    let (status, body) = HttpClient::with_timeout(10)
        .get_json(&format!("{base_url}/api/tags"))
        .map_err(|error| unreachable_hint(error, &base_url))?;
    if status >= 400 {
        return Err(map_api_error(status, &body, ""));
    }
    let models = body["models"].as_array().ok_or_else(|| LlmError::BadResponse("la respuesta no trae «models»".into()))?;
    Ok(models.iter().filter_map(|model| model["name"].as_str()).map(str::to_string).collect())
}

/// Si Ollama no está abierto, el error de red dice qué hacer en vez de mostrar solo «connection refused».
fn unreachable_hint(error: LlmError, base_url: &str) -> LlmError {
    match error {
        LlmError::Network(_) => LlmError::Network(format!(
            "Ollama no responde en {base_url}. Comprueba que esté abierto (ollama.com/download) y que la dirección de Ajustes sea correcta"
        )),
        other => other,
    }
}

#[cfg(test)]
fn build_request(model: &str, system: &str, history: &[Message], tools: &[ToolSpec]) -> Value {
    build_request_with(model, system, history, tools, false, None)
}

/// `think`: `Some(true/false)` pide (o no) razonamiento de forma explícita; `None` deja el comportamiento del modelo.
fn build_request_with(model: &str, system: &str, history: &[Message], tools: &[ToolSpec], stream: bool, think: Option<bool>) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": system })];
    for message in history {
        match message {
            Message::User(text) => messages.push(json!({ "role": "user", "content": text })),
            Message::Assistant { text, tool_calls, .. } => {
                let mut entry = json!({ "role": "assistant", "content": text });
                if !tool_calls.is_empty() {
                    entry["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| {
                            // Ollama exige los argumentos como objeto; un texto suelto (JSON inválido) se descarta.
                            let arguments = if call.args.is_object() { call.args.clone() } else { json!({}) };
                            json!({ "function": { "name": call.name, "arguments": arguments } })
                        })
                        .collect();
                }
                messages.push(entry);
            }
            Message::ToolResult { name, output, .. } => {
                messages.push(json!({ "role": "tool", "tool_name": name, "content": output_text(output) }));
            }
            // Ollama recibe las imágenes en base64 dentro del propio mensaje, sin el prefijo `data:`.
            Message::Image { note, image } => messages.push(json!({ "role": "user", "content": note, "images": [image.base64] })),
        }
    }
    let mut request = json!({
        "model": model,
        "messages": messages,
        "stream": stream,
        "options": { "num_ctx": CONTEXT_TOKENS },
    });
    if let Some(think) = think {
        request["think"] = Value::Bool(think);
    }
    if !tools.is_empty() {
        request["tools"] = tools
            .iter()
            .map(|tool| json!({ "type": "function", "function": { "name": tool.name, "description": tool.description, "parameters": tool.parameters } }))
            .collect();
    }
    request
}

/// Los errores de Ollama son `{ "error": "texto" }`. Los más comunes se traducen a una acción concreta.
fn map_api_error(status: u16, body: &Value, model: &str) -> LlmError {
    let message = body["error"].as_str().or_else(|| body["error"]["message"].as_str()).unwrap_or("sin detalles");
    let text = if status == 404 && message.contains("not found") {
        format!("El modelo «{model}» no está descargado en Ollama. Descárgalo con: ollama pull {model}")
    } else if message.contains("image input") || message.contains("does not support images") || message.contains("vision") {
        format!("El modelo «{model}» no puede ver imágenes. Para mirar la pantalla usa uno con visión, como qwen3.5:9b")
    } else if message.contains("does not support tools") {
        format!("El modelo «{model}» no admite herramientas, que Ryan necesita. Prueba con otro como qwen3:8b o llama3.1:8b")
    } else {
        format!("Ollama respondió {status}: {message}")
    };
    // Unsupported muestra el texto tal cual; Api hablaría de la API key, que aquí no existe.
    LlmError::Unsupported(text)
}

/// Los modelos con razonamiento (DeepSeek-R1, Qwen3...) pueden dejar su cadena de pensamiento
/// entre `<think>` y `</think>` dentro del texto; no es parte de la respuesta.
fn strip_thinking(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out.trim().to_string(),
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn parse_response(status: u16, body: &Value, model: &str) -> Result<LlmResponse, LlmError> {
    if status >= 400 {
        return Err(map_api_error(status, body, model));
    }
    let message = body.get("message").filter(|m| m.is_object()).ok_or_else(|| LlmError::BadResponse("la respuesta no trae «message»".into()))?;
    let text = strip_thinking(message["content"].as_str().unwrap_or(""));
    Ok(LlmResponse { text, tool_calls: parse_tool_calls(&message["tool_calls"])?, raw: None })
}

fn parse_tool_calls(calls: &Value) -> Result<Vec<ToolInvocation>, LlmError> {
    let mut tool_calls = Vec::new();
    for call in calls.as_array().into_iter().flatten() {
        let function = &call["function"];
        let name = function["name"].as_str().ok_or_else(|| LlmError::BadResponse("llamada a herramienta sin nombre".into()))?;
        let args = match &function["arguments"] {
            Value::Object(_) => function["arguments"].clone(),
            // Algunos modelos devuelven los argumentos como texto JSON.
            Value::String(text) if !text.trim().is_empty() => serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone())),
            _ => json!({}),
        };
        tool_calls.push(ToolInvocation { id: call["id"].as_str().map(str::to_string), name: name.to_string(), args });
    }
    Ok(tool_calls)
}

/// Quita el razonamiento (`<think>…</think>`) del texto que llega en trozos, aunque una etiqueta venga partida
/// entre dos trozos («<th» + «ink>»). También descarta los saltos de línea iniciales, que suelen seguir al razonamiento.
#[derive(Default)]
struct ThinkFilter {
    pending: String,
    in_think: bool,
    started: bool,
}

/// Cuántos caracteres del final de `text` podrían ser el comienzo de `tag` (y hay que esperar a ver el siguiente trozo).
fn partial_suffix_len(text: &str, tag: &str) -> usize {
    (1..tag.len()).rev().find(|&length| text.ends_with(&tag[..length])).unwrap_or(0)
}

impl ThinkFilter {
    fn feed(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        self.drain(false)
    }

    /// Fin del texto: lo que se retenía por si era una etiqueta ya no puede serlo.
    fn finish(&mut self) -> String {
        self.drain(true)
    }

    fn drain(&mut self, last: bool) -> String {
        let mut out = String::new();
        loop {
            if self.in_think {
                if let Some(end) = self.pending.find("</think>") {
                    self.pending.drain(..end + "</think>".len());
                    self.in_think = false;
                    continue;
                }
                let keep = if last { 0 } else { partial_suffix_len(&self.pending, "</think>") };
                self.pending.drain(..self.pending.len() - keep);
                break;
            }
            if let Some(start) = self.pending.find("<think>") {
                out.push_str(&self.pending[..start]);
                self.pending.drain(..start + "<think>".len());
                self.in_think = true;
                continue;
            }
            let keep = if last { 0 } else { partial_suffix_len(&self.pending, "<think>") };
            let cut = self.pending.len() - keep;
            out.push_str(&self.pending[..cut]);
            self.pending.drain(..cut);
            break;
        }
        if !self.started {
            out = out.trim_start().to_string();
            self.started = !out.is_empty();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "create_folder".into(),
            description: "Crea una carpeta".into(),
            parameters: json!({ "type": "object", "properties": { "name": { "type": "string" } } }),
        }
    }

    #[test]
    fn la_peticion_no_hace_streaming_amplia_el_contexto_y_declara_herramientas() {
        let request = build_request("qwen3:8b", "eres una mascota", &[Message::User("hola".into())], &[spec()]);
        assert_eq!(request["model"], "qwen3:8b");
        assert_eq!(request["stream"], false);
        assert_eq!(request["options"]["num_ctx"], 8192);
        assert_eq!(request["messages"][0], json!({ "role": "system", "content": "eres una mascota" }));
        assert_eq!(request["messages"][1], json!({ "role": "user", "content": "hola" }));
        let tool = &request["tools"][0];
        assert_eq!((tool["type"].as_str(), tool["function"]["name"].as_str()), (Some("function"), Some("create_folder")));
        assert!(build_request("m", "s", &[], &[]).get("tools").is_none());
    }

    #[test]
    fn el_turno_con_herramienta_y_su_resultado_se_reconstruyen() {
        let history = vec![
            Message::Assistant {
                text: "".into(),
                tool_calls: vec![
                    ToolInvocation { id: None, name: "t".into(), args: json!({ "a": 1 }) },
                    ToolInvocation { id: None, name: "u".into(), args: json!("texto suelto") },
                ],
                raw: None,
            },
            Message::ToolResult { call_id: None, name: "t".into(), output: json!({ "summary": "ok" }), is_error: false },
        ];
        let request = build_request("m", "s", &history, &[]);
        assert_eq!(request["messages"][1]["tool_calls"][0], json!({ "function": { "name": "t", "arguments": { "a": 1 } } }));
        assert_eq!(request["messages"][1]["tool_calls"][1]["function"]["arguments"], json!({}));
        assert_eq!(request["messages"][2], json!({ "role": "tool", "tool_name": "t", "content": "{\"summary\":\"ok\"}" }));
    }

    #[test]
    fn la_imagen_va_en_el_campo_images_sin_prefijo_data() {
        use zen_core::ai::Image;
        let history = vec![
            Message::User("¿qué ves?".into()),
            Message::Image { note: "Imagen obtenida con look_at_screen.".into(), image: Image { mime: "image/png".into(), base64: "QUJD".into() } },
        ];
        let request = build_request("qwen3.5:9b", "s", &history, &[]);
        assert_eq!(
            request["messages"][2],
            json!({ "role": "user", "content": "Imagen obtenida con look_at_screen.", "images": ["QUJD"] })
        );
        assert!(OllamaProvider::new(DEFAULT_URL, "m".into()).supports_images());
    }

    #[test]
    fn un_modelo_sin_vision_explica_que_modelo_usar() {
        let body = json!({ "error": "this model is missing data required for image input" });
        let error = parse_response(500, &body, "qwen3:8b").unwrap_err().to_string();
        assert!(error.contains("no puede ver imágenes") && error.contains("qwen3.5:9b"), "{error}");
    }

    #[test]
    fn interpreta_texto_y_llamadas_a_herramientas() {
        let body = json!({ "message": { "role": "assistant", "content": "Creo la carpeta.", "tool_calls": [
            { "function": { "name": "create_folder", "arguments": { "name": "X" } } },
            { "id": "call_7", "function": { "name": "t", "arguments": "{\"a\":2}" } },
            { "function": { "name": "sin_args" } }
        ] } });
        let response = parse_response(200, &body, "m").unwrap();
        assert_eq!(response.text, "Creo la carpeta.");
        assert_eq!(response.tool_calls[0], ToolInvocation { id: None, name: "create_folder".into(), args: json!({ "name": "X" }) });
        assert_eq!(response.tool_calls[1], ToolInvocation { id: Some("call_7".into()), name: "t".into(), args: json!({ "a": 2 }) });
        assert_eq!(response.tool_calls[2].args, json!({}));
    }

    #[test]
    fn el_razonamiento_no_forma_parte_de_la_respuesta() {
        assert_eq!(strip_thinking("<think>mmm\nhay que saludar</think>\n\nHola"), "Hola");
        assert_eq!(strip_thinking("Hola <think>a</think>y <think>b</think>adiós"), "Hola y adiós");
        assert_eq!(strip_thinking("Hola<think>sin cerrar"), "Hola");
        assert_eq!(strip_thinking("sin pensar"), "sin pensar");
    }

    #[test]
    fn los_errores_comunes_dicen_que_hacer() {
        let missing = parse_response(404, &json!({ "error": "model \"qwen3:8b\" not found, try pulling it first" }), "qwen3:8b").unwrap_err();
        assert!(missing.to_string().contains("ollama pull qwen3:8b"), "{missing}");
        let no_tools = parse_response(400, &json!({ "error": "registry.ollama.ai/library/deepseek-r1:8b does not support tools" }), "deepseek-r1:8b").unwrap_err();
        assert!(no_tools.to_string().contains("no admite herramientas") && !no_tools.to_string().contains("API key"), "{no_tools}");
        let other = parse_response(500, &json!({ "error": "boom" }), "m").unwrap_err();
        assert!(other.to_string().contains("500") && other.to_string().contains("boom") && !other.to_string().contains("API key"));
        assert!(matches!(parse_response(200, &json!({}), "m"), Err(LlmError::BadResponse(_))));
    }

    #[test]
    fn valida_la_direccion_del_servidor() {
        assert_eq!(normalize_base_url(" http://localhost:11434/ "), Ok("http://localhost:11434".into()));
        assert_eq!(normalize_base_url("https://mi-servidor.local"), Ok("https://mi-servidor.local".into()));
        for bad in ["", "localhost:11434", "ftp://x", "http://", "http:///x", "http://a b", "http://u:p@host", "http://host?x=1", "http://host/#a", "file:///c:/x"] {
            assert!(normalize_base_url(bad).is_err(), "{bad:?}");
        }
        assert!(normalize_base_url(&format!("http://{}", "a".repeat(200))).is_err());
    }

    /// Servidor mínimo que responde una vez con `body` y devuelve lo que recibió (cabeceras y cuerpo).
    fn fake_ollama(status: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                received.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let length = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if rest.len() >= length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&received).to_string()
        });
        (url, handle)
    }

    /// Como `fake_ollama`, pero atiende varias peticiones seguidas (una respuesta por cada una).
    fn fake_ollama_sequence(replies: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut received = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    received.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&received).to_string();
                    if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                        let length = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                        if rest.len() >= length {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).unwrap();
                requests.push(String::from_utf8_lossy(&received).to_string());
            }
            requests
        });
        (url, handle)
    }

    fn sent_json(request: &str) -> Value {
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    #[test]
    fn por_defecto_se_pide_sin_razonamiento_y_se_puede_activar() {
        let ok = json!({ "message": { "content": "Hola" } }).to_string();
        let (url, server) = fake_ollama_sequence(vec![("200 OK", ok.clone()), ("200 OK", ok)]);
        OllamaProvider::new(&url, "qwen3.5:9b".into()).generate("s", &[Message::User("hola".into())], &[]).unwrap();
        OllamaProvider::new(&url, "qwen3.5:9b".into()).with_think(true).generate("s", &[Message::User("hola".into())], &[]).unwrap();
        let requests = server.join().unwrap();
        assert_eq!(sent_json(&requests[0])["think"], false);
        assert_eq!(sent_json(&requests[1])["think"], true);
    }

    #[test]
    fn un_modelo_que_no_admite_razonar_se_reintenta_sin_el_parametro() {
        let rejected = json!({ "error": "registry.ollama.ai/library/llama3.1:8b does not support thinking" }).to_string();
        let ok = json!({ "message": { "content": "Hola" } }).to_string();
        let (url, server) = fake_ollama_sequence(vec![("400 Bad Request", rejected.clone()), ("200 OK", ok)]);
        let response = OllamaProvider::new(&url, "llama3.1:8b".into()).generate("s", &[Message::User("hola".into())], &[]).unwrap();
        assert_eq!(response.text, "Hola");
        let requests = server.join().unwrap();
        assert!(sent_json(&requests[0]).get("think").is_some());
        assert!(sent_json(&requests[1]).get("think").is_none(), "el reintento no lleva el parámetro");

        // Lo mismo con la respuesta en directo.
        let stream_ok = ndjson(&[json!({ "message": { "content": "Hola" }, "done": false }), json!({ "done": true })]);
        let (url, server) = fake_ollama_sequence(vec![("400 Bad Request", rejected), ("200 OK", stream_ok)]);
        let mut text = String::new();
        let response = OllamaProvider::new(&url, "llama3.1:8b".into()).generate_stream("s", &[Message::User("hola".into())], &[], &mut |d| text.push_str(d)).unwrap();
        assert_eq!((text.as_str(), response.text.as_str()), ("Hola", "Hola"));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn el_filtro_de_razonamiento_funciona_aunque_las_etiquetas_lleguen_partidas() {
        let mut filter = ThinkFilter::default();
        let mut visible = String::new();
        for chunk in ["<th", "ink>pienso", " mucho</thi", "nk>\n\nHo", "la <", "b>mundo", "<"] {
            visible.push_str(&filter.feed(chunk));
        }
        visible.push_str(&filter.finish());
        assert_eq!(visible, "Hola <b>mundo<", "el «<» suelto del final se conserva al terminar");
        // Un razonamiento sin cerrar no se muestra nunca.
        let mut filter = ThinkFilter::default();
        let shown = filter.feed("Hola<think>nunca termina") + &filter.finish();
        assert_eq!(shown, "Hola");
        // Sin razonamiento, el texto pasa tal cual (salvo los espacios iniciales).
        let mut filter = ThinkFilter::default();
        assert_eq!(filter.feed("  Hola ") + &filter.feed("mundo"), "Hola mundo");
    }

    fn ndjson(lines: &[Value]) -> String {
        lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n"
    }

    #[test]
    fn la_respuesta_en_directo_se_reenvia_por_trozos_y_se_reconstruye() {
        let body = ndjson(&[
            json!({ "message": { "role": "assistant", "content": "<think>pen" }, "done": false }),
            json!({ "message": { "role": "assistant", "content": "sando</think>\n\nHo" }, "done": false }),
            json!({ "message": { "role": "assistant", "content": "la" }, "done": false }),
            json!({ "message": { "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": "create_folder", "arguments": { "name": "Fotos" } } }] }, "done": false }),
            json!({ "message": { "role": "assistant", "content": "" }, "done": true }),
        ]);
        let (url, server) = fake_ollama("200 OK", &body);
        let provider = OllamaProvider::new(&url, "qwen3.5:9b".into());
        let mut deltas = Vec::new();
        let response = provider.generate_stream("sistema", &[Message::User("hola".into())], &[spec()], &mut |d| deltas.push(d.to_string())).unwrap();
        assert_eq!(deltas.concat(), "Hola");
        assert!(deltas.len() >= 2, "llegó en más de un trozo: {deltas:?}");
        assert_eq!(response.text, "Hola");
        assert_eq!(response.tool_calls[0].name, "create_folder");
        assert_eq!(response.tool_calls[0].args, json!({ "name": "Fotos" }));
        let request = server.join().unwrap();
        let sent: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(sent["stream"], true);
    }

    #[test]
    fn un_error_a_mitad_de_la_respuesta_en_directo_se_devuelve_como_error() {
        let body = ndjson(&[json!({ "message": { "content": "Ho" }, "done": false }), json!({ "error": "model runner has unexpectedly stopped" })]);
        let (url, server) = fake_ollama("200 OK", &body);
        let mut deltas = Vec::new();
        let error = OllamaProvider::new(&url, "m".into()).generate_stream("s", &[Message::User("hola".into())], &[], &mut |d| deltas.push(d.to_string())).unwrap_err();
        assert!(error.to_string().contains("unexpectedly stopped"), "{error}");
        assert_eq!(deltas, ["Ho"], "lo que llegó antes se mostró");
        server.join().unwrap();

        let (url, server) = fake_ollama("404 Not Found", &json!({ "error": "model \"nada\" not found" }).to_string());
        let error = OllamaProvider::new(&url, "nada".into()).generate_stream("s", &[Message::User("hola".into())], &[], &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("ollama pull nada"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn habla_con_un_servidor_ollama_de_punta_a_punta() {
        let answer = json!({ "message": { "role": "assistant", "content": "<think>x</think>Hola", "tool_calls": [
            { "function": { "name": "create_folder", "arguments": { "name": "Fotos" } } } ] } })
        .to_string();
        let (url, server) = fake_ollama("200 OK", &answer);
        let provider = OllamaProvider::new(&format!("{url}/"), "qwen3:8b".into());
        let response = provider.generate("sistema", &[Message::User("hola".into())], &[spec()]).unwrap();
        assert_eq!(response.text, "Hola");
        assert_eq!(response.tool_calls[0].args, json!({ "name": "Fotos" }));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/chat "), "{request}");
        assert!(!request.to_ascii_lowercase().contains("authorization"), "no se manda ninguna clave");
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!((body["stream"].clone(), body["model"].clone()), (json!(false), json!("qwen3:8b")));
    }

    #[test]
    fn lista_los_modelos_y_explica_los_errores_del_servidor() {
        let (url, server) = fake_ollama("200 OK", &json!({ "models": [{ "name": "qwen3:8b" }, { "name": "deepseek-r1:8b" }] }).to_string());
        assert_eq!(list_models(&url).unwrap(), ["qwen3:8b", "deepseek-r1:8b"]);
        assert!(server.join().unwrap().starts_with("GET /api/tags "));

        let (url, server) = fake_ollama("404 Not Found", &json!({ "error": "model \"nada\" not found, try pulling it first" }).to_string());
        let error = OllamaProvider::new(&url, "nada".into()).generate("s", &[Message::User("hola".into())], &[]).unwrap_err();
        assert!(error.to_string().contains("ollama pull nada"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn si_ollama_no_esta_abierto_el_error_dice_que_hacer() {
        // Se reserva un puerto libre y se cierra: nadie escucha en él.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let error = list_models(&format!("http://127.0.0.1:{port}")).unwrap_err();
        assert!(matches!(&error, LlmError::Network(m) if m.contains("Ollama no responde") && m.contains("ollama.com/download")), "{error}");
    }

    /// Necesita Ollama abierto. Se ejecuta con `cargo test -p zen-ai -- --ignored ollama_real --nocapture`.
    #[test]
    #[ignore = "necesita Ollama en marcha"]
    fn ollama_real_lista_los_modelos() {
        let models = list_models(DEFAULT_URL).expect("Ollama debería responder");
        println!("modelos: {models:?}");
    }

    /// Respuesta en directo con un modelo real: muestra cuándo llega el primer trozo y cuántos son.
    /// Se ejecuta con `ZEN_TEST_OLLAMA_MODEL=qwen3.5:9b cargo test -p zen-ai -- --ignored --nocapture ollama_real_en_directo`.
    #[test]
    #[ignore = "necesita Ollama y un modelo descargado"]
    fn ollama_real_en_directo() {
        let model = std::env::var("ZEN_TEST_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let started = std::time::Instant::now();
        let mut chunks: Vec<(u128, String)> = Vec::new();
        let response = OllamaProvider::new(DEFAULT_URL, model)
            .generate_stream("Eres un asistente breve.", &[Message::User("Cuéntame en tres frases qué es un faro.".into())], &[], &mut |text| {
                chunks.push((started.elapsed().as_millis(), text.to_string()));
            })
            .unwrap();
        println!("primer trozo a los {} ms; {} trozos; total {} ms", chunks.first().map(|c| c.0).unwrap_or(0), chunks.len(), started.elapsed().as_millis());
        println!("texto: {}", response.text);
        assert!(chunks.len() > 3, "debería llegar en varios trozos");
        assert_eq!(chunks.iter().map(|c| c.1.as_str()).collect::<String>().trim(), response.text.trim());
    }

    /// Envía el PNG de `ZEN_TEST_IMAGE` a un modelo con visión (`ZEN_TEST_OLLAMA_MODEL`) y pregunta qué ve.
    #[test]
    #[ignore = "necesita Ollama y un modelo con visión"]
    fn ollama_real_describe_una_imagen() {
        use base64::Engine;
        use zen_core::ai::Image;
        let model = std::env::var("ZEN_TEST_OLLAMA_MODEL").unwrap_or_else(|_| "qwen3.5:9b".into());
        let bytes = std::fs::read(std::env::var("ZEN_TEST_IMAGE").expect("falta ZEN_TEST_IMAGE")).unwrap();
        let image = Image { mime: "image/png".into(), base64: base64::engine::general_purpose::STANDARD.encode(bytes) };
        let history = vec![
            Message::User("Mira la ventana y dime, en español, qué factura es, cuándo vence y qué error muestra.".into()),
            Message::Image { note: "Imagen obtenida con look_at_screen.".into(), image },
        ];
        let started = std::time::Instant::now();
        let response = OllamaProvider::new(DEFAULT_URL, model).generate("Eres un asistente breve.", &history, &[]).unwrap();
        println!("({} s) {}", started.elapsed().as_secs(), response.text);
        assert!(!response.text.is_empty());
    }

    /// Con `ZEN_TEST_OLLAMA_MODEL` (por ejemplo qwen3:8b) hace una pregunta de verdad.
    #[test]
    #[ignore = "necesita Ollama y un modelo descargado"]
    fn ollama_real_responde_y_pide_una_herramienta() {
        let model = std::env::var("ZEN_TEST_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let provider = OllamaProvider::new(DEFAULT_URL, model);
        let response = provider.generate("Eres un asistente breve.", &[Message::User("Di hola en una frase.".into())], &[]).unwrap();
        println!("respuesta: {}", response.text);
        assert!(!response.text.is_empty());
        let ask = Message::User("Crea una carpeta llamada Fotos usando la herramienta.".into());
        let response = provider.generate("Usa las herramientas cuando se pida una acción.", &[ask], &[spec()]).unwrap();
        println!("herramientas: {:?}", response.tool_calls);
        assert_eq!(response.tool_calls.first().map(|c| c.name.as_str()), Some("create_folder"));
    }
}
