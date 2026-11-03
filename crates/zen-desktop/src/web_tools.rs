//! Abrir sitios web y buscar música. Todo se abre con el navegador o la aplicación predeterminados
//! a través del explorador de Windows, nunca con un shell.

use std::net::IpAddr;

use serde_json::{json, Value};
use url::{Host, Url};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, optional_str, required_str};

const MAX_URL_LEN: usize = 2048;
const MAX_QUERY_LEN: usize = 100;

/// Sitios muy conocidos que se abren sin preguntar. El resto pide confirmación.
const TRUSTED_SITES: &[&str] = &[
    "google.com",
    "youtube.com",
    "wikipedia.org",
    "github.com",
    "stackoverflow.com",
    "spotify.com",
    "microsoft.com",
    "mozilla.org",
    "openai.com",
    "anthropic.com",
    "claude.ai",
];

/// Abre una URL o un URI con el programa predeterminado (el verbo «open» del explorador de Windows).
/// Sin shell: es un único argumento. Con `explorer.exe <url>` Windows no abría nada cuando el navegador ya
/// estaba en marcha; ShellExecute sí abre una pestaña nueva en ese caso.
pub(crate) fn open_with_shell(target: &str) -> std::io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    if target.chars().any(char::is_control) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "la dirección tiene caracteres de control"));
    }
    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, file) = (wide("open"), wide(target));
    // SAFETY: los textos terminan en cero y viven durante la llamada; el resto de argumentos son nulos.
    let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    // Windows devuelve un valor mayor que 32 si funcionó; si no, un código de error.
    if (result as isize) > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("Windows no pudo abrir la dirección (código {})", result as isize)))
    }
}

/// ¿Es `host` el dominio `site` o uno de sus subdominios? («evilyoutube.com» no cuenta.)
fn host_belongs_to(host: &str, site: &str) -> bool {
    host == site || host.strip_suffix(site).is_some_and(|rest| rest.ends_with('.'))
}

fn is_trusted(host: &str) -> bool {
    TRUSTED_SITES.iter().any(|site| host_belongs_to(host, site))
}

struct Website {
    url: Url,
    host: String,
}

fn parse_website(args: &Value) -> Result<Website, ToolError> {
    check_keys(args, &["url"])?;
    let text = required_str(args, "url")?.trim();
    if text.is_empty() || text.len() > MAX_URL_LEN {
        return Err(invalid(format!("la dirección debe tener entre 1 y {MAX_URL_LEN} caracteres")));
    }
    // El analizador descarta en silencio tabuladores y saltos de línea: se rechazan antes.
    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid("la dirección no puede contener espacios ni caracteres de control"));
    }
    let url = Url::parse(text).map_err(|error| invalid(format!("dirección no válida ({error})")))?;
    if url.scheme() != "https" {
        return Err(ToolError::NotAllowed("solo abro direcciones https://".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ToolError::NotAllowed("no abro direcciones con usuario o contraseña incrustados".into()));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(ToolError::NotAllowed("no abro direcciones con un puerto distinto del habitual".into()));
    }
    let host = match url.host() {
        Some(Host::Domain(domain)) => domain.to_ascii_lowercase(),
        // Las direcciones numéricas (incluidas las de la red local y las «disfrazadas» como 0x7f.1) se
        // rechazan todas: un sitio legítimo tiene nombre.
        Some(Host::Ipv4(ip)) => {
            return Err(ToolError::NotAllowed(format!("no abro direcciones numéricas ({})", IpAddr::V4(ip))));
        }
        Some(Host::Ipv6(_)) => return Err(ToolError::NotAllowed("no abro direcciones numéricas".into())),
        None => return Err(invalid("la dirección no tiene sitio")),
    };
    let local_suffix = [".localhost", ".local", ".internal", ".lan", ".home.arpa", ".intranet"];
    if host == "localhost" || !host.contains('.') || local_suffix.iter().any(|suffix| host.ends_with(suffix)) {
        return Err(ToolError::NotAllowed(format!("«{host}» parece un sitio de tu red local; no lo abro")));
    }
    Ok(Website { url, host })
}

/// Abre una dirección que el propio usuario acaba de pulsar en el chat (por ejemplo, una fuente de una búsqueda).
/// Pasa por las mismas reglas que `open_website`: solo https, sin credenciales ni direcciones locales o numéricas.
pub fn open_url(url: &str) -> Result<(), String> {
    let site = parse_website(&json!({ "url": url })).map_err(|error| error.to_string())?;
    open_with_shell(site.url.as_str()).map_err(|error| format!("No se pudo abrir el navegador: {error}"))
}

pub struct OpenWebsite;

impl Tool for OpenWebsite {
    fn name(&self) -> &'static str {
        "open_website"
    }

    fn description(&self) -> &'static str {
        "Abre una página web (solo https://) en el navegador predeterminado. Los sitios muy conocidos se \
         abren directamente; cualquier otro pide confirmación al usuario. No abre direcciones locales ni \
         numéricas."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "url": { "type": "string", "description": "Dirección completa, por ejemplo https://www.youtube.com" } },
            "required": ["url"]
        })
    }

    /// Depende de la dirección: los sitios conocidos son seguros; el resto, con confirmación.
    fn risk(&self, args: &Value) -> RiskLevel {
        match parse_website(args) {
            Ok(site) if is_trusted(&site.host) => RiskLevel::Safe,
            _ => RiskLevel::Confirm,
        }
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_website(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_website(args) {
            Ok(site) => {
                let mut text = format!("Abrir en tu navegador:\n{}\n\nSitio: {}", site.url, site.host);
                if site.host.split('.').any(|label| label.starts_with("xn--")) {
                    text.push_str("\n⚠ Usa caracteres internacionales: comprueba que es el sitio que esperas.");
                }
                text
            }
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let site = parse_website(args)?;
        open_with_shell(site.url.as_str())
            .map_err(|error| ToolError::Failed(format!("No se pudo abrir el navegador: {error}")))?;
        Ok(ToolOutput { summary: format!("Abrí {} en tu navegador", site.host), data: json!({ "url": site.url.as_str() }) })
    }
}

/// Codifica todo lo que no sea un carácter no reservado, para que la búsqueda no pueda añadir
/// partes a la dirección (por ejemplo `:` o `/`).
pub(crate) fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    Spotify,
    YoutubeMusic,
}

impl Service {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "spotify" => Some(Service::Spotify),
            "youtube_music" => Some(Service::YoutubeMusic),
            _ => None,
        }
    }

    fn display(self) -> &'static str {
        match self {
            Service::Spotify => "Spotify",
            Service::YoutubeMusic => "YouTube Music",
        }
    }
}

/// Qué abrir: la aplicación de Spotify si está instalada; si no, la web.
fn music_target(service: Service, query: &str, spotify_app: bool) -> String {
    let encoded = percent_encode(query);
    match service {
        Service::Spotify if spotify_app => format!("spotify:search:{encoded}"),
        Service::Spotify => format!("https://open.spotify.com/search/{encoded}"),
        Service::YoutubeMusic => format!("https://music.youtube.com/search?q={encoded}"),
    }
}

fn parse_music(args: &Value) -> Result<(Service, String), ToolError> {
    check_keys(args, &["query", "service"])?;
    let query = required_str(args, "query")?.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_LEN || query.chars().any(char::is_control) {
        return Err(invalid(format!("la búsqueda debe tener entre 1 y {MAX_QUERY_LEN} caracteres, sin saltos de línea")));
    }
    let service = match optional_str(args, "service")? {
        None => Service::Spotify,
        Some(key) => Service::from_key(key).ok_or_else(|| invalid(format!("servicio desconocido «{key}»; usa spotify o youtube_music")))?,
    };
    Ok((service, query.to_string()))
}

pub struct SearchMusic;

impl Tool for SearchMusic {
    fn name(&self) -> &'static str {
        "search_music"
    }

    fn description(&self) -> &'static str {
        "Busca música (artista, canción o álbum) en Spotify o YouTube Music. Abre la búsqueda para que el \
         usuario elija y pulse reproducir: no puede iniciar la reproducción por sí sola."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Qué buscar, por ejemplo «Arctic Monkeys»." },
                "service": { "type": "string", "enum": ["spotify", "youtube_music"], "description": "Servicio (opcional; por defecto spotify)." }
            },
            "required": ["query"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_music(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_music(args) {
            Ok((service, query)) => format!("Buscar «{query}» en {}", service.display()),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (service, query) = parse_music(args)?;
        let target = music_target(service, &query, crate::applications::is_installed("spotify"));
        open_with_shell(&target).map_err(|error| ToolError::Failed(format!("No se pudo abrir la búsqueda: {error}")))?;
        let how = if target.starts_with("spotify:") { "en la aplicación de Spotify" } else { "en tu navegador" };
        Ok(ToolOutput {
            summary: format!("Abrí la búsqueda de «{query}» en {} {how}. Pulsa reproducir para escucharla.", service.display()),
            data: json!({ "service": service.display(), "query": query }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(url: &str) -> Result<Website, ToolError> {
        parse_website(&json!({ "url": url }))
    }

    fn risk(url: &str) -> RiskLevel {
        OpenWebsite.risk(&json!({ "url": url }))
    }

    #[test]
    fn acepta_direcciones_https_normales() {
        for url in [
            "https://example.com",
            "https://www.youtube.com/watch?v=abc&t=10",
            "HTTPS://Wikipedia.ORG/wiki/Rust_(lenguaje)",
            "https://sub.dominio.es:443/ruta#parte",
        ] {
            assert!(site(url).is_ok(), "{url}");
        }
        assert_eq!(site("https://WWW.Example.COM/a").unwrap().host, "www.example.com");
    }

    #[test]
    fn rechaza_otros_esquemas() {
        for url in ["http://example.com", "ftp://example.com", "file:///C:/Windows/win.ini", "javascript:alert(1)", "data:text/html,hola", "ms-settings:privacy", "spotify:search:x", "example.com", "//example.com", ""] {
            assert!(site(url).is_err(), "{url}");
        }
    }

    #[test]
    fn rechaza_credenciales_incrustadas_y_puertos_raros() {
        for url in ["https://youtube.com@evil.com", "https://user:pass@example.com", "https://:pass@example.com", "https://example.com:8443", "https://example.com:80"] {
            assert!(matches!(site(url), Err(ToolError::NotAllowed(_))), "{url}");
        }
    }

    #[test]
    fn rechaza_direcciones_numericas_y_de_red_local() {
        for url in [
            "https://127.0.0.1", "https://10.0.0.5", "https://192.168.1.1/admin", "https://169.254.169.254", "https://8.8.8.8",
            "https://[::1]", "https://[fe80::1]", "https://[::ffff:127.0.0.1]", "https://2130706433", "https://0x7f.1",
            "https://localhost", "https://app.localhost", "https://impresora.local", "https://router.lan", "https://intranet", "https://servidor.internal",
        ] {
            assert!(site(url).is_err(), "{url}");
        }
    }

    #[test]
    fn rechaza_espacios_y_caracteres_de_control_y_urls_enormes() {
        for url in ["https://exa\nmple.com", "https://exa\tmple.com", "https://example.com/a b", "https://example.com/\u{0}", "https://exa mple.com"] {
            assert!(site(url).is_err(), "{url:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(site(&long).is_err());
        assert!(OpenWebsite.validate(&json!({ "url": "https://example.com", "extra": 1 })).is_err());
        assert!(OpenWebsite.validate(&json!({ "url": 5 })).is_err());
    }

    #[test]
    fn los_sitios_conocidos_son_seguros_y_el_resto_pide_confirmacion() {
        for url in ["https://youtube.com", "https://www.youtube.com/watch?v=1", "https://es.wikipedia.org/wiki/Rust", "https://github.com/rust-lang/rust", "https://music.youtube.com"] {
            assert_eq!(risk(url), RiskLevel::Safe, "{url}");
        }
        for url in ["https://example.com", "https://notyoutube.com", "https://evilyoutube.com", "https://youtube.com.evil.com", "https://youtube.com.evil.com/youtube.com", "https://xn--youtub-eb3a.com"] {
            assert_eq!(risk(url), RiskLevel::Confirm, "{url}");
        }
        // Una dirección inválida nunca baja a «segura».
        assert_eq!(risk("http://youtube.com"), RiskLevel::Confirm);
        assert_eq!(OpenWebsite.risk(&json!({})), RiskLevel::Confirm);
    }

    #[test]
    fn la_vista_previa_muestra_la_direccion_completa_y_avisa_de_nombres_internacionales() {
        let preview = OpenWebsite.preview(&json!({ "url": "https://example.com/ruta?x=1" }));
        assert!(preview.contains("https://example.com/ruta?x=1") && preview.contains("Sitio: example.com"), "{preview}");
        assert!(!preview.contains('⚠'));
        let idn = OpenWebsite.preview(&json!({ "url": "https://аpple.com" }));
        assert!(idn.contains("xn--") && idn.contains('⚠'), "{idn}");
    }

    #[test]
    fn open_url_rechaza_lo_que_open_website_rechaza() {
        // Las fuentes de una búsqueda vienen de internet: nunca deben poder abrir otra cosa que una página pública.
        for url in ["javascript:alert(1)", "file:///C:/Windows/win.ini", "http://example.com", "https://localhost/admin", "https://192.168.1.1", "https://user:pw@example.com", "ms-msdt:/id x", ""] {
            assert!(open_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn el_dominio_de_confianza_se_compara_por_etiquetas() {
        assert!(host_belongs_to("youtube.com", "youtube.com"));
        assert!(host_belongs_to("a.b.youtube.com", "youtube.com"));
        assert!(!host_belongs_to("evilyoutube.com", "youtube.com"));
        assert!(!host_belongs_to("youtube.com.evil.com", "youtube.com"));
        assert!(!host_belongs_to("com", "youtube.com"));
    }

    #[test]
    fn codifica_la_busqueda_para_que_no_pueda_alterar_la_direccion() {
        assert_eq!(percent_encode("Arctic Monkeys"), "Arctic%20Monkeys");
        assert_eq!(percent_encode("a:b/c?d=e&f#g"), "a%3Ab%2Fc%3Fd%3De%26f%23g");
        assert_eq!(percent_encode("Mägo de Oz ñ"), "M%C3%A4go%20de%20Oz%20%C3%B1");
        assert_eq!(percent_encode("AC-DC_1.0~x"), "AC-DC_1.0~x");
        assert_eq!(percent_encode("\"; calc"), "%22%3B%20calc");
    }

    #[test]
    fn elige_aplicacion_o_web_segun_lo_instalado() {
        assert_eq!(music_target(Service::Spotify, "Arctic Monkeys", true), "spotify:search:Arctic%20Monkeys");
        assert_eq!(music_target(Service::Spotify, "Arctic Monkeys", false), "https://open.spotify.com/search/Arctic%20Monkeys");
        assert_eq!(music_target(Service::YoutubeMusic, "a&b", true), "https://music.youtube.com/search?q=a%26b");
        // Una búsqueda maliciosa no puede introducir otro URI ni otra ruta.
        let sneaky = music_target(Service::Spotify, "x:track:123 spotify:playlist:evil/../", true);
        assert!(sneaky.starts_with("spotify:search:") && sneaky[15..].chars().all(|c| c.is_ascii_alphanumeric() || "%-._~".contains(c)), "{sneaky}");
    }

    /// Abre de verdad una página y una búsqueda: `cargo test -p zen-desktop -- --ignored abre_de_verdad_web`.
    #[test]
    #[ignore = "abre el navegador y Spotify"]
    fn abre_de_verdad_web_y_musica() {
        let web = OpenWebsite.execute(&json!({ "url": "https://example.com/" })).unwrap();
        assert_eq!(web.summary, "Abrí example.com en tu navegador");
        let music = SearchMusic.execute(&json!({ "query": "Arctic Monkeys" })).unwrap();
        println!("{}", music.summary);
    }

    #[test]
    fn valida_la_busqueda_de_musica() {
        assert!(SearchMusic.validate(&json!({ "query": "Arctic Monkeys" })).is_ok());
        assert!(SearchMusic.validate(&json!({ "query": "Arctic Monkeys", "service": "youtube_music" })).is_ok());
        for bad in [
            json!({ "query": "" }), json!({ "query": "   " }), json!({ "query": "a\nb" }), json!({ "query": "x".repeat(MAX_QUERY_LEN + 1) }),
            json!({ "query": "x", "service": "napster" }), json!({ "query": "x", "otro": 1 }), json!({}), json!({ "query": 5 }),
        ] {
            assert!(SearchMusic.validate(&bad).is_err(), "{bad}");
        }
        assert_eq!(SearchMusic.risk(&json!({})), RiskLevel::Safe);
        assert_eq!(SearchMusic.preview(&json!({ "query": "Arctic Monkeys" })), "Buscar «Arctic Monkeys» en Spotify");
    }
}
