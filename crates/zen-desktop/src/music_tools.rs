//! «Pon tal canción»: busca lo pedido y lo reproduce.
//!
//! - **Spotify** (aplicación de escritorio): abre la búsqueda y pulsa el botón «Reproducir» de la canción
//!   que mejor coincide, a través de la accesibilidad de Windows. Solo toca la ventana de Spotify.
//! - **YouTube / YouTube Music**: lee la página pública de resultados, elige el primer vídeo y abre su
//!   dirección (construida por nosotros a partir de un identificador validado) en el navegador.
//!
//! Después comprueba en los controles multimedia de Windows qué suena de verdad, para no afirmar nada falso.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid, optional_str, required_str};
use crate::input;
use crate::media_session::{self, Control as MediaButton, PlayState};
use crate::uia::{self, Control};

const MAX_QUERY_CHARS: usize = 100;
/// Se miran solo las primeras canciones de los resultados: Spotify ya los ordena por relevancia.
const CANDIDATES: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    Spotify,
    YoutubeMusic,
    Youtube,
}

impl Service {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "spotify" => Some(Service::Spotify),
            "youtube_music" => Some(Service::YoutubeMusic),
            "youtube" => Some(Service::Youtube),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Service::Spotify => "Spotify",
            Service::YoutubeMusic => "YouTube Music",
            Service::Youtube => "YouTube",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Spotify: leer los resultados y elegir
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    pub artists: Vec<String>,
    /// Nombre exacto del botón «Reproducir …» y cuántos botones iguales había antes (por si se repite).
    play_name: String,
    occurrence: usize,
}

/// Textos de la interfaz de Spotify según el idioma: (prefijo del botón de reproducir, prefijo de «más opciones»).
const SPOTIFY_TEXTS: &[(&str, &str)] = &[("Reproducir ", "Más opciones para "), ("Play ", "More options for ")];

/// Las filas de canciones de los resultados: un botón «Reproducir X», los enlaces de X y de sus artistas y
/// un botón «Más opciones para X». Esto último distingue una canción de una playlist o un álbum.
pub fn parse_spotify_tracks(controls: &[Control]) -> Vec<Track> {
    let mut tracks = Vec::new();
    for (index, control) in controls.iter().enumerate() {
        if control.kind != "botón" {
            continue;
        }
        for (play_prefix, more_prefix) in SPOTIFY_TEXTS {
            let Some(title) = control.name.strip_prefix(play_prefix).filter(|t| !t.is_empty()) else { continue };
            let more = format!("{more_prefix}{title}");
            let window = &controls[index + 1..(index + 9).min(controls.len())];
            let Some(end) = window.iter().position(|c| c.name == more) else { continue };
            let artists = window[..end].iter().filter(|c| c.kind == "enlace" && c.name != title).map(|c| c.name.clone()).collect();
            let occurrence = controls[..index].iter().filter(|c| c.name == control.name && c.kind == "botón").count();
            tracks.push(Track { title: title.to_string(), artists, play_name: control.name.clone(), occurrence });
            break;
        }
    }
    tracks
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Qué parte de lo pedido aparece en «título + artistas» (0.0 a 1.0), con un pequeño descuento si el título trae
/// palabras que no se pidieron («Live», «Remastered»…): ante un empate, gana la versión más parecida a lo pedido.
fn score(query: &str, track: &Track) -> f32 {
    let wanted = normalize(query);
    let words: Vec<&str> = wanted.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return 0.0;
    }
    let haystack = normalize(&format!("{} {}", track.title, track.artists.join(" ")));
    let matched = words.iter().filter(|word| haystack.contains(*word)).count() as f32 / words.len() as f32;
    let title = normalize(&track.title);
    let title_words: Vec<&str> = title.split(' ').filter(|w| !w.is_empty()).collect();
    let extra = title_words.iter().filter(|word| !words.contains(word)).count() as f32 / title_words.len().max(1) as f32;
    matched - 0.1 * extra
}

/// La canción que mejor coincide entre las primeras; si empatan, la que Spotify puso antes.
pub fn pick_track<'a>(query: &str, tracks: &'a [Track]) -> Option<&'a Track> {
    let mut best: Option<(&Track, f32)> = None;
    for track in tracks.iter().take(CANDIDATES) {
        let value = score(query, track);
        if best.is_none_or(|(_, top)| value > top) {
            best = Some((track, value));
        }
    }
    best.map(|(track, _)| track)
}

// ---------------------------------------------------------------------------------------------
// YouTube: leer los resultados
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
}

pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn watch_url(service: Service, id: &str) -> String {
    let host = if service == Service::YoutubeMusic { "music.youtube.com" } else { "www.youtube.com" };
    format!("https://{host}/watch?v={id}")
}

/// Vídeos de la página de resultados de YouTube (los datos viajan en `ytInitialData`).
pub fn parse_youtube_results(html: &str) -> Result<Vec<Video>, String> {
    const START: &str = "var ytInitialData = ";
    let start = html.find(START).ok_or("YouTube cambió el formato de su página de resultados")? + START.len();
    let end = html[start..].find(";</script>").ok_or("YouTube cambió el formato de su página de resultados")? + start;
    let data: Value = serde_json::from_str(&html[start..end]).map_err(|_| "no pude leer los resultados de YouTube")?;
    let mut videos = Vec::new();
    let mut stack = vec![&data];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                if let Some(renderer) = map.get("videoRenderer") {
                    if let Some(id) = renderer["videoId"].as_str().filter(|id| valid_video_id(id)) {
                        videos.push(Video {
                            id: id.to_string(),
                            title: renderer["title"]["runs"][0]["text"].as_str().unwrap_or("").to_string(),
                            channel: renderer["ownerText"]["runs"][0]["text"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    // La pila recorre en orden inverso y mezcla ramas: se ordena según aparece en la página.
    videos.sort_by_key(|video| html.find(&format!("\"videoId\":\"{}\"", video.id)).unwrap_or(usize::MAX));
    videos.dedup_by(|a, b| a.id == b.id);
    Ok(videos)
}

/// Palabras de versiones no oficiales: se evitan salvo que el usuario las pida.
const UNOFFICIAL_WORDS: &[&str] = &["cover", "karaoke", "slowed", "reverb", "nightcore", "8d", "sped", "instrumental", "espanol", "traducida", "subtitulado", "reaccion", "reaction", "tutorial", "piano", "guitar"];

/// Qué vídeo reproducir. Para vídeos, el primero de YouTube (ya viene ordenado por relevancia). Para música se
/// prefieren los canales oficiales («… - Topic», VEVO) y se evitan los covers, karaokes y versiones «slowed».
fn pick_video<'a>(service: Service, query: &str, videos: &'a [Video]) -> Option<&'a Video> {
    if service != Service::YoutubeMusic {
        return videos.first();
    }
    let wanted = normalize(query);
    let words: Vec<&str> = wanted.split(' ').filter(|w| !w.is_empty()).collect();
    let mut best: Option<(&Video, f32)> = None;
    for video in videos.iter().take(8) {
        let title = normalize(&video.title);
        let channel = normalize(&video.channel);
        let haystack = format!("{title} {channel}");
        let matched = if words.is_empty() { 0.0 } else { words.iter().filter(|w| haystack.contains(*w)).count() as f32 / words.len() as f32 };
        let mut value = matched;
        if video.channel.trim_end().ends_with("- Topic") {
            value += 0.3;
        } else if channel.contains("vevo") || channel.contains("official") {
            value += 0.15;
        }
        let junk = title.split(' ').filter(|w| UNOFFICIAL_WORDS.contains(w) && !words.contains(w)).count();
        value -= 0.4 * junk as f32;
        if best.is_none_or(|(_, top)| value > top) {
            best = Some((video, value));
        }
    }
    best.map(|(video, _)| video)
}

fn search_youtube(query: &str) -> Result<Vec<Video>, String> {
    let url = format!("https://www.youtube.com/results?search_query={}", crate::web_tools::percent_encode(query));
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let mut response = agent
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36")
        .header("Accept-Language", "es-ES,es;q=0.9,en;q=0.8")
        .call()
        .map_err(|error| format!("no pude buscar en YouTube: {error}"))?;
    let html = response.body_mut().read_to_string().map_err(|error| format!("no pude leer la respuesta de YouTube: {error}"))?;
    parse_youtube_results(&html)
}

// ---------------------------------------------------------------------------------------------
// Comprobar lo que suena
// ---------------------------------------------------------------------------------------------

/// ¿El título de lo que suena corresponde a lo esperado? (Cada aplicación lo adorna de forma distinta.)
fn same_title(playing: &str, expected: &str) -> bool {
    let (a, b) = (normalize(playing), normalize(expected));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let prefix: String = b.chars().take(14).collect();
    a.contains(&b) || b.contains(&a) || a.contains(&prefix)
}

/// Espera hasta `timeout` a que lo esperado esté sonando en alguna sesión multimedia (no solo la más reciente).
/// Si está en pausa tras unos segundos, pulsa play una vez en esa sesión.
fn confirm_playing(expected_title: &str, timeout: Duration) -> bool {
    let started = Instant::now();
    let mut pressed_play = false;
    while started.elapsed() < timeout {
        if let Ok(sessions) = media_session::all_playing() {
            if let Some(now) = sessions.iter().find(|s| same_title(&s.title, expected_title)) {
                if now.state == PlayState::Playing {
                    return true;
                }
                if !pressed_play && started.elapsed() > Duration::from_secs(3) && now.can_play_pause {
                    pressed_play = true;
                    let _ = media_session::control_matching(&|title| same_title(title, expected_title), MediaButton::PlayPause);
                }
            }
        }
        sleep(Duration::from_millis(500));
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Reproducir
// ---------------------------------------------------------------------------------------------

fn spotify_window() -> Option<input::WindowInfo> {
    input::visible_windows().into_iter().find(|w| w.process == "spotify")
}

fn spotify_tracks(hwnd: isize) -> Vec<Track> {
    uia::scan_controls(hwnd).map(|controls| parse_spotify_tracks(&controls)).unwrap_or_default()
}

fn play_on_spotify(query: &str) -> Result<ToolOutput, ToolError> {
    if !crate::applications::is_installed("spotify") {
        return Err(ToolError::Failed("Spotify no está instalado en este equipo. Prueba con youtube_music.".into()));
    }
    // Lo que había antes en pantalla: sirve para no pulsar resultados viejos mientras carga la búsqueda nueva.
    let before: Vec<String> = spotify_window().map(|w| spotify_tracks(w.hwnd)).unwrap_or_default().into_iter().map(|t| t.title).collect();

    let target = format!("spotify:search:{}", crate::web_tools::percent_encode(query));
    crate::web_tools::open_with_shell(&target).map_err(|error| ToolError::Failed(format!("no pude abrir Spotify: {error}")))?;

    let started = Instant::now();
    let mut window = None;
    while started.elapsed() < Duration::from_secs(45) {
        if let Some(found) = spotify_window() {
            window = Some(found);
            break;
        }
        sleep(Duration::from_millis(500));
    }
    let window = window.ok_or_else(|| ToolError::Failed("Spotify no llegó a abrirse a tiempo.".into()))?;

    // Esperar a que aparezcan las canciones de la búsqueda nueva.
    let wait_from = Instant::now();
    let mut tracks = Vec::new();
    while wait_from.elapsed() < Duration::from_secs(40) {
        sleep(Duration::from_millis(800));
        let found = spotify_tracks(window.hwnd);
        let titles: Vec<&String> = found.iter().map(|t| &t.title).collect();
        let changed = titles.iter().map(|t| t.as_str()).ne(before.iter().map(String::as_str));
        if !found.is_empty() && (changed || wait_from.elapsed() > Duration::from_secs(8)) {
            tracks = found;
            break;
        }
    }
    let track = pick_track(query, &tracks)
        .ok_or_else(|| ToolError::Failed(format!("No encontré canciones para «{query}» en Spotify (¿hay sesión iniciada?).")))?
        .clone();

    uia::activate_nth(window.hwnd, &track.play_name, track.occurrence).map_err(ToolError::Failed)?;
    let artist = track.artists.first().cloned().unwrap_or_default();
    let label = if artist.is_empty() { track.title.clone() } else { format!("{} de {artist}", track.title) };
    let confirmed = confirm_playing(&track.title, Duration::from_secs(10));
    let summary = if confirmed {
        format!("Reproduciendo «{label}» en Spotify")
    } else {
        format!("Pulsé reproducir «{label}» en Spotify, pero no pude confirmar que esté sonando")
    };
    Ok(ToolOutput { summary, data: json!({ "service": "spotify", "title": track.title, "artist": artist, "playing": confirmed }) })
}

fn play_on_youtube(service: Service, query: &str) -> Result<ToolOutput, ToolError> {
    let videos = search_youtube(query).map_err(ToolError::Failed)?;
    let video = pick_video(service, query, &videos).ok_or_else(|| ToolError::Failed(format!("No encontré nada para «{query}» en {}.", service.name())))?;
    crate::web_tools::open_with_shell(&watch_url(service, &video.id)).map_err(|error| ToolError::Failed(format!("no pude abrir el navegador: {error}")))?;
    let confirmed = confirm_playing(&video.title, Duration::from_secs(12));
    let label = if video.channel.is_empty() { video.title.clone() } else { format!("{} ({})", video.title, video.channel) };
    let summary = if confirmed {
        format!("Reproduciendo «{label}» en {}", service.name())
    } else {
        format!("Abrí «{label}» en {}; si no empieza solo, pulsa play en el navegador", service.name())
    };
    Ok(ToolOutput { summary, data: json!({ "service": service.name(), "title": video.title, "channel": video.channel, "playing": confirmed }) })
}

fn parse(args: &Value) -> Result<(String, Option<Service>), ToolError> {
    check_keys(args, &["query", "service"])?;
    let query = required_str(args, "query")?.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS || query.chars().any(char::is_control) {
        return Err(invalid(format!("«query» debe tener entre 1 y {MAX_QUERY_CHARS} caracteres, sin saltos de línea")));
    }
    let service = match optional_str(args, "service")? {
        None => None,
        Some(key) => Some(Service::from_key(key).ok_or_else(|| invalid(format!("servicio desconocido «{key}»; usa spotify, youtube_music o youtube")))?),
    };
    Ok((query, service))
}

pub struct PlayMusic;

impl PlayMusic {
    fn resolve_service(service: Option<Service>) -> Service {
        service.unwrap_or(if crate::applications::is_installed("spotify") { Service::Spotify } else { Service::YoutubeMusic })
    }
}

impl Tool for PlayMusic {
    fn name(&self) -> &'static str {
        "play_music"
    }

    fn description(&self) -> &'static str {
        "Busca una canción, artista o vídeo y lo REPRODUCE. Servicios: spotify (aplicación de escritorio), \
         youtube_music (música en el navegador) y youtube (vídeos). Úsala cuando el usuario quiera escuchar o ver \
         algo («pon…», «reproduce…», «quiero ver…»); si no indica servicio, usa Spotify. Para solo buscar sin \
         reproducir existe search_music."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Qué reproducir: canción y artista, o el título del vídeo." },
                "service": { "type": "string", "enum": ["spotify", "youtube_music", "youtube"], "description": "Dónde reproducirlo (opcional; por defecto Spotify)." }
            },
            "required": ["query"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse(args) {
            Ok((query, service)) => format!("Buscar «{query}» y reproducirlo en {}", Self::resolve_service(service).name()),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (query, service) = parse(args)?;
        match Self::resolve_service(service) {
            Service::Spotify => play_on_spotify(&query),
            other => play_on_youtube(other, &query),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(kind: &'static str, name: &str) -> Control {
        Control { name: name.into(), kind }
    }

    /// Fragmento real de lo que Windows ve en la búsqueda de Spotify.
    fn spotify_results() -> Vec<Control> {
        vec![
            c("botón", "Reproducir"), // el botón del reproductor de abajo: sin título, no es una canción
            c("botón", "Reproducir end credits"),
            c("enlace", "end credits"),
            c("botón", "Radio de Arctic Monkeys De Spotify"),
            c("botón", "Reproducir No. 1 Party Anthem"),
            c("enlace", "No. 1 Party Anthem"),
            c("enlace", "Arctic Monkeys"),
            c("botón", "Más opciones para No. 1 Party Anthem"),
            c("botón", "Agregar a playlist"),
            c("botón", "Reproducir Do I Wanna Know?"),
            c("enlace", "Do I Wanna Know?"),
            c("enlace", "Arctic Monkeys"),
            c("botón", "Más opciones para Do I Wanna Know?"),
            c("botón", "Agregar a playlist"),
            c("botón", "Reproducir 505"),
            c("enlace", "505"),
            c("enlace", "Arctic Monkeys"),
            c("botón", "Más opciones para 505"),
        ]
    }

    #[test]
    fn reconoce_las_filas_de_canciones_y_descarta_playlists() {
        let tracks = parse_spotify_tracks(&spotify_results());
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["No. 1 Party Anthem", "Do I Wanna Know?", "505"], "«end credits» es una playlist y no tiene «Más opciones»");
        assert_eq!(tracks[1].artists, ["Arctic Monkeys"]);
        assert_eq!(tracks[1].play_name, "Reproducir Do I Wanna Know?");
    }

    #[test]
    fn tambien_entiende_spotify_en_ingles_y_cuenta_repeticiones() {
        let controls = vec![
            c("botón", "Play Home"), c("enlace", "Home"), c("enlace", "Band"), c("botón", "More options for Home"),
            c("botón", "Play Home"), c("enlace", "Home"), c("enlace", "Other Band"), c("botón", "More options for Home"),
        ];
        let tracks = parse_spotify_tracks(&controls);
        assert_eq!(tracks.len(), 2);
        assert_eq!((tracks[0].occurrence, tracks[1].occurrence), (0, 1), "dos botones iguales se distinguen por orden");
        assert_eq!(tracks[1].artists, ["Other Band"]);
    }

    #[test]
    fn elige_la_cancion_que_mejor_coincide() {
        let tracks = parse_spotify_tracks(&spotify_results());
        assert_eq!(pick_track("do i wanna know", &tracks).unwrap().title, "Do I Wanna Know?");
        assert_eq!(pick_track("505 arctic monkeys", &tracks).unwrap().title, "505");
        assert_eq!(pick_track("arctic monkeys", &tracks).unwrap().title, "No. 1 Party Anthem", "con empate gana la primera de Spotify");
        let versions = vec![
            Track { title: "505 - Live".into(), artists: vec!["Arctic Monkeys".into()], play_name: "Reproducir 505 - Live".into(), occurrence: 0 },
            Track { title: "505".into(), artists: vec!["Arctic Monkeys".into()], play_name: "Reproducir 505".into(), occurrence: 0 },
        ];
        assert_eq!(pick_track("505 arctic monkeys", &versions).unwrap().title, "505", "prefiere la versión sin adornos que no se pidieron");
        assert_eq!(pick_track("505 live arctic monkeys", &versions).unwrap().title, "505 - Live", "pero si piden la versión en vivo, esa");
        assert_eq!(pick_track("algo que no sale", &tracks).unwrap().title, "No. 1 Party Anthem", "sin coincidencias se fía del orden de Spotify");
        assert!(pick_track("x", &[]).is_none());
        assert_eq!(pick_track("DÓ I WÁNNA KNOW", &tracks).unwrap().title, "Do I Wanna Know?", "sin mayúsculas ni tildes");
    }

    #[test]
    fn valida_los_identificadores_de_video() {
        assert!(valid_video_id("bpOSxM0rNPM") && valid_video_id("a-b_c1D2e3F"));
        for bad in ["", "corto", "demasiado_largo_", "bpOSxM0rNP!", "bpOSxM0rNP/", "bpOSxM0r NP", "bpOSxM0rNPM&x=1"] {
            assert!(!valid_video_id(bad), "{bad:?}");
        }
        assert_eq!(watch_url(Service::Youtube, "bpOSxM0rNPM"), "https://www.youtube.com/watch?v=bpOSxM0rNPM");
        assert_eq!(watch_url(Service::YoutubeMusic, "bpOSxM0rNPM"), "https://music.youtube.com/watch?v=bpOSxM0rNPM");
    }

    #[test]
    fn lee_los_resultados_de_youtube_en_orden_y_descarta_ids_raros() {
        let data = json!({ "contents": [
            { "adSlotRenderer": { "videoId": "AAAAAAAAAAA" } },
            { "videoRenderer": { "videoId": "bpOSxM0rNPM", "title": { "runs": [{ "text": "Do I Wanna Know? (Official)" }] }, "ownerText": { "runs": [{ "text": "Arctic Monkeys" }] } } },
            { "videoRenderer": { "videoId": "mal id!!!!!", "title": { "runs": [{ "text": "Roto" }] } } },
            { "videoRenderer": { "videoId": "lPe5zInLju4", "title": { "runs": [{ "text": "Subtitulado" }] } } },
        ]});
        let html = format!("<html><script>var ytInitialData = {data};</script></html>");
        let videos = parse_youtube_results(&html).unwrap();
        assert_eq!(videos.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["bpOSxM0rNPM", "lPe5zInLju4"]);
        assert_eq!(videos[0].title, "Do I Wanna Know? (Official)");
        assert_eq!(videos[0].channel, "Arctic Monkeys");
        assert!(parse_youtube_results("<html>sin datos</html>").is_err());
        assert!(parse_youtube_results("var ytInitialData = {roto;</script>").is_err());
    }

    #[test]
    fn para_musica_prefiere_el_canal_oficial_y_evita_covers() {
        let video = |id: &str, title: &str, channel: &str| Video { id: id.into(), title: title.into(), channel: channel.into() };
        let results = vec![
            video("AAAAAAAAAAA", "505 - arctic monkeys // español", "phtogrfy"),
            video("BBBBBBBBBBB", "505 (cover)", "Algún Chico"),
            video("CCCCCCCCCCC", "505", "Arctic Monkeys - Topic"),
            video("DDDDDDDDDDD", "Arctic Monkeys - 505 (Official Video)", "Arctic Monkeys"),
        ];
        assert_eq!(pick_video(Service::YoutubeMusic, "505 arctic monkeys", &results).unwrap().id, "CCCCCCCCCCC");
        assert_eq!(pick_video(Service::YoutubeMusic, "505 cover", &results).unwrap().id, "BBBBBBBBBBB", "si piden un cover, se respeta");
        assert_eq!(pick_video(Service::Youtube, "505 arctic monkeys", &results).unwrap().id, "AAAAAAAAAAA", "los vídeos respetan el orden de YouTube");
        assert!(pick_video(Service::YoutubeMusic, "x", &[]).is_none());
    }

    #[test]
    fn compara_titulos_aunque_cada_app_los_adorne() {
        assert!(same_title("Do I Wanna Know?", "Do I Wanna Know?"));
        assert!(same_title("Arctic Monkeys - Do I Wanna Know? (Official Video) - YouTube", "Arctic Monkeys - Do I Wanna Know? (Official Video)"));
        assert!(same_title("do i wanna know", "Do I Wanna Know?"));
        assert!(!same_title("505", "Do I Wanna Know?"));
        assert!(!same_title("", "x") && !same_title("x", ""));
    }

    #[test]
    fn valida_la_peticion() {
        assert_eq!(parse(&json!({ "query": "  Arctic   Monkeys " })).unwrap(), ("Arctic Monkeys".to_string(), None));
        assert_eq!(parse(&json!({ "query": "x", "service": "youtube" })).unwrap().1, Some(Service::Youtube));
        for bad in [json!({}), json!({ "query": "" }), json!({ "query": "  " }), json!({ "query": "a\u{7}b" }),json!({ "query": "x".repeat(101) }), json!({ "query": "x", "service": "tidal" }), json!({ "query": "x", "otro": 1 }), json!({ "query": 5 })] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
        assert_eq!(PlayMusic.risk(&json!({})), RiskLevel::Safe);
    }

    /// Prueba real: pone música de verdad. `cargo test -p zen-desktop -- --ignored --nocapture reproduce_de_verdad`
    #[test]
    #[ignore = "reproduce música y abre aplicaciones"]
    fn reproduce_de_verdad() {
        let service = std::env::var("ZEN_TEST_SERVICE").unwrap_or_else(|_| "spotify".into());
        let query = std::env::var("ZEN_TEST_QUERY").unwrap_or_else(|_| "Do I Wanna Know Arctic Monkeys".into());
        let output = PlayMusic.execute(&json!({ "query": query, "service": service })).expect("debe reproducir");
        println!("{} {}", output.summary, output.data);
        assert_eq!(output.data["playing"], true, "{}", output.summary);
    }
}
