//! Contrato entre el sistema y cada herramienta. El modelo de IA solo conoce `ToolSpec`;
//! quien valida, decide permisos y ejecuta es el programa.

mod registry;

use std::fmt;

use serde_json::Value;

use crate::ai::Image;

pub use registry::ToolRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    /// Se ejecuta sin preguntar.
    Safe,
    /// Pide confirmación al usuario.
    Confirm,
    /// Pide confirmación explícita y detallada; la política nunca puede rebajarla.
    Dangerous,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolError {
    UnknownTool(String),
    InvalidArguments(String),
    NotAllowed(String),
    Failed(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::UnknownTool(name) => write!(f, "La herramienta «{name}» no existe."),
            ToolError::InvalidArguments(msg) => write!(f, "Argumentos inválidos: {msg}"),
            ToolError::NotAllowed(msg) => write!(f, "No permitido: {msg}"),
            ToolError::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// Resultado estructurado: `summary` es para el usuario, `data` para el resto del sistema.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub summary: String,
    pub data: Value,
}

/// Petición de ejecución tal como la formula la IA o el usuario.
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

/// Lo que se le describe al modelo de IA: nombre, descripción y esquema JSON de los parámetros.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Esquema JSON de los parámetros.
    fn parameters(&self) -> Value;
    /// El riesgo puede depender de los argumentos (por ejemplo, cuántos archivos se mueven).
    fn risk(&self, args: &Value) -> RiskLevel;
    /// Comprueba los argumentos sin efectos secundarios.
    fn validate(&self, args: &Value) -> Result<(), ToolError>;
    /// Texto para el usuario: qué va a hacer exactamente y sobre qué.
    fn preview(&self, args: &Value) -> String;
    /// Solo se llama después de `validate` y de pasar por la política de permisos.
    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError>;
    /// Imágenes que dejó la última ejecución para que las mire el modelo; se entregan una sola vez.
    fn take_images(&self) -> Vec<Image> {
        Vec::new()
    }
}

/// Herramientas cuyo resultado es una imagen: solo se ofrecen a proveedores que `supports_images`.
pub const VISION_TOOLS: [&str; 1] = ["look_at_screen"];