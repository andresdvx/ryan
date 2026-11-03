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

pub struct OllamaProvider {
    base_url: String,
    model: String,
    http: HttpClient,
}

impl OllamaProvider {
    pub fn new(base_url: &str, model: String) -> Self {
        let base_url = normalize_base_url(base_url).unwrap_or_else(|_| DEFAULT_URL.to_string());
        Self { base_url, model, http: HttpClient::with_timeout(TIMEOUT_SECS) }
    }
}

impl LlmProvider for OllamaProvider {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError> {
        let body = build_request(&self.model, system, history, tools);
        let (status, value) = self
            .http
            .post_json(&format!("{}/api/chat", self.base_url), &[], &body)
            .map_err(|error| unreachable_hint(error, &self.base_url))?;
        parse_response(status, &value, &self.model)
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

fn build_request(model: &str, system: &str, history: &[Message], tools: &[ToolSpec]) -> Value {
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
        "stream": false,
        "options": { "num_ctx": CONTEXT_TOKENS },
    });
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
    let mut tool_calls = Vec::new();
    for call in message["tool_calls"].as_array().into_iter().flatten() {
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
    Ok(LlmResponse { text, tool_calls, raw: None })
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
