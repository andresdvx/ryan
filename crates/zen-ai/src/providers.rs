use zen_core::ai::LlmProvider;

use crate::{claude, gemini, ollama, openai, OllamaSettings};

/// Servicios de IA que el usuario puede elegir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Gemini,
    Claude,
    OpenAi,
    /// Modelos locales servidos por Ollama: sin API key.
    Ollama,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 4] = [ProviderKind::Gemini, ProviderKind::Claude, ProviderKind::OpenAi, ProviderKind::Ollama];

    /// ¿Puede este proveedor recibir imágenes? De momento solo Ollama (con un modelo con visión).
    pub fn supports_images(self) -> bool {
        self == ProviderKind::Ollama
    }

    /// Los servicios en la nube exigen una API key; Ollama corre en el propio equipo.
    pub fn needs_key(self) -> bool {
        self != ProviderKind::Ollama
    }

    /// Identificador estable: se guarda en los ajustes y en el Administrador de credenciales.
    pub fn id(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "gemini",
            ProviderKind::Claude => "claude",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Ollama => "ollama",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "Google Gemini",
            ProviderKind::Claude => "Anthropic Claude",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::Ollama => "Ollama (modelo local)",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Gemini => gemini::DEFAULT_MODEL,
            ProviderKind::Claude => claude::DEFAULT_MODEL,
            ProviderKind::OpenAi => openai::DEFAULT_MODEL,
            ProviderKind::Ollama => ollama::DEFAULT_MODEL,
        }
    }

    /// Variable de entorno que se usa como alternativa a la clave guardada (vacía si no hay clave).
    pub fn env_var(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "GEMINI_API_KEY",
            ProviderKind::Claude => "ANTHROPIC_API_KEY",
            ProviderKind::OpenAi => "OPENAI_API_KEY",
            ProviderKind::Ollama => "",
        }
    }

    /// Dónde conseguir la clave (o, para Ollama, el programa).
    pub fn key_url(self) -> &'static str {
        match self {
            ProviderKind::Gemini => "aistudio.google.com/apikey",
            ProviderKind::Claude => "console.anthropic.com/settings/keys",
            ProviderKind::OpenAi => "platform.openai.com/api-keys",
            ProviderKind::Ollama => "ollama.com/download",
        }
    }

    /// `api_key` solo la usan los servicios en la nube; `local` solo Ollama.
    pub fn create(self, api_key: String, model: String, local: &OllamaSettings) -> Box<dyn LlmProvider> {
        match self {
            ProviderKind::Gemini => Box::new(gemini::GeminiProvider::new(api_key, model)),
            ProviderKind::Claude => Box::new(claude::ClaudeProvider::new(api_key, model)),
            ProviderKind::OpenAi => Box::new(openai::OpenAiProvider::new(api_key, model)),
            ProviderKind::Ollama => Box::new(ollama::OllamaProvider::new(&local.url, model).with_think(local.think)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Red real con claves falsas: las tres búsquedas llegan a su servicio y el rechazo por clave se interpreta bien.
    /// Se ejecuta con `cargo test -p zen-ai -- --ignored busqueda_real`.
    #[test]
    #[ignore = "usa la red"]
    fn busqueda_real_con_claves_invalidas_devuelve_el_error_de_la_api() {
        use zen_core::ai::LlmError;
        for kind in ProviderKind::ALL.into_iter().filter(|kind| kind.needs_key()) {
            let provider = kind.create("clave-invalida".into(), kind.default_model().into(), &OllamaSettings::default());
            match provider.web_search("clima en Lima") {
                Err(LlmError::Api { status, message }) => {
                    assert!([400, 401, 403].contains(&status), "{}: {status} {message}", kind.id());
                    println!("{:<7} {status}: {message}", kind.id());
                }
                other => panic!("{}: resultado inesperado {other:?}", kind.id()),
            }
        }
    }

    #[test]
    fn los_identificadores_son_unicos_y_se_pueden_recuperar() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::from_id(kind.id()), Some(kind));
            assert!(!kind.default_model().is_empty());
            assert_eq!(kind.env_var().is_empty(), !kind.needs_key(), "solo Ollama carece de variable de clave");
        }
        assert_eq!(ProviderKind::from_id("otro"), None);
        let mut ids: Vec<_> = ProviderKind::ALL.iter().map(|k| k.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
    }

    #[test]
    fn solo_ollama_funciona_sin_clave() {
        assert!(!ProviderKind::Ollama.needs_key());
        assert!(ProviderKind::ALL.iter().filter(|k| **k != ProviderKind::Ollama).all(|k| k.needs_key()));
    }

    #[test]
    fn ollama_no_tiene_busqueda_web_integrada() {
        use zen_core::ai::LlmError;
        let provider = ProviderKind::Ollama.create(String::new(), "qwen3:8b".into(), &OllamaSettings::default());
        assert!(matches!(provider.web_search("clima"), Err(LlmError::Unsupported(_))));
    }
}