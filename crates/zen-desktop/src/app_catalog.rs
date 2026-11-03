//! Lista de aplicaciones que la IA puede abrir. Es una lista blanca que decide el usuario: las apps
//! conocidas vienen activadas (y se pueden desactivar) y el resto de las del menú Inicio se activan una
//! a una. La IA solo puede elegir una clave de esta lista, nunca una ruta ni un comando.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::{json, Value};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::applications::{entry, launch, resolve, Target, APPS};
use crate::args::{check_keys, invalid, required_str};
use crate::powershell::{run_fixed, ScriptError};

pub const MAX_CUSTOM_APPS: usize = 100;
const MAX_NAME_CHARS: usize = 80;
const MAX_ID_CHARS: usize = 400;
const MAX_KEY_CHARS: usize = 40;
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(25);

/// Una aplicación del menú Inicio que el usuario permitió abrir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomApp {
    pub key: String,
    pub name: String,
    /// Identificador de aplicación de Windows (el que usa `shell:AppsFolder`).
    pub app_id: String,
}

/// Una aplicación del menú Inicio, tal como la lista Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredApp {
    pub name: String,
    pub app_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinStatus {
    pub key: &'static str,
    pub name: &'static str,
    pub enabled: bool,
    pub installed: bool,
}

#[derive(Default)]
struct State {
    disabled: BTreeSet<String>,
    custom: Vec<CustomApp>,
    discovered: Option<Vec<DiscoveredApp>>,
}

pub struct AppCatalog {
    state: RwLock<State>,
}

impl Default for AppCatalog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// Reglas puras
// ---------------------------------------------------------------------------------------------

/// Herramientas con las que se podría ejecutar código o cambiar el sistema: nunca se ofrecen a la IA,
/// aunque aparezcan en el menú Inicio.
const BLOCKED: &[&str] = &[
    "powershell", "pwsh", "cmd.exe", "command prompt", "símbolo del sistema", "simbolo del sistema", "windows terminal",
    "windowsterminal", "terminal", "wt.exe", "wsl", "bash", "ubuntu", "debian", "kali", "regedit", "editor del registro",
    "registry editor", "mmc.exe", "management console", "consola de administración", "gpedit", "directiva de grupo",
    "wscript", "cscript", "mshta", "rundll32", "msiexec",
];

/// Más herramientas de administración: consolas `.msc` (Servicios, Visor de eventos, Programador de
/// tareas, Administración de equipos…) y el cuadro «Ejecutar», desde el que se puede lanzar cualquier cosa.
const BLOCKED_ADMIN: &[&str] = &[
    ".msc", "rundialog", "task scheduler", "programador de tareas", "computer management", "administración de equipos",
    "event viewer", "visor de eventos", "component services", "servicios de componentes",
];
/// Nombres que solo se bloquean si son exactamente ese (para no pillar «Servicios de…» de otras cosas).
const BLOCKED_EXACT_NAMES: &[&str] = &["services", "servicios", "run", "ejecutar"];

pub fn is_blocked(name: &str, app_id: &str) -> bool {
    let name = name.to_lowercase();
    let app_id = app_id.to_lowercase();
    BLOCKED.iter().chain(BLOCKED_ADMIN).any(|word| name.contains(word) || app_id.contains(word)) || BLOCKED_EXACT_NAMES.contains(&name.as_str())
}

/// Qué es en realidad una entrada del menú Inicio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Una aplicación que se abre con `shell:AppsFolder`.
    App,
    /// Un juego de Steam (`steam://rungameid/<número>`).
    SteamGame(u64),
    /// Un documento o una página web que Windows muestra como acceso del menú Inicio; no es una aplicación.
    NotAnApp,
}

const DOCUMENT_EXTENSIONS: &[&str] = &[".pdf", ".txt", ".chm", ".url", ".html", ".htm", ".rtf", ".md", ".doc", ".docx", ".xls", ".xlsx", ".log"];

fn classify(app_id: &str) -> Kind {
    let lower = app_id.to_lowercase();
    if let Some(number) = lower.strip_prefix("steam://rungameid/") {
        return match number.parse::<u64>() {
            Ok(id) if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) => Kind::SteamGame(id),
            _ => Kind::NotAnApp,
        };
    }
    if lower.contains("://") || DOCUMENT_EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return Kind::NotAnApp;
    }
    Kind::App
}

/// ¿Se le puede ofrecer esta entrada al usuario para que la active?
fn is_selectable(app: &DiscoveredApp) -> bool {
    classify(&app.app_id) != Kind::NotAnApp && !is_blocked(&app.name, &app.app_id)
}

fn clean_text(text: &str, max_chars: usize) -> bool {
    let count = text.chars().count();
    count > 0 && count <= max_chars && text.trim() == text && !text.chars().any(char::is_control)
}

fn valid_key(key: &str) -> bool {
    (1..=MAX_KEY_CHARS).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// «Docker Desktop» → «docker_desktop».
fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.to_lowercase().chars() {
        let mapped = match ch {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c if c.is_ascii_alphanumeric() => c,
            _ => '_',
        };
        if mapped != '_' || !out.ends_with('_') {
            out.push(mapped);
        }
    }
    let trimmed: String = out.trim_matches('_').chars().take(MAX_KEY_CHARS - 3).collect();
    if trimmed.is_empty() {
        "app".into()
    } else {
        trimmed
    }
}

/// Una clave libre a partir del nombre (`slug`, `slug_2`, `slug_3`…).
fn unique_key(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let base = slug(name);
    let mut key = base.clone();
    let mut n = 2;
    while taken(&key) {
        key = format!("{base}_{n}");
        n += 1;
    }
    key
}

/// Interpreta la salida de `Get-StartApps | ConvertTo-Json`: un objeto si solo hay una, una lista si hay varias.
fn parse_start_apps(text: &str) -> Result<Vec<DiscoveredApp>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(text).map_err(|error| format!("respuesta inesperada del menú Inicio: {error}"))?;
    let items = match value {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        _ => return Err("respuesta inesperada del menú Inicio".into()),
    };
    let mut apps: Vec<DiscoveredApp> = Vec::new();
    for item in items {
        let (Some(name), Some(app_id)) = (item["Name"].as_str(), item["AppID"].as_str()) else { continue };
        let (name, app_id) = (name.trim(), app_id.trim());
        if !clean_text(name, MAX_NAME_CHARS) || !clean_text(app_id, MAX_ID_CHARS) || apps.iter().any(|a| a.app_id == app_id) {
            continue;
        }
        apps.push(DiscoveredApp { name: name.to_string(), app_id: app_id.to_string() });
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

// ---------------------------------------------------------------------------------------------
// Catálogo
// ---------------------------------------------------------------------------------------------

impl AppCatalog {
    pub fn new() -> Self {
        Self { state: RwLock::new(State::default()) }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().expect("catálogo envenenado")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().expect("catálogo envenenado")
    }

    /// Carga lo guardado en los ajustes. Todo se vuelve a comprobar: el archivo lo puede haber editado cualquiera.
    pub fn configure(&self, disabled: impl IntoIterator<Item = String>, custom: Vec<CustomApp>) {
        let mut state = self.write();
        state.disabled = disabled.into_iter().filter(|key| entry(key).is_some()).collect();
        state.custom.clear();
        for app in custom {
            let valid = valid_key(&app.key)
                && entry(&app.key).is_none()
                && clean_text(&app.name, MAX_NAME_CHARS)
                && clean_text(&app.app_id, MAX_ID_CHARS)
                && !is_blocked(&app.name, &app.app_id)
                && !state.custom.iter().any(|c| c.key == app.key || c.app_id == app.app_id)
                && state.custom.len() < MAX_CUSTOM_APPS;
            if valid {
                state.custom.push(app);
            }
        }
    }

    pub fn disabled(&self) -> BTreeSet<String> {
        self.read().disabled.clone()
    }

    pub fn custom(&self) -> Vec<CustomApp> {
        self.read().custom.clone()
    }

    pub fn discovered(&self) -> Option<Vec<DiscoveredApp>> {
        self.read().discovered.clone()
    }

    /// Lo que se le ofrece al usuario (apps y juegos, sin documentos ni enlaces) y cuántas herramientas
    /// del sistema se ocultaron por seguridad. `None` si aún no se leyó el menú Inicio.
    pub fn selectable(&self) -> Option<(Vec<DiscoveredApp>, usize)> {
        let (discovered, enabled) = {
            let state = self.read();
            let enabled: BTreeSet<String> = state.custom.iter().map(|app| app.app_id.clone()).collect();
            (state.discovered.clone()?, enabled)
        };
        let blocked = discovered.iter().filter(|app| classify(&app.app_id) != Kind::NotAnApp && is_blocked(&app.name, &app.app_id)).count();
        // Las que ya son una app conocida (mismo nombre) no se repiten aquí, salvo que ya se hubieran añadido.
        let duplicates_known = |app: &DiscoveredApp| APPS.iter().any(|known| known.display.eq_ignore_ascii_case(&app.name)) && !enabled.contains(&app.app_id);
        Some((discovered.into_iter().filter(|app| is_selectable(app) && !duplicates_known(app)).collect(), blocked))
    }

    fn keys_of(state: &State) -> Vec<String> {
        APPS.iter()
            .filter(|app| !state.disabled.contains(app.key))
            .map(|app| app.key.to_string())
            .chain(state.custom.iter().map(|app| app.key.clone()))
            .collect()
    }

    /// Claves que la IA puede usar ahora mismo.
    pub fn enabled_keys(&self) -> Vec<String> {
        Self::keys_of(&self.read())
    }

    /// Estado de las aplicaciones conocidas (si están activadas y si están instaladas en este equipo).
    pub fn builtin_status(&self) -> Vec<BuiltinStatus> {
        let disabled = self.disabled();
        APPS.iter()
            .map(|app| BuiltinStatus {
                key: app.key,
                name: app.display,
                enabled: !disabled.contains(app.key),
                installed: resolve(app).is_some(),
            })
            .collect()
    }

    /// Vuelve a leer el menú Inicio. Tarda uno o dos segundos (arranca PowerShell).
    pub fn refresh(&self) -> Result<Vec<DiscoveredApp>, String> {
        let text = run_fixed("Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress", DISCOVERY_TIMEOUT).map_err(|error| match error {
            ScriptError::Spawn(reason) => format!("no se pudo consultar el menú Inicio: {reason}"),
            ScriptError::Timeout => "la consulta del menú Inicio tardó demasiado".to_string(),
            ScriptError::Failed(reason) => format!("no se pudo consultar el menú Inicio: {reason}"),
        })?;
        let apps = parse_start_apps(&text)?;
        self.write().discovered = Some(apps.clone());
        Ok(apps)
    }

    pub fn set_builtin_enabled(&self, key: &str, enabled: bool) -> Result<(), String> {
        let app = entry(key).ok_or_else(|| format!("Aplicación desconocida: {key}"))?;
        let mut state = self.write();
        if enabled {
            state.disabled.remove(app.key);
        } else {
            state.disabled.insert(app.key.to_string());
        }
        Ok(())
    }

    /// Activa o desactiva una aplicación del menú Inicio. Solo se puede activar lo que Windows acaba de
    /// listar (no se acepta un identificador inventado) y nunca una herramienta de sistema.
    pub fn set_custom_enabled(&self, app_id: &str, enabled: bool) -> Result<(), String> {
        let mut state = self.write();
        if !enabled {
            state.custom.retain(|app| app.app_id != app_id);
            return Ok(());
        }
        if state.custom.iter().any(|app| app.app_id == app_id) {
            return Ok(());
        }
        let discovered = state
            .discovered
            .as_ref()
            .ok_or("Primero actualiza la lista de aplicaciones del menú Inicio.")?
            .iter()
            .find(|app| app.app_id == app_id)
            .cloned()
            .ok_or("Esa aplicación no aparece en el menú Inicio.")?;
        if is_blocked(&discovered.name, &discovered.app_id) {
            return Err(format!("«{}» es una herramienta del sistema y no se puede abrir desde el asistente.", discovered.name));
        }
        if classify(&discovered.app_id) == Kind::NotAnApp {
            return Err(format!("«{}» es un documento o una página web, no una aplicación.", discovered.name));
        }
        if state.custom.len() >= MAX_CUSTOM_APPS {
            return Err(format!("Ya hay {MAX_CUSTOM_APPS} aplicaciones añadidas; quita alguna antes."));
        }
        let key = unique_key(&discovered.name, &|candidate| entry(candidate).is_some() || state.custom.iter().any(|app| app.key == candidate));
        state.custom.push(CustomApp { key, name: discovered.name, app_id: discovered.app_id });
        Ok(())
    }

    #[cfg(test)]
    fn set_discovered(&self, apps: Vec<DiscoveredApp>) {
        self.write().discovered = Some(apps);
    }
}

// ---------------------------------------------------------------------------------------------
// Herramienta
// ---------------------------------------------------------------------------------------------

pub struct OpenApplication {
    catalog: Arc<AppCatalog>,
}

impl OpenApplication {
    pub fn new(catalog: Arc<AppCatalog>) -> Self {
        Self { catalog }
    }

    fn selected(&self, args: &Value) -> Result<(String, Target), ToolError> {
        check_keys(args, &["application"])?;
        let key = required_str(args, "application")?;
        let state = self.catalog.read();

        if let Some(app) = entry(key) {
            if state.disabled.contains(app.key) {
                return Err(ToolError::NotAllowed(format!("«{}» está desactivada en los ajustes", app.display)));
            }
            let target = resolve(app).ok_or_else(|| {
                ToolError::Failed(format!(
                    "No encontré {} instalada en este equipo (busqué en Microsoft Store, el registro, rutas habituales y el menú Inicio).",
                    app.display
                ))
            })?;
            return Ok((app.display.to_string(), target));
        }
        if let Some(app) = state.custom.iter().find(|app| app.key.eq_ignore_ascii_case(key)) {
            let gone = state.discovered.as_ref().is_some_and(|list| !list.iter().any(|d| d.app_id == app.app_id));
            if gone {
                return Err(ToolError::Failed(format!("«{}» ya no aparece en el menú Inicio; quizá se desinstaló.", app.name)));
            }
            let target = match classify(&app.app_id) {
                Kind::App => Target::AppsFolder(app.app_id.clone()),
                // La dirección la construye el programa a partir de un número: nunca se pasa texto ajeno.
                Kind::SteamGame(id) => Target::Uri(format!("steam://rungameid/{id}")),
                Kind::NotAnApp => return Err(ToolError::NotAllowed(format!("«{}» no es una aplicación", app.name))),
            };
            return Ok((app.name.clone(), target));
        }
        Err(invalid(format!("aplicación no permitida «{key}»; opciones: {}", AppCatalog::keys_of(&state).join(", "))))
    }
}

impl Tool for OpenApplication {
    fn name(&self) -> &'static str {
        "open_application"
    }

    fn description(&self) -> &'static str {
        "Abre una aplicación de la lista que el usuario permitió (sus claves y nombres están en el parámetro). \
         No puede abrir nada que no esté en esa lista."
    }

    fn parameters(&self) -> Value {
        let state = self.catalog.read();
        let keys = AppCatalog::keys_of(&state);
        let names: Vec<String> = APPS
            .iter()
            .filter(|app| !state.disabled.contains(app.key))
            .map(|app| format!("{} = {}", app.key, app.display))
            .chain(state.custom.iter().map(|app| format!("{} = {}", app.key, app.name)))
            .collect();
        let mut property = json!({ "type": "string", "description": format!("Aplicación a abrir. Opciones: {}.", names.join("; ")) });
        // Un esquema con una lista vacía de valores no es válido: sin apps activadas se deja solo la descripción.
        if !keys.is_empty() {
            property["enum"] = json!(keys);
        }
        json!({ "type": "object", "properties": { "application": property }, "required": ["application"] })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        self.selected(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match self.selected(args) {
            Ok((name, target)) => format!("Abrir {name} ({})", target.describe()),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let (name, target) = self.selected(args)?;
        launch(&target).map_err(|error| ToolError::Failed(format!("No se pudo abrir {name}: {error}")))?;
        Ok(ToolOutput { summary: format!("Abrí {name}"), data: json!({ "application": required_str(args, "application").unwrap_or_default() }) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovered(name: &str, id: &str) -> DiscoveredApp {
        DiscoveredApp { name: name.into(), app_id: id.into() }
    }

    fn catalog_with(apps: Vec<DiscoveredApp>) -> Arc<AppCatalog> {
        let catalog = Arc::new(AppCatalog::new());
        catalog.set_discovered(apps);
        catalog
    }

    #[test]
    fn bloquea_consolas_terminales_y_herramientas_de_sistema() {
        for (name, id) in [
            ("Windows PowerShell", "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe"),
            ("Símbolo del sistema", "Microsoft.WindowsCommandPrompt"),
            ("Terminal", "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"),
            ("Editor del Registro", "{1AC14E77}\\regedit.exe"),
            ("Ubuntu", "CanonicalGroupLimited.Ubuntu_79rhkp1fndgsc!ubuntu"),
            ("Git Bash", "C:\\Program Files\\Git\\git-bash.exe"),
            ("Raro", "{1AC14E77}\\cmd.exe"),
            ("Consola de administración de Microsoft", "mmc.exe"),
        ] {
            assert!(is_blocked(name, id), "{name}");
        }
        for (name, id) in [
            ("Spotify", "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            ("Docker Desktop", "{6D809377}\\Docker\\Docker\\Docker Desktop.exe"),
            ("Discord", "com.squirrel.Discord.Discord"),
            ("Calculadora", "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"),
            ("Visual Studio Code", "Microsoft.VisualStudioCode"),
            ("Terminator: la película", "juegos\\terminator.exe"),
        ] {
            assert!(!is_blocked(name, id), "{name} no debía bloquearse");
        }
    }

    #[test]
    fn bloquea_las_herramientas_de_administracion_sin_pillar_apps_normales() {
        for (name, id) in [
            ("Services", "{1AC14E77}\\services.msc"),
            ("Event Viewer", "Microsoft.AutoGenerated.{BB044BFD}"),
            ("Task Scheduler", "Microsoft.AutoGenerated.{C1C6F8AC}"),
            ("Computer Management", "Microsoft.AutoGenerated.{8ABD94FB}"),
            ("Run", "Microsoft.Windows.Shell.RunDialog"),
            ("Component Services", "{1AC14E77}\\comexp.msc"),
        ] {
            assert!(is_blocked(name, id), "{name}");
        }
        for (name, id) in [("Servicios financieros del banco", "banco.app"), ("Runner", "juegos.runner"), ("Calculadora", "Microsoft.WindowsCalculator!App")] {
            assert!(!is_blocked(name, id), "{name} no debía bloquearse");
        }
    }

    #[test]
    fn distingue_aplicaciones_juegos_de_steam_y_enlaces() {
        for id in [
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\charmap.exe", "Microsoft.AutoGenerated.{923DD477}",
            "com.squirrel.Discord.Discord", "MSEdge", "C:\\Riot Games\\Riot Client\\RiotClientServices.exe",
        ] {
            assert_eq!(classify(id), Kind::App, "{id}");
        }
        assert_eq!(classify("steam://rungameid/391220"), Kind::SteamGame(391_220));
        assert_eq!(classify("STEAM://RUNGAMEID/7"), Kind::SteamGame(7));
        for id in [
            "https://www.acer.com/legal", "http://support.steampowered.com/", "C:\\ProgramData\\Acer\\Guia.pdf", "{X}\\Git\\ReleaseNotes.html",
            "{X}\\Windows Kits\\10\\Shortcuts\\Docs.url", "{X}\\WinRAR\\Rar.txt", "ms-msdt:/id PCWDiagnostic", "steam://rungameid/abc",
            "steam://rungameid/1/../../x", "steam://rungameid/", "steam://run/391220", "steam://rungameid/12 34", "ftp://servidor/archivo",
        ] {
            let kind = classify(id);
            assert!(kind == Kind::NotAnApp || (id.starts_with("ms-msdt") && kind == Kind::App), "{id} -> {kind:?}");
        }
    }

    #[test]
    fn ofrece_solo_aplicaciones_y_cuenta_las_herramientas_ocultas() {
        let catalog = catalog_with(vec![
            discovered("Mapa de caracteres", "{X}\\charmap.exe"),
            discovered("Acer Legal Information", "https://www.acer.com/legal"),
            discovered("Windows PowerShell", "{X}\\powershell.exe"),
            discovered("Services", "{X}\\services.msc"),
            discovered("Rise of the Tomb Raider", "steam://rungameid/391220"),
            discovered("Guía", "C:\\ProgramData\\Acer\\Guia.pdf"),
        ]);
        let (apps, hidden) = catalog.selectable().unwrap();
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Mapa de caracteres", "Rise of the Tomb Raider"]);
        assert_eq!(hidden, 2, "PowerShell y Servicios; los documentos y enlaces no cuentan como herramientas ocultas");
        assert!(AppCatalog::new().selectable().is_none());
        assert!(catalog.set_custom_enabled("https://www.acer.com/legal", true).unwrap_err().contains("no una aplicación"));
        assert!(catalog.set_custom_enabled("C:\\ProgramData\\Acer\\Guia.pdf", true).is_err());
        assert!(catalog.set_custom_enabled("{X}\\services.msc", true).is_err());
    }

    #[test]
    fn no_repite_en_las_del_menu_inicio_las_que_ya_son_conocidas() {
        let catalog = catalog_with(vec![
            discovered("Spotify", "SpotifyAB.SpotifyMusic!Spotify"),
            discovered("calculadora", "Microsoft.WindowsCalculator!App"),
            discovered("Docker Desktop", "docker-id"),
        ]);
        let names = |catalog: &AppCatalog| catalog.selectable().unwrap().0.into_iter().map(|a| a.name).collect::<Vec<_>>();
        assert_eq!(names(&catalog), ["Docker Desktop"], "Spotify y la Calculadora ya están entre las conocidas");
        // Si el usuario ya había añadido una (por ejemplo en una versión anterior), sigue apareciendo para poder quitarla.
        catalog.configure(
            Vec::<String>::new(),
            vec![CustomApp { key: "spotify_2".into(), name: "Spotify".into(), app_id: "SpotifyAB.SpotifyMusic!Spotify".into() }],
        );
        assert_eq!(names(&catalog), ["Spotify", "Docker Desktop"]);
    }

    #[test]
    fn un_juego_de_steam_se_abre_con_una_direccion_construida_por_el_programa() {
        let catalog = catalog_with(vec![discovered("Rise of the Tomb Raider", "steam://rungameid/391220")]);
        catalog.set_custom_enabled("steam://rungameid/391220", true).unwrap();
        let tool = OpenApplication::new(catalog.clone());
        let key = catalog.custom()[0].key.clone();
        assert_eq!(key, "rise_of_the_tomb_raider");
        assert_eq!(tool.selected(&json!({ "application": key })).unwrap().1, Target::Uri("steam://rungameid/391220".into()));
        assert!(tool.preview(&json!({ "application": "rise_of_the_tomb_raider" })).contains("juego de Steam"));
    }

    #[test]
    fn genera_claves_estables_y_unicas() {
        assert_eq!(slug("Docker Desktop"), "docker_desktop");
        assert_eq!(slug("  Ñandú  Músic@ (beta) "), "nandu_music_beta");
        assert_eq!(slug("✔✔✔"), "app");
        assert!(valid_key(&slug(&"x".repeat(200))) && slug(&"x".repeat(200)).len() <= MAX_KEY_CHARS - 3);
        let taken = |k: &str| ["docker", "docker_2"].contains(&k);
        assert_eq!(unique_key("Docker", &taken), "docker_3");
        assert_eq!(unique_key("Otra", &taken), "otra");
    }

    #[test]
    fn interpreta_la_salida_de_get_startapps() {
        let many = r#"[{"Name":"Spotify","AppID":"SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"},{"Name":"calculadora","AppID":"Calc"},{"Name":"Duplicada","AppID":"Calc"},{"Name":"","AppID":"x"},{"Name":"Sin id"},{"Name":"Con\nsalto","AppID":"y"}]"#;
        let apps = parse_start_apps(many).unwrap();
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["calculadora", "Spotify"], "ordenadas sin distinguir mayúsculas; las inválidas y repetidas se descartan");
        let single = parse_start_apps(r#"{"Name":"Única","AppID":"id"}"#).unwrap();
        assert_eq!(single, vec![discovered("Única", "id")]);
        assert_eq!(parse_start_apps("").unwrap(), vec![]);
        assert_eq!(parse_start_apps("[]").unwrap(), vec![]);
        for bad in ["no es json", "42", "\"texto\""] {
            assert!(parse_start_apps(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn configure_descarta_lo_invalido_bloqueado_o_repetido() {
        let catalog = AppCatalog::new();
        let app = |key: &str, name: &str, id: &str| CustomApp { key: key.into(), name: name.into(), app_id: id.into() };
        catalog.configure(
            ["spotify".to_string(), "no_existe".to_string()],
            vec![
                app("docker", "Docker Desktop", "docker-id"),
                app("docker", "Repetida", "otro-id"),
                app("otra", "Mismo id", "docker-id"),
                app("spotify", "Colisiona con una conocida", "x"),
                app("Mayus", "Clave inválida", "y"),
                app("consola", "Windows PowerShell", "powershell.exe"),
                app("vacio", "", "z"),
                app("largo", &"x".repeat(MAX_NAME_CHARS + 1), "w"),
            ],
        );
        assert_eq!(catalog.disabled().into_iter().collect::<Vec<_>>(), ["spotify"], "las claves desconocidas se ignoran");
        assert_eq!(catalog.custom().iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["docker"]);
    }

    #[test]
    fn activar_y_desactivar_aplicaciones_del_menu_inicio() {
        let catalog = catalog_with(vec![
            discovered("Docker Desktop", "docker-id"),
            discovered("Windows PowerShell", "powershell.exe"),
            discovered("Spotify", "spotify-id"),
        ]);
        catalog.set_custom_enabled("docker-id", true).unwrap();
        catalog.set_custom_enabled("docker-id", true).unwrap();
        assert_eq!(catalog.custom().len(), 1, "activar dos veces no duplica");
        assert!(catalog.enabled_keys().contains(&"docker_desktop".to_string()));
        // «Spotify» colisiona con la conocida: la clave nueva es distinta.
        catalog.set_custom_enabled("spotify-id", true).unwrap();
        assert!(catalog.custom().iter().any(|a| a.key == "spotify_2"));
        assert!(catalog.set_custom_enabled("powershell.exe", true).unwrap_err().contains("herramienta del sistema"));
        assert!(catalog.set_custom_enabled("C:\\Windows\\System32\\calc.exe", true).unwrap_err().contains("no aparece"));
        catalog.set_custom_enabled("docker-id", false).unwrap();
        assert!(!catalog.enabled_keys().contains(&"docker_desktop".to_string()));
        // Sin haber leído el menú Inicio no se puede activar nada.
        assert!(AppCatalog::new().set_custom_enabled("x", true).unwrap_err().contains("actualiza la lista"));
    }

    #[test]
    fn limita_el_numero_de_aplicaciones_añadidas() {
        let apps: Vec<_> = (0..MAX_CUSTOM_APPS + 5).map(|i| discovered(&format!("App número {i}"), &format!("id-{i}"))).collect();
        let catalog = catalog_with(apps);
        for i in 0..MAX_CUSTOM_APPS {
            catalog.set_custom_enabled(&format!("id-{i}"), true).unwrap();
        }
        assert!(catalog.set_custom_enabled("id-100", true).unwrap_err().contains("Ya hay"));
        let keys = catalog.enabled_keys();
        let unique: BTreeSet<_> = keys.iter().collect();
        assert_eq!(keys.len(), unique.len(), "no hay claves repetidas");
    }

    #[test]
    fn la_herramienta_solo_acepta_lo_activado_y_el_esquema_lo_refleja() {
        let catalog = catalog_with(vec![discovered("Docker Desktop", "docker-id")]);
        let tool = OpenApplication::new(catalog.clone());
        let enum_of = |tool: &OpenApplication| -> Vec<String> {
            tool.parameters()["properties"]["application"]["enum"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect()
        };
        assert!(enum_of(&tool).contains(&"spotify".to_string()) && !enum_of(&tool).contains(&"docker_desktop".to_string()));
        assert!(matches!(tool.validate(&json!({ "application": "docker_desktop" })), Err(ToolError::InvalidArguments(_))));

        catalog.set_custom_enabled("docker-id", true).unwrap();
        assert!(enum_of(&tool).contains(&"docker_desktop".to_string()), "el esquema se actualiza al instante");
        assert!(tool.parameters()["properties"]["application"]["description"].as_str().unwrap().contains("docker_desktop = Docker Desktop"));
        tool.validate(&json!({ "application": "docker_desktop" })).unwrap();
        assert!(tool.preview(&json!({ "application": "docker_desktop" })).contains("Abrir Docker Desktop (aplicación del menú Inicio)"));

        catalog.set_builtin_enabled("notepad", false).unwrap();
        assert!(!enum_of(&tool).contains(&"notepad".to_string()));
        assert!(matches!(tool.validate(&json!({ "application": "notepad" })), Err(ToolError::NotAllowed(m)) if m.contains("desactivada")));
        catalog.set_builtin_enabled("notepad", true).unwrap();
        tool.validate(&json!({ "application": "notepad" })).unwrap();
        assert!(catalog.set_builtin_enabled("cmd", false).is_err());
    }

    #[test]
    fn una_app_que_ya_no_esta_en_el_menu_se_rechaza() {
        let catalog = catalog_with(vec![discovered("Docker Desktop", "docker-id")]);
        catalog.set_custom_enabled("docker-id", true).unwrap();
        catalog.set_discovered(vec![]);
        let tool = OpenApplication::new(catalog);
        assert!(matches!(tool.validate(&json!({ "application": "docker_desktop" })), Err(ToolError::Failed(m)) if m.contains("ya no aparece")));
    }

    #[test]
    fn nunca_acepta_rutas_ni_comandos() {
        let tool = OpenApplication::new(catalog_with(vec![]));
        for value in ["cmd", "powershell", "C:\\Windows\\System32\\cmd.exe", "notepad.exe /c calc", "", "NOTEPAD && calc", "../x"] {
            assert!(matches!(tool.validate(&json!({ "application": value })), Err(ToolError::InvalidArguments(_))), "{value:?}");
        }
        assert!(tool.validate(&json!({ "application": "notepad", "args": "x" })).is_err());
        assert!(tool.validate(&json!({ "application": "NOTEPAD" })).is_ok());
        assert_eq!(tool.risk(&json!({})), RiskLevel::Safe);
    }

    #[test]
    fn sin_aplicaciones_activadas_el_esquema_sigue_siendo_valido() {
        let catalog = Arc::new(AppCatalog::new());
        for app in APPS {
            catalog.set_builtin_enabled(app.key, false).unwrap();
        }
        let schema = OpenApplication::new(catalog).parameters();
        assert!(schema["properties"]["application"].get("enum").is_none());
        assert_eq!(schema["required"], json!(["application"]));
    }

    /// Dónde encuentra cada app conocida y cuántas del menú Inicio lista: `-- --ignored --nocapture catalogo_real`.
    #[test]
    #[ignore = "depende de lo instalado en el equipo"]
    fn catalogo_real_muestra_lo_que_encuentra() {
        let catalog = AppCatalog::new();
        for status in catalog.builtin_status() {
            println!("{:<12} activada={} instalada={}", status.key, status.enabled, status.installed);
        }
        let apps = catalog.refresh().expect("el menú Inicio debe poder consultarse");
        println!("--- {} aplicaciones en el menú Inicio ---", apps.len());
        for app in &apps {
            println!("{}{}  [{}]", if is_blocked(&app.name, &app.app_id) { "(bloqueada) " } else { "" }, app.name, app.app_id);
        }
        assert!(apps.len() > 5);
    }

    /// Abre de verdad una app del menú Inicio añadida por el usuario (la Calculadora): `-- --ignored abre_una_app_anadida`.
    #[test]
    #[ignore = "abre una ventana"]
    fn abre_una_app_anadida_de_verdad() {
        let catalog = Arc::new(AppCatalog::new());
        let apps = catalog.refresh().unwrap();
        // Dos tipos de identificador: una app de Store y una clásica con identificador de ruta.
        for (name, fragment) in [("Calculadora", "WindowsCalculator"), ("Character Map", "charmap.exe")] {
            let app = apps.iter().find(|a| a.name == name && a.app_id.contains(fragment)).unwrap_or_else(|| panic!("«{name}» está en el menú Inicio"));
            catalog.set_custom_enabled(&app.app_id, true).unwrap();
        }
        let tool = OpenApplication::new(catalog.clone());
        for custom in catalog.custom() {
            let output = tool.execute(&json!({ "application": custom.key })).unwrap();
            println!("{}  [{}]", output.summary, custom.app_id);
        }
    }
}
