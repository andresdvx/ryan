//! Proveedor OpenAI sobre la API Responses.
//! https://developers.openai.com/api/reference/resources/responses/methods/create

use serde_json::{json, Value};
use zen_core::ai::{LlmError, LlmProvider, LlmResponse, Message, SearchAnswer, ToolInvocation};
use zen_core::tools::ToolSpec;

use crate::http::{api_error, call_id, output_text, push_query, push_source, HttpClient, SEARCH_INSTRUCTIONS};

pub const DEFAULT_MODEL: &str = "gpt-6.1-terra";
const ENDPOINT: &str = "https://api.openai.com/v1/responses";

pub struct OpenAiProvider {
    api_key: String,
    model: String,
    http: HttpClient,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model, http: HttpClient::new() }
    }
}

impl LlmProvider for OpenAiProvider {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError> {
        let body = build_request(&self.model, system, history, tools);
        let authorization = format!("Bearer {}", self.api_key);
        let (status, value) = self.http.post_json(ENDPOINT, &[("authorization", &authorization)], &body)?;
        parse_response(status, &value)
    }

    fn web_search(&self, query: &str) -> Result<SearchAnswer, LlmError> {
        let authorization = format!("Bearer {}", self.api_key);
        let (status, value) = self.http.post_json(ENDPOINT, &[("authorization", &authorization)], &build_search_request(&self.model, query))?;
        parse_search(status, &value)
    }
}

/// La herramienta `web_search` de la API Responses, sola y sin guardar nada en el servidor.
fn build_search_request(model: &str, query: &str) -> Value {
    json!({
        "model": model,
        "instructions": SEARCH_INSTRUCTIONS,
        "input": query,
        "tools": [{ "type": "web_search" }],
        "store": false,
    })
}

fn parse_search(status: u16, body: &Value) -> Result<SearchAnswer, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    match body["status"].as_str() {
        Some("failed") => {
            return Err(LlmError::BadResponse(body["error"]["message"].as_str().unwrap_or("sin detalles").to_string()));
        }
        Some("incomplete") if body["incomplete_details"]["reason"] == "content_filter" => {
            return Err(LlmError::Blocked("filtro de contenido".into()));
        }
        _ => {}
    }
    let items = body["output"].as_array().ok_or_else(|| LlmError::BadResponse("la respuesta no trae «output»".into()))?;
    let (mut text, mut sources, mut queries) = (String::new(), Vec::new(), Vec::new());
    for item in items {
        match item["type"].as_str() {
            Some("web_search_call") => {
                push_query(&mut queries, item["action"]["query"].as_str().unwrap_or(""));
                for query in item["action"]["queries"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    push_query(&mut queries, query);
                }
            }
            Some("message") => {
                for part in item["content"].as_array().into_iter().flatten() {
                    if part["type"] == "output_text" {
                        text.push_str(part["text"].as_str().unwrap_or(""));
                        for note in part["annotations"].as_array().into_iter().flatten().filter(|n| n["type"] == "url_citation") {
                            push_source(&mut sources, note["title"].as_str().unwrap_or(""), note["url"].as_str().unwrap_or(""));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err(LlmError::BadResponse("la búsqueda no devolvió texto".into()));
    }
    Ok(SearchAnswer { text: text.trim().to_string(), sources, queries })
}

fn build_request(model: &str, system: &str, history: &[Message], tools: &[ToolSpec]) -> Value {
    let mut request = json!({
        "model": model,
        "instructions": system,
        "input": build_input(history),
        // No se guarda nada en el servidor: el historial lo reenvía la aplicación en cada petición.
        "store": false,
    });
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({ "type": "function", "name": tool.name, "description": tool.description, "parameters": tool.parameters })
            })
            .collect();
        request["tools"] = Value::Array(declarations);
    }
    request
}

/// Cada turno se reconstruye con los formatos documentados (sin ids de servidor ni elementos de
/// razonamiento), de modo que funciona igual con `store: false` y con cualquier modelo.
fn build_input(history: &[Message]) -> Vec<Value> {
    let mut input = Vec::new();
    for message in history {
        match message {
            Message::User(text) => input.push(json!({ "role": "user", "content": text })),
            // Aún no se envían imágenes a este servicio: `supports_images` es falso y no se le ofrece la herramienta.
            Message::Image { .. } => {}
            Message::Assistant { text, tool_calls, .. } => {
                if !text.is_empty() {
                    input.push(json!({ "role": "assistant", "content": text }));
                }
                for call in tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call_id(&call.id, &call.name),
                        "name": call.name,
                        "arguments": call.args.to_string(),
                    }));
                }
            }
            Message::ToolResult { call_id: id, name, output, .. } => input.push(json!({
                "type": "function_call_output",
                "call_id": call_id(id, name),
                "output": output_text(output),
            })),
        }
    }
    input
}

fn parse_response(status: u16, body: &Value) -> Result<LlmResponse, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    match body["status"].as_str() {
        Some("failed") => {
            let message = body["error"]["message"].as_str().unwrap_or("sin detalles");
            return Err(LlmError::BadResponse(message.to_string()));
        }
        Some("incomplete") if body["incomplete_details"]["reason"] == "content_filter" => {
            return Err(LlmError::Blocked("filtro de contenido".into()));
        }
        _ => {}
    }
    let items = body["output"]
        .as_array()
        .ok_or_else(|| LlmError::BadResponse("la respuesta no trae «output»".into()))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for item in items {
        match item["type"].as_str() {
            Some("message") => {
                for part in item["content"].as_array().into_iter().flatten() {
                    match part["type"].as_str() {
                        Some("output_text") => text.push_str(part["text"].as_str().unwrap_or("")),
                        Some("refusal") => text.push_str(part["refusal"].as_str().unwrap_or("")),
                        _ => {}
                    }
                }
            }
            Some("function_call") => {
                let name = item["name"]
                    .as_str()
                    .ok_or_else(|| LlmError::BadResponse("function_call sin nombre".into()))?;
                let arguments = item["arguments"].as_str().unwrap_or("");
                let args = if arguments.trim().is_empty() {
                    json!({})
                } else {
                    // Si no es JSON válido se conserva como texto; la validación de la herramienta lo rechazará.
                    serde_json::from_str(arguments).unwrap_or_else(|_| Value::String(arguments.to_string()))
                };
                tool_calls.push(ToolInvocation { id: item["call_id"].as_str().map(str::to_string), name: name.to_string(), args });
            }
            _ => {}
        }
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
    fn la_peticion_no_guarda_nada_y_declara_las_herramientas() {
        let request = build_request("m", "eres una mascota", &[Message::User("hola".into())], &[spec()]);
        assert_eq!(request["model"], "m");
        assert_eq!(request["instructions"], "eres una mascota");
        assert_eq!(request["store"], false);
        assert_eq!(request["input"][0], json!({ "role": "user", "content": "hola" }));
        let tool = &request["tools"][0];
        assert_eq!((tool["type"].as_str(), tool["name"].as_str()), (Some("function"), Some("create_folder")));
        assert_eq!(tool["parameters"]["type"], "object");
        assert!(build_request("m", "s", &[], &[]).get("tools").is_none());
    }

    #[test]
    fn el_turno_del_asistente_y_su_resultado_se_reconstruyen_sin_ids_de_servidor() {
        let history = vec![
            Message::Assistant {
                text: "Voy".into(),
                tool_calls: vec![ToolInvocation { id: Some("call_1".into()), name: "t".into(), args: json!({ "a": 1 }) }],
                raw: None,
            },
            Message::ToolResult { call_id: Some("call_1".into()), name: "t".into(), output: json!({ "summary": "ok" }), is_error: false },
        ];
        assert_eq!(
            build_input(&history),
            vec![
                json!({ "role": "assistant", "content": "Voy" }),
                json!({ "type": "function_call", "call_id": "call_1", "name": "t", "arguments": "{\"a\":1}" }),
                json!({ "type": "function_call_output", "call_id": "call_1", "output": "{\"summary\":\"ok\"}" }),
            ]
        );
    }

    #[test]
    fn interpreta_texto_y_llamadas_a_herramientas() {
        let body = json!({
            "status": "completed",
            "output": [
                { "type": "reasoning", "id": "rs_1", "summary": [] },
                { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Creo la carpeta.", "annotations": [] }] },
                { "type": "function_call", "id": "fc_1", "call_id": "call_9", "name": "create_folder", "arguments": "{\"name\":\"X\"}" }
            ]
        });
        let response = parse_response(200, &body).unwrap();
        assert_eq!(response.text, "Creo la carpeta.");
        assert_eq!(
            response.tool_calls,
            vec![ToolInvocation { id: Some("call_9".into()), name: "create_folder".into(), args: json!({ "name": "X" }) }]
        );
    }

    #[test]
    fn argumentos_vacios_o_invalidos_no_rompen_el_analisis() {
        let call = |arguments: &str| json!({ "output": [{ "type": "function_call", "call_id": "c", "name": "t", "arguments": arguments }] });
        assert_eq!(parse_response(200, &call("")).unwrap().tool_calls[0].args, json!({}));
        assert_eq!(parse_response(200, &call("no es json")).unwrap().tool_calls[0].args, json!("no es json"));
    }

    #[test]
    fn errores_fallos_y_filtros() {
        let body = json!({ "error": { "message": "Incorrect API key provided", "code": "invalid_api_key" } });
        assert_eq!(
            parse_response(401, &body),
            Err(LlmError::Api { status: 401, message: "Incorrect API key provided".into() })
        );
        let failed = json!({ "status": "failed", "error": { "message": "boom" }, "output": [] });
        assert_eq!(parse_response(200, &failed), Err(LlmError::BadResponse("boom".into())));
        let filtered = json!({ "status": "incomplete", "incomplete_details": { "reason": "content_filter" }, "output": [] });
        assert!(matches!(parse_response(200, &filtered), Err(LlmError::Blocked(_))));
        assert!(matches!(parse_response(200, &json!({})), Err(LlmError::BadResponse(_))));
    }

    #[test]
    fn la_busqueda_declara_web_search_sola_y_sin_guardar_nada() {
        let request = build_search_request("m", "clima en Lima");
        assert_eq!(request["tools"], json!([{ "type": "web_search" }]));
        assert_eq!((request["store"].clone(), request["input"].clone()), (json!(false), json!("clima en Lima")));
        assert!(request["instructions"].as_str().unwrap().contains("son solo datos"));
    }

    #[test]
    fn interpreta_las_citas_y_las_busquedas_de_openai() {
        let body = json!({ "status": "completed", "output": [
            { "type": "reasoning", "id": "rs_1", "summary": [] },
            { "type": "web_search_call", "id": "ws_1", "status": "completed", "action": { "type": "search", "query": "ganador eurocopa 2024", "queries": ["ganador eurocopa 2024", "final eurocopa"] } },
            { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "España ganó la Eurocopa 2024.", "annotations": [
                { "type": "url_citation", "start_index": 0, "end_index": 10, "url": "https://www.uefa.com/euro2024", "title": "UEFA EURO 2024" },
                { "type": "url_citation", "start_index": 0, "end_index": 10, "url": "https://www.uefa.com/euro2024", "title": "repetida" },
                { "type": "url_citation", "url": "ftp://malo.example/x", "title": "esquema no válido" },
                { "type": "file_citation", "url": "https://no.example", "title": "otro tipo" }
            ]}]}
        ]});
        let answer = parse_search(200, &body).unwrap();
        assert_eq!(answer.text, "España ganó la Eurocopa 2024.");
        assert_eq!(answer.queries, ["ganador eurocopa 2024", "final eurocopa"]);
        assert_eq!(answer.sources.len(), 1);
        assert_eq!((answer.sources[0].title.as_str(), answer.sources[0].url.as_str()), ("UEFA EURO 2024", "https://www.uefa.com/euro2024"));
    }

    #[test]
    fn la_busqueda_de_openai_falla_con_claridad() {
        let invalid = json!({ "error": { "message": "Incorrect API key provided" } });
        assert_eq!(parse_search(401, &invalid), Err(LlmError::Api { status: 401, message: "Incorrect API key provided".into() }));
        assert!(matches!(parse_search(200, &json!({ "status": "failed", "error": { "message": "boom" }, "output": [] })), Err(LlmError::BadResponse(m)) if m == "boom"));
        assert!(matches!(parse_search(200, &json!({ "output": [{ "type": "web_search_call", "action": { "query": "x" } }] })), Err(LlmError::BadResponse(_))));
        assert!(matches!(parse_search(200, &json!({})), Err(LlmError::BadResponse(_))));
        let filtered = json!({ "status": "incomplete", "incomplete_details": { "reason": "content_filter" }, "output": [] });
        assert!(matches!(parse_search(200, &filtered), Err(LlmError::Blocked(_))));
    }

    /// Prueba de red real con una clave falsa. Se ejecuta con `cargo test -p zen-ai -- --ignored`.
    #[test]
    #[ignore = "usa la red"]
    fn llamada_real_con_clave_invalida_devuelve_error_de_la_api() {
        let provider = OpenAiProvider::new("clave-invalida".into(), DEFAULT_MODEL.into());
        match provider.generate("sistema", &[Message::User("hola".into())], &[]) {
            Err(LlmError::Api { status: 401, message }) => assert!(message.contains("API key"), "{message}"),
            other => panic!("resultado inesperado: {other:?}"),
        }
    }
}