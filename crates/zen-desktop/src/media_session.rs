//! Lo que suena ahora mismo en Windows (Spotify, un vídeo en el navegador, VLC…) y sus botones.
//! Usa «System Media Transport Controls», la misma API que alimenta los controles multimedia del sistema,
//! así que no depende de ninguna aplicación ni de ningún servicio en concreto.

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{DataReader, IInputStream, IRandomAccessStreamReference};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::core::Interface;

/// Una carátula mayor que esto no se envía a la interfaz.
const MAX_THUMBNAIL_BYTES: u32 = 800_000;
/// Segundos entre 1601 (época de las fechas de Windows) y 1970.
const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

impl PlayState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlayState::Playing => "playing",
            PlayState::Paused => "paused",
            PlayState::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    /// Aplicación que reproduce, en un nombre legible («Spotify», «Chrome»…).
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub state: PlayState,
    pub position_secs: f64,
    /// 0 si la aplicación no informa de la duración (por ejemplo, una emisión en directo).
    pub duration_secs: f64,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
    /// Identifica la carátula: cambia cuando cambia la pista, para no reenviar la imagen cada segundo.
    pub artwork_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
}

impl Control {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "play_pause" => Some(Control::PlayPause),
            "next" => Some(Control::Next),
            "previous" => Some(Control::Previous),
            _ => None,
        }
    }
}

thread_local! {
    static INITIALIZED: Cell<bool> = const { Cell::new(false) };
}

/// Las API de Windows Runtime necesitan que el hilo esté inicializado. Es seguro llamarla varias veces.
fn ensure_initialized() {
    INITIALIZED.with(|done| {
        if !done.get() {
            // SAFETY: inicialización estándar del hilo actual; un error (p. ej. «ya inicializado») no impide seguir.
            let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
            done.set(true);
        }
    });
}

fn current_session() -> Result<Option<Session>, String> {
    ensure_initialized();
    let manager = Manager::RequestAsync().and_then(|op| op.join()).map_err(|error| format!("no se pudo consultar los controles multimedia: {error}"))?;
    // Sin nada reproduciéndose, Windows devuelve «sin sesión» (puede llegar como error): no es un fallo.
    Ok(manager.GetCurrentSession().ok())
}

/// «Spotify.exe» → «Spotify»; «SpotifyAB.SpotifyMusic_…!Spotify» → «Spotify»; «Microsoft.ZuneMusic_…!…» → «Reproductor multimedia».
fn friendly_app_name(app_id: &str) -> String {
    let lower = app_id.to_lowercase();
    for (needle, name) in [
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("zunemusic", "Reproductor multimedia"),
        ("zunevideo", "Películas y TV"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("youtube", "YouTube"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
    ] {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    let last = app_id.rsplit(['!', '\\', '/']).next().unwrap_or(app_id);
    let trimmed = last.strip_suffix(".exe").or_else(|| last.strip_suffix(".EXE")).unwrap_or(last);
    let name: String = trimmed.chars().filter(|c| !c.is_control()).take(40).collect();
    if name.is_empty() {
        "Reproductor".to_string()
    } else {
        name
    }
}

fn status_to_state(status: Status) -> PlayState {
    match status {
        Status::Playing => PlayState::Playing,
        Status::Paused => PlayState::Paused,
        _ => PlayState::Stopped,
    }
}

/// Posición actual: la que informó la aplicación más el tiempo transcurrido desde que la informó (si está sonando).
fn live_position(reported_secs: f64, last_updated_filetime: i64, now_filetime: i64, playing: bool, duration_secs: f64) -> f64 {
    let mut position = reported_secs.max(0.0);
    if playing && last_updated_filetime > 0 {
        let elapsed = (now_filetime - last_updated_filetime) as f64 / 10_000_000.0;
        // Un desfase absurdo (reloj cambiado, dato antiguo) no debe mover la barra.
        if (0.0..=6.0 * 3_600.0).contains(&elapsed) {
            position += elapsed;
        }
    }
    if duration_secs > 0.0 {
        position = position.min(duration_secs);
    }
    position
}

fn now_filetime() -> i64 {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    (unix.as_secs() as i64 + WINDOWS_TO_UNIX_SECS) * 10_000_000 + (unix.subsec_nanos() / 100) as i64
}

/// Lo que suena ahora (la sesión más reciente), o None si no hay ninguna sesión multimedia.
pub fn now_playing() -> Result<Option<NowPlaying>, String> {
    let Some(session) = current_session()? else { return Ok(None) };
    describe(&session).map(Some)
}

fn all_sessions() -> Result<Vec<Session>, String> {
    ensure_initialized();
    let manager = Manager::RequestAsync().and_then(|op| op.join()).map_err(|error| format!("no se pudo consultar los controles multimedia: {error}"))?;
    let list = manager.GetSessions().map_err(|error| format!("no se pudo listar las sesiones multimedia: {error}"))?;
    Ok((0..list.Size().unwrap_or(0)).filter_map(|index| list.GetAt(index).ok()).collect())
}

/// Todas las sesiones multimedia abiertas (Spotify, pestañas del navegador, VLC…), no solo la más reciente.
pub fn all_playing() -> Result<Vec<NowPlaying>, String> {
    Ok(all_sessions()?.iter().filter_map(|session| describe(session).ok()).collect())
}

/// Pulsa un botón en la sesión cuyo título cumple la condición (no tiene por qué ser la más reciente).
pub fn control_matching(matches: &dyn Fn(&str) -> bool, control: Control) -> Result<bool, String> {
    for session in all_sessions()? {
        let Ok(info) = describe(&session) else { continue };
        if matches(&info.title) {
            let result = match control {
                Control::PlayPause => session.TryTogglePlayPauseAsync(),
                Control::Next => session.TrySkipNextAsync(),
                Control::Previous => session.TrySkipPreviousAsync(),
            };
            return result.and_then(|op| op.join()).map_err(|error| format!("no se pudo enviar la orden: {error}"));
        }
    }
    Err("No encontré esa sesión multimedia.".into())
}

/// Datos de una sesión multimedia concreta.
fn describe(session: &Session) -> Result<NowPlaying, String> {
    let fail = |error: windows::core::Error| format!("no se pudo leer lo que suena: {error}");

    let app = session.SourceAppUserModelId().map(|id| id.to_string()).unwrap_or_default();
    let properties = session.TryGetMediaPropertiesAsync().and_then(|op| op.join()).map_err(fail)?;
    let (title, artist, album) = (
        properties.Title().map(|s| s.to_string()).unwrap_or_default(),
        properties.Artist().map(|s| s.to_string()).unwrap_or_default(),
        properties.AlbumTitle().map(|s| s.to_string()).unwrap_or_default(),
    );

    let playback = session.GetPlaybackInfo().map_err(fail)?;
    let state = status_to_state(playback.PlaybackStatus().unwrap_or(Status::Stopped));
    let controls = playback.Controls().map_err(fail)?;
    let can = |read: windows::core::Result<bool>| read.unwrap_or(false);

    let timeline = session.GetTimelineProperties().map_err(fail)?;
    let secs = |ticks: windows::core::Result<windows::Foundation::TimeSpan>| ticks.map(|t| t.Duration as f64 / 10_000_000.0).unwrap_or(0.0);
    let (start, end) = (secs(timeline.StartTime()), secs(timeline.EndTime()));
    let duration = (end - start).max(0.0);
    let reported = (secs(timeline.Position()) - start).max(0.0);
    let last_updated = timeline.LastUpdatedTime().map(|d| d.UniversalTime).unwrap_or(0);
    let position = live_position(reported, last_updated, now_filetime(), state == PlayState::Playing, duration);

    Ok(NowPlaying {
        app: friendly_app_name(&app),
        artwork_key: format!("{app}|{title}|{artist}|{album}"),
        title,
        artist,
        album,
        state,
        position_secs: position,
        duration_secs: duration,
        can_play_pause: can(controls.IsPlayPauseToggleEnabled()) || can(controls.IsPlayEnabled()) || can(controls.IsPauseEnabled()),
        can_next: can(controls.IsNextEnabled()),
        can_previous: can(controls.IsPreviousEnabled()),
    })
}

/// Carátula de lo que suena, como `data:` URI lista para un `<img>`. `None` si no hay o es demasiado grande.
pub fn artwork() -> Option<String> {
    ensure_initialized();
    let session = current_session().ok()??;
    let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let reference: IRandomAccessStreamReference = properties.Thumbnail().ok()?;
    let stream = reference.OpenReadAsync().ok()?.join().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok()?;
    if size == 0 || size > MAX_THUMBNAIL_BYTES {
        return None;
    }
    let mime = stream.ContentType().ok()?.to_string();
    if !matches!(mime.as_str(), "image/jpeg" | "image/png" | "image/bmp" | "image/gif" | "image/webp") {
        return None;
    }
    let input: IInputStream = stream.cast().ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(format!("data:{mime};base64,{}", base64(&bytes)))
}

/// Pulsa un botón del reproductor activo. Devuelve si la aplicación lo aceptó.
pub fn control(control: Control) -> Result<bool, String> {
    let session = current_session()?.ok_or("No hay nada reproduciéndose.")?;
    let result = match control {
        Control::PlayPause => session.TryTogglePlayPauseAsync(),
        Control::Next => session.TrySkipNextAsync(),
        Control::Previous => session.TrySkipPreviousAsync(),
    };
    result.and_then(|op| op.join()).map_err(|error| format!("no se pudo enviar la orden: {error}"))
}

/// Base64 estándar (sin dependencias: solo se usa para las carátulas).
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_coincide_con_los_vectores_del_estandar() {
        for (input, expected) in [
            ("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(base64(&[0xFF, 0xFE, 0xFD, 0x00, 0x01]), "//79AAE=");
        assert_eq!(base64(&(0..=255).collect::<Vec<u8>>()).len(), 344);
    }

    #[test]
    fn nombres_legibles_de_las_aplicaciones() {
        for (id, expected) in [
            ("Spotify.exe", "Spotify"),
            ("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "Spotify"),
            ("MSEdge", "Edge"),
            ("Chrome", "Chrome"),
            ("firefox.exe", "Firefox"),
            ("vlc.exe", "VLC"),
            ("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic", "Reproductor multimedia"),
            ("AlgunaApp.exe", "AlgunaApp"),
            ("C:\\Programas\\Mi Reproductor\\mireproductor.exe", "mireproductor"),
            ("Paquete_xyz!App", "App"),
            ("", "Reproductor"),
            ("\u{7}\u{8}", "\u{7}\u{8}".trim_matches(|c: char| c.is_control())),
        ] {
            let name = friendly_app_name(id);
            if expected.is_empty() {
                assert_eq!(name, "Reproductor");
            } else {
                assert_eq!(name, expected, "{id:?}");
            }
        }
    }

    #[test]
    fn la_posicion_avanza_solo_mientras_suena_y_no_pasa_del_final() {
        let now = 1_000_000_000_000_i64;
        let two_secs_ago = now - 20_000_000;
        assert_eq!(live_position(10.0, two_secs_ago, now, true, 200.0), 12.0);
        assert_eq!(live_position(10.0, two_secs_ago, now, false, 200.0), 10.0, "en pausa no avanza");
        assert_eq!(live_position(199.0, two_secs_ago, now, true, 200.0), 200.0, "no pasa de la duración");
        assert_eq!(live_position(10.0, two_secs_ago, now, true, 0.0), 12.0, "sin duración conocida sigue contando");
        assert_eq!(live_position(10.0, 0, now, true, 200.0), 10.0, "sin marca de tiempo no se inventa nada");
        assert_eq!(live_position(10.0, now + 50_000_000, now, true, 200.0), 10.0, "una marca en el futuro se ignora");
        assert_eq!(live_position(10.0, now - 7 * 3_600 * 10_000_000, now, true, 0.0), 10.0, "un dato antiquísimo se ignora");
        assert_eq!(live_position(-5.0, 0, now, false, 100.0), 0.0);
    }

    #[test]
    fn las_ordenes_se_reconocen_por_su_nombre() {
        assert_eq!(Control::from_key("play_pause"), Some(Control::PlayPause));
        assert_eq!(Control::from_key("next"), Some(Control::Next));
        assert_eq!(Control::from_key("previous"), Some(Control::Previous));
        for bad in ["", "stop", "Next", "volume_up", "play_pause; calc"] {
            assert_eq!(Control::from_key(bad), None, "{bad:?}");
        }
        assert_eq!(PlayState::Playing.as_str(), "playing");
    }

    #[test]
    fn el_reloj_de_windows_es_coherente() {
        // 2020-01-01 en FILETIME: 132_223_104_000_000_000.
        assert!(now_filetime() > 132_223_104_000_000_000);
    }

    /// Pausa y reanuda la sesión actual (¡actúa sobre lo que esté sonando!): solo con un reproductor de prueba.
    #[test]
    #[ignore = "actúa sobre el reproductor real"]
    fn alterna_pausa_en_la_sesion_actual() {
        let before = now_playing().unwrap().expect("debe haber sesión").state;
        assert!(control(Control::PlayPause).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(700));
        let during = now_playing().unwrap().unwrap().state;
        assert!(control(Control::PlayPause).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(700));
        let after = now_playing().unwrap().unwrap().state;
        println!("{before:?} -> {during:?} -> {after:?}");
        assert_ne!(before, during);
        assert_eq!(before, after);
    }

    /// Pone en pausa todo lo que suene (para dejar el equipo en silencio tras las pruebas reales).
    #[test]
    #[ignore = "pausa la música real"]
    fn pausa_todo_lo_que_suena() {
        for session in all_playing().expect("la consulta debe funcionar") {
            if session.state == PlayState::Playing {
                let title = session.title.clone();
                println!("pausando: {} | {title}", session.app);
                control_matching(&|t| t == title, Control::PlayPause).expect("pausar");
            }
        }
    }

    /// Todas las sesiones multimedia abiertas: cargo test -p zen-desktop -- --ignored --nocapture sesiones_abiertas
    #[test]
    #[ignore = "depende de lo que esté sonando"]
    fn sesiones_abiertas() {
        for session in all_playing().expect("la consulta debe funcionar") {
            println!("{} | {} | {:?}", session.app, session.title, session.state);
        }
    }

    /// Lo que suena ahora en este equipo: `cargo test -p zen-desktop -- --ignored --nocapture suena_ahora`.
    #[test]
    #[ignore = "depende de lo que esté sonando"]
    fn suena_ahora_muestra_la_sesion_actual() {
        match now_playing().expect("la consulta debe funcionar") {
            Some(now) => {
                println!("{now:#?}");
                let art = artwork();
                println!("carátula: {}", art.as_ref().map(|a| format!("{} caracteres ({})", a.len(), &a[..a.find(',').unwrap_or(20)])).unwrap_or("ninguna".into()));
            }
            None => println!("no hay ninguna sesión multimedia"),
        }
    }
}
