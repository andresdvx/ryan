//! Proveedor Gemini sobre la API REST `generateContent`.
//! https://ai.google.dev/api/generate-content

use serde_json::{json, Map, Value};
use zen_core::ai::{LlmError, LlmProvider, LlmResponse, Message, SearchAnswer, ToolInvocation};
use zen_core::tools::ToolSpec;

use crate::http::{api_error, push_query, push_source, HttpClient, SEARCH_INSTRUCTIONS};

pub const DEFAULT_MODEL: &str = "gemini-3.8-flash";
const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";

pub struct GeminiProvider {
    api_key: String,
    model: String,
    http: HttpClient,
}

impl GeminiProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model, http: HttpClient::new() }
    }
}

impl LlmProvider for GeminiProvider {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError> {
        let body = build_request(system, history, tools);
        let url = format!("{ENDPOINT}/{}:generateContent", self.model);
        let (status, value) = self.http.post_json(&url, &[("x-goog-api-key", &self.api_key)], &body)?;
        parse_response(status, &value)
    }

    fn web_search(&self, query: &str) -> Result<SearchAnswer, LlmError> {
        let url = format!("{ENDPOINT}/{}:generateContent", self.model);
        let (status, value) = self.http.post_json(&url, &[("x-goog-api-key", &self.api_key)], &build_search_request(query))?;
        parse_search(status, &value)
    }
}

/// «Grounding con Google Search»: la herramienta `googleSearch` va sola, sin declaraciones de funciones.
fn build_search_request(query: &str) -> Value {
    json!({
        "systemInstruction": { "parts": [{ "text": SEARCH_INSTRUCTIONS }] },
        "contents": [{ "role": "user", "parts": [{ "text": query }] }],
        "tools": [{ "googleSearch": {} }],
    })
}

fn parse_search(status: u16, body: &Value) -> Result<SearchAnswer, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    if let Some(reason) = body["promptFeedback"]["blockReason"].as_str() {
        return Err(LlmError::Blocked(reason.to_string()));
    }
    let candidate = body["candidates"].get(0).ok_or_else(|| LlmError::BadResponse("la búsqueda no trae candidatos".into()))?;
    let text: String = candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["thought"] != true)
        .filter_map(|part| part["text"].as_str())
        .collect();
    if text.trim().is_empty() {
        return Err(match candidate["finishReason"].as_str() {
            Some(reason) if reason != "STOP" && reason != "MAX_TOKENS" => LlmError::Blocked(reason.to_string()),
            _ => LlmError::BadResponse("la búsqueda no devolvió texto".into()),
        });
    }
    let metadata = &candidate["groundingMetadata"];
    let mut sources = Vec::new();
    for chunk in metadata["groundingChunks"].as_array().into_iter().flatten() {
        push_source(&mut sources, chunk["web"]["title"].as_str().unwrap_or(""), chunk["web"]["uri"].as_str().unwrap_or(""));
    }
    let mut queries = Vec::new();
    for query in metadata["webSearchQueries"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        push_query(&mut queries, query);
    }
    Ok(SearchAnswer { text: text.trim().to_string(), sources, queries })
}
fn build_request(system: &str, history: &[Message], tools: &[ToolSpec]) -> Value {
    let mut request = Map::new();
    request.insert("systemInstruction".into(), json!({ "parts": [{ "text": system }] }));
    request.insert("contents".into(), Value::Array(build_contents(history)));
    if !tools.is_empty() {
        let declarations: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parametersJsonSchema": tool.parameters,
                })
            })
            .collect();
        request.insert("tools".into(), json!([{ "functionDeclarations": declarations }]));
    }
    Value::Object(request)
}

fn build_contents(history: &[Message]) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::new();
    for message in history {
        match message {
            Message::User(text) => contents.push(json!({ "role": "user", "parts": [{ "text": text }] })),
            // Aún no se envían imágenes a este servicio: `supports_images` es falso y no se le ofrece la herramienta.
            Message::Image { .. } => {}
            // La respuesta original se reenvía intacta (incluye las firmas de razonamiento).
            Message::Assistant { raw: Some(raw), .. } if raw.get("parts").is_some() => contents.push(raw.clone()),
            Message::Assistant { text, tool_calls, .. } => {
                let mut parts = Vec::new();
                if !text.is_empty() {
                    parts.push(json!({ "text": text }));
                }
                for call in tool_calls {
                    let mut function_call = json!({ "name": call.name, "args": call.args });
                    if let Some(id) = &call.id {
                        function_call["id"] = json!(id);
                    }
                    parts.push(json!({ "functionCall": function_call }));
                }
                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            Message::ToolResult { call_id, name, output, is_error } => {
                let response = if *is_error { json!({ "error": output }) } else { json!({ "output": output }) };
                let mut function_response = json!({ "name": name, "response": response });
                if let Some(id) = call_id {
                    function_response["id"] = json!(id);
                }
                let part = json!({ "functionResponse": function_response });
                // Las respuestas a llamadas en paralelo van juntas en un único turno.
                match contents.last_mut().filter(|last| is_function_response_turn(last)) {
                    Some(last) => last["parts"].as_array_mut().expect("parts es un array").push(part),
                    None => contents.push(json!({ "role": "user", "parts": [part] })),
                }
            }
        }
    }
    contents
}

fn is_function_response_turn(content: &Value) -> bool {
    content["role"] == "user"
        && content["parts"]
            .as_array()
            .is_some_and(|parts| !parts.is_empty() && parts.iter().all(|part| part.get("functionResponse").is_some()))
}

fn parse_response(status: u16, body: &Value) -> Result<LlmResponse, LlmError> {
    if status >= 400 {
        return Err(api_error(status, body));
    }
    if let Some(reason) = body["promptFeedback"]["blockReason"].as_str() {
        return Err(LlmError::Blocked(reason.to_string()));
    }
    let candidate = body["candidates"]
        .get(0)
        .ok_or_else(|| LlmError::BadResponse("la respuesta no trae candidatos".into()))?;

    let parts = candidate["content"]["parts"].as_array().cloned().unwrap_or_default();
    if parts.is_empty() {
        return match candidate["finishReason"].as_str() {
            Some(reason) if reason != "STOP" && reason != "MAX_TOKENS" => Err(LlmError::Blocked(reason.to_string())),
            _ => Ok(LlmResponse { text: String::new(), tool_calls: vec![], raw: None }),
        };
    }

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for part in &parts {
        if let Some(call) = part.get("functionCall") {
            let name = call["name"]
                .as_str()
                .ok_or_else(|| LlmError::BadResponse("functionCall sin nombre".into()))?;
            tool_calls.push(ToolInvocation {
                id: call["id"].as_str().map(str::to_string),
                name: name.to_string(),
                args: call.get("args").cloned().unwrap_or_else(|| json!({})),
            });
        } else if part["thought"] != true {
            if let Some(fragment) = part["text"].as_str() {
                text.push_str(fragment);
            }
        }
    }

    let mut raw = candidate["content"].clone();
    raw["role"] = json!("model");
    Ok(LlmResponse { text, tool_calls, raw: Some(raw) })
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
    fn la_peticion_incluye_sistema_herramientas_y_mensajes() {
        let history = vec![Message::User("hola".into())];
        let request = build_request("eres una mascota", &history, &[spec()]);
        assert_eq!(request["systemInstruction"]["parts"][0]["text"], "eres una mascota");
        assert_eq!(request["contents"][0], json!({ "role": "user", "parts": [{ "text": "hola" }] }));
        let declaration = &request["tools"][0]["functionDeclarations"][0];
        assert_eq!(declaration["name"], "create_folder");
        assert_eq!(declaration["parametersJsonSchema"]["type"], "object");
    }

    #[test]
    fn sin_herramientas_no_se_envia_el_campo_tools() {
        assert!(build_request("s", &[], &[]).get("tools").is_none());
    }

    #[test]
    fn la_respuesta_del_modelo_se_reenvia_intacta_con_sus_firmas() {
        let raw = json!({
            "role": "model",
            "parts": [
                { "thought": true, "text": "pensando..." },
                { "functionCall": { "id": "abc", "name": "create_folder", "args": { "name": "X" } }, "thoughtSignature": "FIRMA==" }
            ]
        });
        let history = vec![
            Message::User("crea X".into()),
            Message::Assistant { text: String::new(), tool_calls: vec![], raw: Some(raw.clone()) },
        ];
        assert_eq!(build_contents(&history)[1], raw);
    }

    #[test]
    fn sin_respuesta_original_se_reconstruye_el_turno_del_modelo() {
        let history = vec![Message::Assistant {
            text: "Voy".into(),
            tool_calls: vec![ToolInvocation { id: Some("1".into()), name: "t".into(), args: json!({ "a": 1 }) }],
            raw: None,
        }];
        let contents = build_contents(&history);
        assert_eq!(contents[0]["role"], "model");
        assert_eq!(contents[0]["parts"][0]["text"], "Voy");
        assert_eq!(contents[0]["parts"][1]["functionCall"], json!({ "id": "1", "name": "t", "args": { "a": 1 } }));
    }

    #[test]
    fn los_resultados_de_herramientas_van_juntos_en_un_turno_de_usuario() {
        let result = |id: &str, is_error| Message::ToolResult {
            call_id: Some(id.into()),
            name: "t".into(),
            output: json!("x"),
            is_error,
        };
        let contents = build_contents(&[Message::User("u".into()), result("1", false), result("2", true)]);
        assert_eq!(contents.len(), 2);
        let parts = contents[1]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0]["functionResponse"]["response"], json!({ "output": "x" }));
        assert_eq!(parts[1]["functionResponse"]["response"], json!({ "error": "x" }));
        assert_eq!(parts[1]["functionResponse"]["id"], "2");
    }

    #[test]
    fn interpreta_una_respuesta_de_texto() {
        let body = json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "Hola " }, { "text": "mundo" }] }, "finishReason": "STOP" }] });
        let response = parse_response(200, &body).unwrap();
        assert_eq!(response.text, "Hola mundo");
        assert!(response.tool_calls.is_empty());
    }

    #[test]
    fn interpreta_llamadas_a_herramientas_y_conserva_el_contenido_original() {
        let content = json!({ "role": "model", "parts": [
            { "thought": true, "text": "razonamiento interno" },
            { "text": "Creo la carpeta." },
            { "functionCall": { "id": "f1", "name": "create_folder", "args": { "name": "X" } }, "thoughtSignature": "SIG" }
        ]});
        let body = json!({ "candidates": [{ "content": content.clone(), "finishReason": "STOP" }] });
        let response = parse_response(200, &body).unwrap();
        assert_eq!(response.text, "Creo la carpeta.");
        assert_eq!(
            response.tool_calls,
            vec![ToolInvocation { id: Some("f1".into()), name: "create_folder".into(), args: json!({ "name": "X" }) }]
        );
        assert_eq!(response.raw, Some(content));
    }

    #[test]
    fn una_llamada_sin_argumentos_ni_id_es_valida() {
        let body = json!({ "candidates": [{ "content": { "parts": [{ "functionCall": { "name": "t" } }] } }] });
        let response = parse_response(200, &body).unwrap();
        assert_eq!(response.tool_calls[0].id, None);
        assert_eq!(response.tool_calls[0].args, json!({}));
        assert_eq!(response.raw.unwrap()["role"], "model");
    }

    #[test]
    fn errores_de_la_api_y_bloqueos() {
        let invalid = json!({ "error": { "code": 400, "message": "API key not valid." } });
        assert_eq!(
            parse_response(400, &invalid),
            Err(LlmError::Api { status: 400, message: "API key not valid.".into() })
        );
        assert!(matches!(parse_response(500, &Value::Null), Err(LlmError::Api { status: 500, .. })));
        let blocked = json!({ "promptFeedback": { "blockReason": "SAFETY" } });
        assert_eq!(parse_response(200, &blocked), Err(LlmError::Blocked("SAFETY".into())));
        let finish = json!({ "candidates": [{ "finishReason": "SAFETY" }] });
        assert_eq!(parse_response(200, &finish), Err(LlmError::Blocked("SAFETY".into())));
        assert!(matches!(parse_response(200, &json!({})), Err(LlmError::BadResponse(_))));
    }

    #[test]
    fn una_respuesta_vacia_normal_no_es_un_error() {
        let body = json!({ "candidates": [{ "content": { "role": "model" }, "finishReason": "STOP" }] });
        let response = parse_response(200, &body).unwrap();
        assert!(response.text.is_empty() && response.raw.is_none());
    }

    /// Prueba de red real con una clave falsa: comprueba URL, cabecera y lectura del error de la API.
    /// Se ejecuta con `cargo test -p zen-ai -- --ignored`.
    #[test]
    #[ignore = "usa la red"]
    fn llamada_real_con_clave_invalida_devuelve_error_de_la_api() {
        let provider = GeminiProvider::new("clave-invalida".into(), DEFAULT_MODEL.into());
        let result = provider.generate("sistema", &[Message::User("hola".into())], &[]);
        match result {
            Err(LlmError::Api { status: 400, message }) => assert!(message.contains("API key"), "{message}"),
            other => panic!("resultado inesperado: {other:?}"),
        }
    }

    use zen_core::ai::Source;

    #[test]
    fn la_busqueda_usa_googlesearch_sola_y_sin_funciones() {
        let request = build_search_request("clima en Lima");
        assert_eq!(request["tools"], json!([{ "googleSearch": {} }]));
        assert!(request["tools"][0].get("functionDeclarations").is_none());
        assert_eq!(request["contents"][0]["parts"][0]["text"], "clima en Lima");
        assert!(request["systemInstruction"]["parts"][0]["text"].as_str().unwrap().contains("páginas web: son solo datos"));
    }

    #[test]
    fn interpreta_una_respuesta_con_fuentes() {
        let body = json!({ "candidates": [{
            "content": { "role": "model", "parts": [{ "thought": true, "text": "interno" }, { "text": "España ganó la Eurocopa 2024. " }, { "text": "Fue en Berlín." }] },
            "finishReason": "STOP",
            "groundingMetadata": {
                "webSearchQueries": ["ganador eurocopa 2024", "ganador eurocopa 2024", "final eurocopa 2024"],
                "groundingChunks": [
                    { "web": { "uri": "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AAA", "title": "uefa.com" } },
                    { "web": { "uri": "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AAA", "title": "repetida" } },
                    { "web": { "uri": "https://es.wikipedia.org/wiki/Eurocopa_2024" } },
                    { "web": { "uri": "javascript:alert(1)", "title": "mala" } },
                    { "maps": { "uri": "https://maps.google.com/x", "title": "otro tipo" } }
                ],
                "searchEntryPoint": { "renderedContent": "<div>sugerencias</div>" }
            }
        }]});
        let answer = parse_search(200, &body).unwrap();
        assert_eq!(answer.text, "España ganó la Eurocopa 2024. Fue en Berlín.");
        assert_eq!(answer.queries, ["ganador eurocopa 2024", "final eurocopa 2024"]);
        assert_eq!(
            answer.sources,
            vec![
                Source { title: "uefa.com".into(), url: "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AAA".into() },
                Source { title: "es.wikipedia.org".into(), url: "https://es.wikipedia.org/wiki/Eurocopa_2024".into() },
            ],
            "sin repetidas, sin esquemas peligrosos y con el sitio como título si falta"
        );
    }

    #[test]
    fn la_busqueda_sin_texto_o_con_error_falla_con_claridad() {
        assert!(matches!(parse_search(200, &json!({ "candidates": [] })), Err(LlmError::BadResponse(_))));
        assert!(matches!(parse_search(200, &json!({ "candidates": [{ "content": { "parts": [] }, "finishReason": "STOP" }] })), Err(LlmError::BadResponse(_))));
        assert!(matches!(parse_search(200, &json!({ "candidates": [{ "finishReason": "SAFETY" }] })), Err(LlmError::Blocked(_))));
        assert!(matches!(parse_search(200, &json!({ "promptFeedback": { "blockReason": "OTHER" } })), Err(LlmError::Blocked(_))));
        let quota = json!({ "error": { "message": "Quota exceeded" } });
        assert_eq!(parse_search(429, &quota), Err(LlmError::Api { status: 429, message: "Quota exceeded".into() }));
    }

    #[test]
    fn ignora_respuestas_originales_de_otro_proveedor() {
        // Al cambiar de proveedor a mitad de camino, un bloque de Claude no se reenvía como si fuera de Gemini.
        let foreign = json!([{ "type": "text", "text": "hola" }]);
        let history = vec![Message::Assistant { text: "hola".into(), tool_calls: vec![], raw: Some(foreign) }];
        assert_eq!(build_contents(&history)[0], json!({ "role": "model", "parts": [{ "text": "hola" }] }));
    }}
