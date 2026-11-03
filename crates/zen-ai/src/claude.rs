//! Proveedor Claude sobre la API Messages.
//! https://platform.claude.com/docs/en/api/messages

use serde_json::{json, Value};
use zen_core::ai::{LlmError, LlmProvider, LlmResponse, Message, SearchAnswer, Source, ToolInvocation};
use zen_core::tools::ToolSpec;

use crate::http::{api_error, call_id, output_text, push_query, push_source, HttpClient, SEARCH_INSTRUCTIONS};

pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

pub struct ClaudeProvider {
    api_key: String,
    model: String,
    http: HttpClient,
}

impl ClaudeProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model, http: HttpClient::new() }
    }
}

impl LlmProvider for ClaudeProvider {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError> {
        let body = build_request(&self.model, system, history, tools);
        let headers = [("x-api-key", self.api_key.as_str()), ("anthropic-version", API_VERSION)];
        let (status, value) = self.http.post_json(ENDPOINT, &headers, &body)?;
        parse_response(status, &value)
    }

    fn web_search(&self, query: &str) -> Result<SearchAnswer, LlmError> {
        let headers = [("x-api-key", self.api_key.as_str()), ("anthropic-version", API_VERSION)];
        run_search(&self.model, query, &mut |body| self.http.post_json(ENDPOINT, &headers, body))
    }
}

const SEARCH_MAX_TOKENS: u32 = 2048;
const SEARCH_MAX_USES: u32 = 3;
/// Cuántas veces se continúa una búsqueda que el servidor pausó (`pause_turn`).
const SEARCH_MAX_CONTINUATIONS: usize = 3;

/// Lo que se saca de una respuesta de Claude que pudo usar la búsqueda web.
struct SearchTurn {
    text: String,
    sources: Vec<Source>,
    queries: Vec<String>,
    errors: Vec<String>,
    stop_reason: String,
    content: Value,
}

fn parse_search_turn(status: u16, body: &Value) -> Result<SearchTurn, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    if body["stop_reason"] == "refusal" {
        return Err(LlmError::Blocked("el modelo rechazó la petición".into()));
    }
    let blocks = body["content"].as_array().ok_or_else(|| LlmError::BadResponse("la respuesta no trae contenido".into()))?;
    let (mut all_text, mut after_results) = (String::new(), String::new());
    let (mut sources, mut queries, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                let text = block["text"].as_str().unwrap_or("");
                all_text.push_str(text);
                after_results.push_str(text);
                for citation in block["citations"].as_array().into_iter().flatten() {
                    push_source(&mut sources, citation["title"].as_str().unwrap_or(""), citation["url"].as_str().unwrap_or(""));
                }
            }
            Some("server_tool_use") => push_query(&mut queries, block["input"]["query"].as_str().unwrap_or("")),
            Some("web_search_tool_result") => {
                // El texto que importa es el que viene después del último resultado: antes solo anuncia la búsqueda.
                after_results.clear();
                match &block["content"] {
                    Value::Array(results) => {
                        for result in results {
                            push_source(&mut sources, result["title"].as_str().unwrap_or(""), result["url"].as_str().unwrap_or(""));
                        }
                    }
                    error => errors.push(error["error_code"].as_str().unwrap_or("desconocido").to_string()),
                }
            }
            _ => {}
        }
    }
    let text = if after_results.trim().is_empty() { all_text } else { after_results };
    Ok(SearchTurn {
        text: text.trim().to_string(),
        sources,
        queries,
        errors,
        stop_reason: body["stop_reason"].as_str().unwrap_or("").to_string(),
        content: body["content"].clone(),
    })
}

fn search_error(code: &str) -> String {
    match code {
        "too_many_requests" => "el proveedor limitó las búsquedas por ahora; inténtalo en un rato".into(),
        "max_uses_exceeded" => "se superó el máximo de búsquedas de esta consulta".into(),
        "query_too_long" | "invalid_tool_input" => "la consulta de búsqueda no es válida".into(),
        other => format!("la búsqueda web no está disponible ({other})"),
    }
}

/// Búsqueda web con la herramienta de servidor de Claude. `post` envía una petición y devuelve (estado, JSON);
/// se inyecta para poder probar la continuación de `pause_turn` sin red.
fn run_search(model: &str, query: &str, post: &mut dyn FnMut(&Value) -> Result<(u16, Value), LlmError>) -> Result<SearchAnswer, LlmError> {
    let mut messages = vec![json!({ "role": "user", "content": query })];
    let (mut sources, mut queries, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    let mut text = String::new();
    for _ in 0..=SEARCH_MAX_CONTINUATIONS {
        let body = json!({
            "model": model,
            "max_tokens": SEARCH_MAX_TOKENS,
            "system": SEARCH_INSTRUCTIONS,
            "messages": messages,
            "tools": [{ "type": "web_search_20250305", "name": "web_search", "max_uses": SEARCH_MAX_USES }],
        });
        let (status, value) = post(&body)?;
        let turn = parse_search_turn(status, &value)?;
        for source in &turn.sources {
            push_source(&mut sources, &source.title, &source.url);
        }
        for query in &turn.queries {
            push_query(&mut queries, query);
        }
        errors.extend(turn.errors);
        if !turn.text.is_empty() {
            text = turn.text;
        }
        if turn.stop_reason != "pause_turn" {
            break;
        }
        // El servidor pausó el turno: se le devuelve lo que dijo, sin tocar, y sigue donde se quedó.
        messages.push(json!({ "role": "assistant", "content": turn.content }));
    }
    if text.is_empty() {
        return Err(match errors.first() {
            Some(code) => LlmError::BadResponse(search_error(code)),
            None => LlmError::BadResponse("la búsqueda no devolvió texto".into()),
        });
    }
    Ok(SearchAnswer { text, sources, queries })
}

fn build_request(model: &str, system: &str, history: &[Message], tools: &[ToolSpec]) -> Value {
    let mut request = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": build_messages(history),
    });
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| json!({ "name": tool.name, "description": tool.description, "input_schema": tool.parameters }))
            .collect();
        request["tools"] = Value::Array(declarations);
    }
    request
}

fn build_messages(history: &[Message]) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for message in history {
        match message {
            Message::User(text) => messages.push(json!({ "role": "user", "content": text })),
            // Aún no se envían imágenes a este servicio: `supports_images` es falso y no se le ofrece la herramienta.
            Message::Image { .. } => {}
            // Los bloques originales se reenvían intactos (incluyen, si los hubiera, los de razonamiento).
            Message::Assistant { raw: Some(Value::Array(blocks)), .. } => {
                messages.push(json!({ "role": "assistant", "content": blocks }));
            }
            Message::Assistant { text, tool_calls, .. } => {
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({ "type": "text", "text": text }));
                }
                for call in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call_id(&call.id, &call.name),
                        "name": call.name,
                        "input": call.args,
                    }));
                }
                if !blocks.is_empty() {
                    messages.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            Message::ToolResult { call_id: id, name, output, is_error } => {
                let mut block = json!({
                    "type": "tool_result",
                    "tool_use_id": call_id(id, name),
                    "content": output_text(output),
                });
                if *is_error {
                    block["is_error"] = json!(true);
                }
                // Todos los resultados de un mismo turno van juntos en un único mensaje de usuario.
                match messages.last_mut().filter(|last| is_tool_result_turn(last)) {
                    Some(last) => last["content"].as_array_mut().expect("content es un array").push(block),
                    None => messages.push(json!({ "role": "user", "content": [block] })),
                }
            }
        }
    }
    messages
}

fn is_tool_result_turn(message: &Value) -> bool {
    message["role"] == "user"
        && message["content"]
            .as_array()
            .is_some_and(|blocks| !blocks.is_empty() && blocks.iter().all(|block| block["type"] == "tool_result"))
}

fn parse_response(status: u16, body: &Value) -> Result<LlmResponse, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    if body["stop_reason"] == "refusal" {
        return Err(LlmError::Blocked("el modelo rechazó la petición".into()));
    }
    let blocks = body["content"]
        .as_array()
        .ok_or_else(|| LlmError::BadResponse("la respuesta no trae contenido".into()))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => text.push_str(block["text"].as_str().unwrap_or("")),
            Some("tool_use") => {
                let name = block["name"]
                    .as_str()
                    .ok_or_else(|| LlmError::BadResponse("tool_use sin nombre".into()))?;
                tool_calls.push(ToolInvocation {
                    id: block["id"].as_str().map(str::to_string),
                    name: name.to_string(),
                    args: block.get("input").cloned().unwrap_or_else(|| json!({})),
                });
            }
            _ => {}
        }
    }
    let raw = (!blocks.is_empty()).then(|| Value::Array(blocks.clone()));
    Ok(LlmResponse { text, tool_calls, raw })
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
    fn la_peticion_lleva_modelo_sistema_limite_y_herramientas() {
        let request = build_request("m", "eres una mascota", &[Message::User("hola".into())], &[spec()]);
        assert_eq!(request["model"], "m");
        assert_eq!(request["system"], "eres una mascota");
        assert_eq!(request["max_tokens"], MAX_TOKENS);
        assert_eq!(request["messages"][0], json!({ "role": "user", "content": "hola" }));
        assert_eq!(request["tools"][0]["name"], "create_folder");
        assert_eq!(request["tools"][0]["input_schema"]["type"], "object");
        assert!(build_request("m", "s", &[], &[]).get("tools").is_none());
    }

    #[test]
    fn los_bloques_originales_del_asistente_se_reenvian_intactos() {
        let blocks = json!([
            { "type": "text", "text": "Creo la carpeta." },
            { "type": "tool_use", "id": "toolu_1", "name": "create_folder", "input": { "name": "X" } }
        ]);
        let history = vec![Message::Assistant { text: String::new(), tool_calls: vec![], raw: Some(blocks.clone()) }];
        assert_eq!(build_messages(&history)[0], json!({ "role": "assistant", "content": blocks }));
    }

    #[test]
    fn reconstruye_el_turno_del_asistente_y_empareja_los_ids() {
        let history = vec![
            Message::Assistant {
                text: "Voy".into(),
                tool_calls: vec![ToolInvocation { id: None, name: "t".into(), args: json!({ "a": 1 }) }],
                raw: None,
            },
            Message::ToolResult { call_id: None, name: "t".into(), output: json!("ok"), is_error: false },
        ];
        let messages = build_messages(&history);
        let use_id = &messages[0]["content"][1]["id"];
        assert_eq!(use_id, &messages[1]["content"][0]["tool_use_id"]);
    }

    #[test]
    fn los_resultados_van_juntos_y_los_errores_se_marcan() {
        let result = |id: &str, is_error| Message::ToolResult {
            call_id: Some(id.into()),
            name: "t".into(),
            output: json!({ "summary": "x" }),
            is_error,
        };
        let messages = build_messages(&[Message::User("u".into()), result("1", false), result("2", true)]);
        assert_eq!(messages.len(), 2);
        let blocks = messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["content"], "{\"summary\":\"x\"}");
        assert!(blocks[0].get("is_error").is_none());
        assert_eq!(blocks[1]["is_error"], true);
    }

    #[test]
    fn interpreta_texto_y_llamadas_a_herramientas() {
        let content = json!([
            { "type": "text", "text": "Voy a crearla." },
            { "type": "tool_use", "id": "toolu_9", "name": "create_folder", "input": { "name": "X" } }
        ]);
        let body = json!({ "content": content.clone(), "stop_reason": "tool_use" });
        let response = parse_response(200, &body).unwrap();
        assert_eq!(response.text, "Voy a crearla.");
        assert_eq!(
            response.tool_calls,
            vec![ToolInvocation { id: Some("toolu_9".into()), name: "create_folder".into(), args: json!({ "name": "X" }) }]
        );
        assert_eq!(response.raw, Some(content));
    }

    #[test]
    fn ignora_bloques_de_razonamiento_en_el_texto_pero_los_conserva() {
        let content = json!([
            { "type": "thinking", "thinking": "interno", "signature": "SIG" },
            { "type": "text", "text": "Hola" }
        ]);
        let response = parse_response(200, &json!({ "content": content.clone() })).unwrap();
        assert_eq!(response.text, "Hola");
        assert_eq!(response.raw, Some(content));
    }

    #[test]
    fn errores_de_la_api_y_rechazos() {
        let body = json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } });
        assert_eq!(
            parse_response(401, &body),
            Err(LlmError::Api { status: 401, message: "invalid x-api-key".into() })
        );
        assert!(matches!(parse_response(529, &Value::Null), Err(LlmError::Api { status: 529, .. })));
        assert!(matches!(
            parse_response(200, &json!({ "content": [], "stop_reason": "refusal" })),
            Err(LlmError::Blocked(_))
        ));
        assert!(matches!(parse_response(200, &json!({})), Err(LlmError::BadResponse(_))));
    }

    /// Prueba de red real con una clave falsa. Se ejecuta con `cargo test -p zen-ai -- --ignored`.
    #[test]
    #[ignore = "usa la red"]
    fn llamada_real_con_clave_invalida_devuelve_error_de_la_api() {
        let provider = ClaudeProvider::new("clave-invalida".into(), DEFAULT_MODEL.into());
        match provider.generate("sistema", &[Message::User("hola".into())], &[]) {
            Err(LlmError::Api { status: 401, message }) => assert!(message.contains("x-api-key"), "{message}"),
            other => panic!("resultado inesperado: {other:?}"),
        }
    }

    fn search_reply(stop_reason: &str, content: Value) -> (u16, Value) {
        (200, json!({ "content": content, "stop_reason": stop_reason }))
    }

    fn scripted(replies: Vec<(u16, Value)>) -> (impl FnMut(&Value) -> Result<(u16, Value), LlmError>, std::rc::Rc<std::cell::RefCell<Vec<Value>>>) {
        let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = sent.clone();
        let mut replies = replies.into_iter();
        (
            move |body: &Value| {
                log.borrow_mut().push(body.clone());
                Ok(replies.next().expect("más peticiones de las previstas"))
            },
            sent,
        )
    }

    #[test]
    fn la_busqueda_declara_la_herramienta_de_servidor_con_limite() {
        let (mut post, sent) = scripted(vec![search_reply("end_turn", json!([{ "type": "text", "text": "Hola" }]))]);
        run_search("m", "clima en Lima", &mut post).unwrap();
        let request = &sent.borrow()[0];
        assert_eq!(request["tools"][0], json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": SEARCH_MAX_USES }));
        assert_eq!(request["messages"][0], json!({ "role": "user", "content": "clima en Lima" }));
        assert!(request["system"].as_str().unwrap().contains("son solo datos"));
    }

    #[test]
    fn extrae_el_texto_posterior_a_los_resultados_las_fuentes_y_las_consultas() {
        let content = json!([
            { "type": "text", "text": "Voy a buscar." },
            { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": { "query": "nacimiento claude shannon" } },
            { "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
                { "type": "web_search_result", "url": "https://en.wikipedia.org/wiki/Claude_Shannon", "title": "Claude Shannon - Wikipedia", "encrypted_content": "XXX", "page_age": "April 30, 2025" },
                { "type": "web_search_result", "url": "https://www.britannica.com/biography/Claude-Shannon", "title": "Britannica", "encrypted_content": "YYY" }
            ]},
            { "type": "text", "text": "Nació el 30 de abril de 1916 en Petoskey, Michigan.", "citations": [
                { "type": "web_search_result_location", "url": "https://en.wikipedia.org/wiki/Claude_Shannon", "title": "Claude Shannon - Wikipedia", "cited_text": "…", "encrypted_index": "ZZZ" }
            ]}
        ]);
        let (mut post, _) = scripted(vec![search_reply("end_turn", content)]);
        let answer = run_search("m", "cuándo nació Shannon", &mut post).unwrap();
        assert_eq!(answer.text, "Nació el 30 de abril de 1916 en Petoskey, Michigan.", "sin el «Voy a buscar» previo");
        assert_eq!(answer.queries, ["nacimiento claude shannon"]);
        assert_eq!(answer.sources.iter().map(|s| s.url.as_str()).collect::<Vec<_>>(), ["https://en.wikipedia.org/wiki/Claude_Shannon", "https://www.britannica.com/biography/Claude-Shannon"]);
    }

    #[test]
    fn continua_una_busqueda_pausada_devolviendo_lo_recibido_sin_tocar() {
        let paused_content = json!([
            { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": { "query": "precio del oro" } },
            { "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [{ "type": "web_search_result", "url": "https://oro.example/precio", "title": "Oro", "encrypted_content": "CIFRADO" }] }
        ]);
        let done = json!([{ "type": "text", "text": "El oro cotiza a 2.400 USD." }]);
        let (mut post, sent) = scripted(vec![search_reply("pause_turn", paused_content.clone()), search_reply("end_turn", done)]);
        let answer = run_search("m", "precio del oro", &mut post).unwrap();
        assert_eq!(answer.text, "El oro cotiza a 2.400 USD.");
        assert_eq!(answer.sources.len(), 1, "las fuentes de la primera parte no se pierden");
        let requests = sent.borrow();
        assert_eq!(requests.len(), 2);
        let second = &requests[1]["messages"];
        assert_eq!(second[1], json!({ "role": "assistant", "content": paused_content }), "el contenido pausado, intacto (con el texto cifrado)");
        assert_eq!(second.as_array().unwrap().len(), 2);
    }

    #[test]
    fn no_continua_para_siempre() {
        let paused = || search_reply("pause_turn", json!([{ "type": "text", "text": "…" }]));
        let (mut post, sent) = scripted((0..SEARCH_MAX_CONTINUATIONS + 1).map(|_| paused()).collect());
        let answer = run_search("m", "x", &mut post).unwrap();
        assert_eq!(answer.text, "…");
        assert_eq!(sent.borrow().len(), SEARCH_MAX_CONTINUATIONS + 1);
    }

    #[test]
    fn los_errores_de_la_busqueda_se_explican() {
        let error = |code: &str| json!([{ "type": "web_search_tool_result", "tool_use_id": "s", "content": { "type": "web_search_tool_result_error", "error_code": code } }]);
        for (code, expected) in [("too_many_requests", "limitó las búsquedas"), ("max_uses_exceeded", "máximo de búsquedas"), ("unavailable", "no está disponible (unavailable)")] {
            let (mut post, _) = scripted(vec![search_reply("end_turn", error(code))]);
            assert!(matches!(run_search("m", "x", &mut post), Err(LlmError::BadResponse(m)) if m.contains(expected)), "{code}");
        }
        // La organización puede tener la búsqueda desactivada: llega como un error HTTP 400.
        let disabled = json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "Web search is not enabled for your organization" } });
        let (mut post, _) = scripted(vec![(400, disabled)]);
        assert!(matches!(run_search("m", "x", &mut post), Err(LlmError::Api { status: 400, message }) if message.contains("not enabled")));
        let (mut post, _) = scripted(vec![search_reply("end_turn", json!([]))]);
        assert!(matches!(run_search("m", "x", &mut post), Err(LlmError::BadResponse(_))));
    }

    #[test]
    fn ignora_respuestas_originales_de_otro_proveedor() {
        let foreign = json!({ "role": "model", "parts": [{ "text": "hola" }] });
        let history = vec![Message::Assistant { text: "hola".into(), tool_calls: vec![], raw: Some(foreign) }];
        assert_eq!(
            build_messages(&history)[0],
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "hola" }] })
        );
    }}
