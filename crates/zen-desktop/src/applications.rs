use std::path::{Path, PathBuf};
use std::process::Command;

use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

/// Formas de encontrar una aplicación, en orden de preferencia.
enum Find {
    /// Ejecutable en una ruta; `%VAR%` se expande con el entorno.
    Exe(&'static str),
    /// Ejecutable registrado en «App Paths» (lo que usa Windows para Win+R).
    AppPaths(&'static str),
    /// Aplicación de Microsoft Store: prefijo del paquete instalado e identificador de aplicación.
    Store { package_prefix: &'static str, aumid: &'static str },
    /// Acceso directo del menú Inicio, por nombre de archivo sin «.lnk».
    StartMenu(&'static str),
}

pub(crate) struct AppEntry {
    pub(crate) key: &'static str,
    pub(crate) display: &'static str,
    find: &'static [Find],
}

/// Lista blanca: la IA solo puede elegir una clave de esta tabla, nunca una ruta ni un comando.
pub(crate) const APPS: &[AppEntry] = &[
    AppEntry {
        key: "notepad",
        display: "Bloc de notas",
        find: &[
            Find::Exe("%SystemRoot%\\System32\\notepad.exe"),
            Find::Store { package_prefix: "Microsoft.WindowsNotepad_", aumid: "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App" },
        ],
    },
    AppEntry {
        key: "calculator",
        display: "Calculadora",
        find: &[
            Find::Exe("%SystemRoot%\\System32\\calc.exe"),
            Find::Store { package_prefix: "Microsoft.WindowsCalculator_", aumid: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App" },
        ],
    },
    AppEntry {
        key: "paint",
        display: "Paint",
        find: &[
            Find::Exe("%SystemRoot%\\System32\\mspaint.exe"),
            Find::Store { package_prefix: "Microsoft.Paint_", aumid: "Microsoft.Paint_8wekyb3d8bbwe!App" },
        ],
    },
    AppEntry { key: "explorer", display: "Explorador de archivos", find: &[Find::Exe("%SystemRoot%\\explorer.exe")] },
    AppEntry {
        key: "vscode",
        display: "Visual Studio Code",
        find: &[
            Find::AppPaths("Code.exe"),
            Find::Exe("%LOCALAPPDATA%\\Programs\\Microsoft VS Code\\Code.exe"),
            Find::Exe("%ProgramFiles%\\Microsoft VS Code\\Code.exe"),
            Find::StartMenu("Visual Studio Code"),
        ],
    },
    AppEntry {
        key: "spotify",
        display: "Spotify",
        find: &[
            Find::Store { package_prefix: "SpotifyAB.SpotifyMusic_", aumid: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify" },
            Find::Exe("%APPDATA%\\Spotify\\Spotify.exe"),
            Find::StartMenu("Spotify"),
        ],
    },
    AppEntry { key: "discord", display: "Discord", find: &[Find::StartMenu("Discord")] },
    AppEntry {
        key: "steam",
        display: "Steam",
        find: &[Find::Exe("%ProgramFiles(x86)%\\Steam\\steam.exe"), Find::StartMenu("Steam")],
    },
    AppEntry {
        key: "chrome",
        display: "Google Chrome",
        find: &[
            Find::AppPaths("chrome.exe"),
            Find::Exe("%ProgramFiles%\\Google\\Chrome\\Application\\chrome.exe"),
            Find::Exe("%ProgramFiles(x86)%\\Google\\Chrome\\Application\\chrome.exe"),
            Find::Exe("%LOCALAPPDATA%\\Google\\Chrome\\Application\\chrome.exe"),
        ],
    },
    AppEntry {
        key: "edge",
        display: "Microsoft Edge",
        find: &[
            Find::AppPaths("msedge.exe"),
            Find::Exe("%ProgramFiles(x86)%\\Microsoft\\Edge\\Application\\msedge.exe"),
        ],
    },
];

/// Cómo se abrirá una aplicación ya encontrada.
#[derive(Debug, PartialEq)]
pub(crate) enum Target {
    Exe(PathBuf),
    /// Identificador de aplicación de Microsoft Store.
    Store(String),
    /// Identificador de una aplicación del menú Inicio (la que el usuario añadió a su lista).
    AppsFolder(String),
    /// Dirección que construye el propio programa (hoy, solo `steam://rungameid/<número>`).
    Uri(String),
    Shortcut(PathBuf),
}

impl Target {
    pub(crate) fn describe(&self) -> String {
        match self {
            Target::Exe(path) | Target::Shortcut(path) => crate::locations::display_path(path),
            Target::Store(_) => "aplicación de Microsoft Store".to_string(),
            Target::AppsFolder(_) => "aplicación del menú Inicio".to_string(),
            Target::Uri(_) => "juego de Steam".to_string(),
        }
    }
}

fn expand_env(template: &str) -> Option<String> {
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        result.push_str(&std::env::var(&after[..end]).ok()?);
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Some(result)
}

pub(crate) fn entry(key: &str) -> Option<&'static AppEntry> {
    APPS.iter().find(|app| app.key.eq_ignore_ascii_case(key))
}

fn app_paths(exe_name: &str) -> Option<PathBuf> {
    let subkey = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe_name}");
    [CURRENT_USER, LOCAL_MACHINE].iter().find_map(|root| {
        let path = PathBuf::from(root.open(&subkey).ok()?.get_string("").ok()?.trim_matches('"').to_string());
        path.is_file().then_some(path)
    })
}

/// Paquetes de Microsoft Store instalados para este usuario.
fn store_package_installed(prefix: &str) -> bool {
    const PACKAGES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Packages";
    let Ok(key) = CURRENT_USER.open(PACKAGES) else { return false };
    let Ok(mut names) = key.keys() else { return false };
    names.any(|name| name.starts_with(prefix))
}

fn find_shortcut(dir: &Path, wanted: &str, depth: usize) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                if let Some(found) = find_shortcut(&path, wanted, depth - 1) {
                    return Some(found);
                }
            }
        } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
            && path.file_stem().is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(wanted))
        {
            return Some(path);
        }
    }
    None
}

fn start_menu_shortcut(name: &str) -> Option<PathBuf> {
    ["APPDATA", "ProgramData"].iter().find_map(|var| {
        let root = PathBuf::from(std::env::var(var).ok()?).join("Microsoft\\Windows\\Start Menu\\Programs");
        find_shortcut(&root, name, 2)
    })
}

pub(crate) fn resolve(app: &AppEntry) -> Option<Target> {
    app.find.iter().find_map(|way| match way {
        Find::Exe(template) => expand_env(template).map(PathBuf::from).filter(|path| path.is_file()).map(Target::Exe),
        Find::AppPaths(exe) => app_paths(exe).map(Target::Exe),
        Find::Store { package_prefix, aumid } => store_package_installed(package_prefix).then(|| Target::Store(aumid.to_string())),
        Find::StartMenu(name) => start_menu_shortcut(name).map(Target::Shortcut),
    })
}

/// ¿Está instalada la aplicación de la lista con esa clave?
pub(crate) fn is_installed(key: &str) -> bool {
    entry(key).and_then(resolve).is_some()
}

fn explorer() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("explorer.exe")
}

/// Sin shell y sin argumentos que controle la IA: los identificadores y rutas salen de la tabla o del sistema.
pub(crate) fn launch(target: &Target) -> std::io::Result<()> {
    match target {
        Target::Exe(path) => Command::new(path).spawn().map(drop),
        Target::Store(id) | Target::AppsFolder(id) => Command::new(explorer()).arg(format!("shell:AppsFolder\\{id}")).spawn().map(drop),
        Target::Shortcut(path) => Command::new(explorer()).arg(path).spawn().map(drop),
        Target::Uri(uri) => Command::new(explorer()).arg(uri).spawn().map(drop),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expande_variables_de_entorno() {
        assert!(expand_env("%SystemRoot%\\System32").unwrap().ends_with("System32"));
        assert_eq!(expand_env("sin variables").as_deref(), Some("sin variables"));
        assert!(expand_env("%NO_EXISTE_ZEN%\\x").is_none());
        assert!(expand_env("%sin_cierre").is_none());
    }

    #[test]
    fn la_tabla_no_tiene_claves_repetidas_ni_vacias() {
        let mut keys: Vec<&str> = APPS.iter().map(|app| app.key).collect();
        assert!(APPS.iter().all(|app| !app.find.is_empty()));
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), APPS.len());
    }

    #[test]
    fn encuentra_el_bloc_de_notas() {
        assert!(matches!(resolve(entry("NOTEPAD").unwrap()), Some(Target::Exe(_) | Target::Store(_))));
        assert!(entry("cmd").is_none() && entry("powershell").is_none());
    }

    #[test]
    fn busca_accesos_directos_en_subcarpetas_sin_distinguir_mayusculas() {
        let t = crate::testutil::temp_locations();
        let nested = t.locations_root("desktop").join("Empresa").join("Sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Mi App.LNK"), b"").unwrap();
        std::fs::write(nested.join("otra.txt"), b"").unwrap();
        let root = t.locations_root("desktop");
        assert_eq!(find_shortcut(&root, "mi app", 2), Some(nested.join("Mi App.LNK")));
        assert_eq!(find_shortcut(&root, "otra", 2), None, "solo accesos directos");
        assert_eq!(find_shortcut(&root, "mi app", 1), None, "respeta la profundidad máxima");
    }
}