//! Interfaz neutral entre el asistente y cualquier proveedor de IA (Gemini, Claude, OpenAI...).
//! Cada proveedor traduce estos tipos a su API; el resto de la aplicación no los conoce.

use std::fmt;

use serde_json::Value;

use crate::tools::ToolSpec;

/// Herramienta que el modelo pide usar.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    pub id: Option<String>,
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolInvocation>,
        /// Respuesta original del proveedor. Algunos exigen reenviarla intacta (por ejemplo, las
        /// firmas de razonamiento de Gemini), así que se guarda sin interpretar.
        raw: Option<Value>,
    },
    ToolResult {
        call_id: Option<String>,
        name: String,
        output: Value,
        is_error: bool,
    },
    /// Una imagen que el modelo debe mirar (por ejemplo la captura de una ventana). Va después de los
    /// resultados de herramientas del mismo turno y solo se conserva la más reciente.
    Image { note: String, image: Image },
}

/// Imagen lista para enviar a un modelo con visión.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    /// Tipo de contenido, por ejemplo `image/png`.
    pub mime: String,
    /// Contenido codificado en base64 (sin el prefijo `data:`).
    pub base64: String,
}

// Una captura puede ocupar cientos de KB: no se vuelca entera en los registros.
impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}, {} bytes en base64)", self.mime, self.base64.len())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    pub text: String,
    pub tool_calls: Vec<ToolInvocation>,
    pub raw: Option<Value>,
}

/// Una fuente consultada por la búsqueda web.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub title: String,
    pub url: String,
}

/// Respuesta de una búsqueda web hecha con la búsqueda integrada del proveedor de IA.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchAnswer {
    pub text: String,
    pub sources: Vec<Source>,
    /// Consultas que realmente se lanzaron al buscador.
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    /// El proveedor no ofrece esa capacidad.
    Unsupported(String),
    Network(String),
    /// El servicio no respondió a tiempo (segundos de espera).
    Timeout(u64),
    Api { status: u16, message: String },
    Blocked(String),
    BadResponse(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Network(msg) => write!(f, "no pude conectar con el servicio de IA ({msg})"),
            LlmError::Unsupported(what) => write!(f, "{what}"),
            LlmError::Timeout(seconds) => write!(
                f,
                "el servicio de IA no respondió en {seconds} s. Puede ser un modelo lento o un problema de conexión; prueba con otro modelo en Ajustes"
            ),
            LlmError::Api { status: 400 | 401 | 403, message } => {
                write!(f, "el servicio de IA rechazó la petición: {message}. Revisa tu API key con /key")
            }
            LlmError::Api { status: 429, .. } => {
                write!(f, "se superó el límite de uso del servicio de IA; espera un momento e inténtalo de nuevo")
            }
            LlmError::Api { status, message } => write!(f, "error {status} del servicio de IA: {message}"),
            LlmError::Blocked(reason) => write!(f, "el servicio de IA bloqueó la respuesta ({reason})"),
            LlmError::BadResponse(msg) => write!(f, "respuesta inesperada del servicio de IA ({msg})"),
        }
    }
}

impl std::error::Error for LlmError {}

pub trait LlmProvider: Send + Sync {
    fn generate(&self, system: &str, history: &[Message], tools: &[ToolSpec]) -> Result<LlmResponse, LlmError>;

    /// ¿Sabe este proveedor recibir imágenes (`Message::Image`)? Si no, no se le ofrecen las herramientas que las producen.
    fn supports_images(&self) -> bool {
        false
    }

    /// Una búsqueda web aparte, con la búsqueda integrada del proveedor (sin tocar la conversación).
    fn web_search(&self, _query: &str) -> Result<SearchAnswer, LlmError> {
        Err(LlmError::Unsupported("este proveedor de IA no tiene búsqueda web integrada".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_mensaje_de_timeout_dice_que_hacer() {
        let message = LlmError::Timeout(90).to_string();
        assert!(message.contains("90 s") && message.contains("otro modelo"));
    }
}
