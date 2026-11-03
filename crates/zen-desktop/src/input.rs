//! Control de otras aplicaciones: ventanas, teclado y reglas de seguridad.
//!
//! Aquí está todo lo que toca Windows (listar ventanas, traer una al frente, simular teclas) y las reglas
//! que deciden dónde NO se puede actuar nunca: terminales, administradores del sistema, avisos de
//! contraseña, la propia mascota… Las reglas son funciones puras, para poder probarlas una a una.

use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// Texto máximo que se escribe de una vez.
pub const MAX_TYPED_CHARS: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
    /// Nombre del programa en minúsculas y sin «.exe» (por ejemplo `notepad`). Vacío si Windows no deja leerlo.
    pub process: String,
    pub pid: u32,
}

// ---------------------------------------------------------------------------------------------
// Reglas de seguridad (puras)
// ---------------------------------------------------------------------------------------------

/// Programas donde nunca se escribe ni se hace clic: escribir en un terminal equivale a ejecutar comandos,
/// y el resto administra el sistema, la seguridad o a la propia mascota.
const BLOCKED_PROCESSES: &[&str] = &[
    "cmd", "powershell", "powershell_ise", "pwsh", "windowsterminal", "wt", "conhost", "openconsole", "wsl", "wslhost", "bash", "mintty",
    "regedit", "mmc", "taskmgr", "consent", "credentialuibroker", "logonui", "lockapp", "securityhealthsystray", "windowsdefender",
    "msiexec", "services", "secpol", "gpedit", "systemsettings", "control", "runas", "sudo", "ryan", "zen-pet", "zen_pet", "winlogon", "wininit",
];

/// Palabras que delatan una ventana de contraseñas, pagos o permisos de administrador.
const SENSITIVE_TITLE_WORDS: &[&str] = &[
    "contraseña", "contrasena", "password", "passcode", "clave", "pin de", "iniciar sesión", "iniciar sesion", "sign in", "log in", "login",
    "credencial", "credential", "autenticación", "autenticacion", "verificación", "verificacion", "banco", "bank", "tarjeta", "card number",
    "pago", "payment", "checkout", "seguridad de windows", "windows security", "control de cuentas", "user account control", "administrador",
    "bitlocker", "keepass", "bitwarden", "1password", "lastpass",
];

pub fn is_blocked_process(process: &str) -> bool {
    let name = process.trim().to_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    BLOCKED_PROCESSES.contains(&name)
}

pub fn is_sensitive_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    SENSITIVE_TITLE_WORDS.iter().any(|word| lower.contains(word))
}

/// ¿Se puede actuar sobre esta ventana? Devuelve el motivo si no.
pub fn check_target(window: &WindowInfo) -> Result<(), String> {
    if window.process.is_empty() {
        return Err("No puedo comprobar qué programa es esa ventana (puede estar ejecutándose como administrador).".into());
    }
    if is_blocked_process(&window.process) {
        return Err(format!("Por seguridad no actúo sobre «{}» (terminales, administradores del sistema y la propia mascota).", window.process));
    }
    if is_sensitive_title(&window.title) {
        return Err(format!("Por seguridad no actúo sobre «{}»: parece una ventana de contraseñas, pagos o permisos.", window.title));
    }
    Ok(())
}

/// Texto que se puede escribir: sin caracteres de control (salvo salto de línea y tabulador) y de tamaño razonable.
pub fn validate_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("el texto está vacío".into());
    }
    if text.chars().count() > MAX_TYPED_CHARS {
        return Err(format!("el texto es demasiado largo (máximo {MAX_TYPED_CHARS} caracteres)"));
    }
    if let Some(bad) = text.chars().find(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(format!("el texto contiene un carácter de control no permitido (U+{:04X})", bad as u32));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Combinaciones de teclas
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    modifiers: Vec<u16>,
    key: u16,
    display: String,
}

impl Combo {
    /// «Ctrl+Shift+T»
    pub fn display(&self) -> &str {
        &self.display
    }

    /// Los códigos de las teclas, en el orden en que se pulsan.
    pub fn keys(&self) -> Vec<u16> {
        let mut keys = self.modifiers.clone();
        keys.push(self.key);
        keys
    }
}

fn named_key(name: &str) -> Option<(u16, String)> {
    let named = [
        ("enter", VK_RETURN, "Enter"), ("intro", VK_RETURN, "Enter"), ("return", VK_RETURN, "Enter"),
        ("tab", VK_TAB, "Tab"), ("esc", VK_ESCAPE, "Esc"), ("escape", VK_ESCAPE, "Esc"),
        ("space", VK_SPACE, "Espacio"), ("espacio", VK_SPACE, "Espacio"),
        ("backspace", VK_BACK, "Retroceso"), ("retroceso", VK_BACK, "Retroceso"),
        ("delete", VK_DELETE, "Supr"), ("supr", VK_DELETE, "Supr"),
        ("home", VK_HOME, "Inicio"), ("end", VK_END, "Fin"),
        ("pageup", VK_PRIOR, "RePág"), ("pagedown", VK_NEXT, "AvPág"),
        ("up", VK_UP, "↑"), ("down", VK_DOWN, "↓"), ("left", VK_LEFT, "←"), ("right", VK_RIGHT, "→"),
    ];
    if let Some((_, vk, label)) = named.iter().find(|(n, _, _)| *n == name) {
        return Some((*vk, label.to_string()));
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => Some((c.to_ascii_uppercase() as u16, c.to_ascii_uppercase().to_string())),
        (Some(c), None) if c.is_ascii_digit() => Some((c as u16, c.to_string())),
        _ => {
            let number = name.strip_prefix('f')?.parse::<u16>().ok().filter(|n| (1..=12).contains(n))?;
            Some((0x70 + number - 1, format!("F{number}"))) // VK_F1 = 0x70
        }
    }
}

/// Interpreta «ctrl+shift+t», «enter», «alt+left»… Solo admite Ctrl, Alt y Shift como modificadores y
/// rechaza las combinaciones que cierran programas, abren el sistema o cambian de usuario.
pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let lower = text.trim().to_lowercase();
    if lower.is_empty() {
        return Err("la combinación de teclas está vacía".into());
    }
    let parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let (key_name, modifier_names) = parts.split_last().expect("split devuelve al menos un elemento");
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for name in modifier_names {
        let flag = match *name {
            "ctrl" | "control" => &mut ctrl,
            "alt" => &mut alt,
            "shift" | "mayus" => &mut shift,
            "win" | "windows" | "super" | "meta" | "cmd" => return Err("las combinaciones con la tecla Windows no están permitidas".into()),
            other => return Err(format!("modificador desconocido «{other}»; usa ctrl, alt o shift")),
        };
        if *flag {
            return Err(format!("el modificador «{name}» está repetido"));
        }
        *flag = true;
    }
    let (key, label) = named_key(key_name).ok_or_else(|| format!("tecla desconocida «{key_name}»"))?;
    if ctrl && alt {
        return Err("Ctrl+Alt no está permitido (es la base de Ctrl+Alt+Supr y de atajos del sistema)".into());
    }
    if alt && key == 0x73 {
        return Err("Alt+F4 no está permitido: cierra programas".into());
    }
    if ctrl && shift && key == VK_ESCAPE {
        return Err("Ctrl+Mayús+Esc no está permitido: abre el Administrador de tareas".into());
    }
    let mut modifiers = Vec::new();
    let mut display = Vec::new();
    for (on, vk, name) in [(ctrl, VK_CONTROL, "Ctrl"), (alt, VK_MENU, "Alt"), (shift, VK_SHIFT, "Mayús")] {
        if on {
            modifiers.push(vk);
            display.push(name.to_string());
        }
    }
    display.push(label);
    Ok(Combo { modifiers, key, display: display.join("+") })
}

// ---------------------------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------------------------

fn process_name(pid: u32) -> String {
    // SAFETY: se abre el proceso solo para consultar su ruta, y el manejador se cierra siempre.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return String::new();
        }
        let mut buffer = [0u16; 520];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(handle);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        Path::new(&path).file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
}

fn window_info(hwnd: HWND) -> WindowInfo {
    // SAFETY: llamadas de solo lectura sobre un identificador de ventana; si ya no existe devuelven vacío.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).clamp(0, 1_000) as usize;
        let mut buffer = vec![0u16; length + 1];
        let copied = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        WindowInfo { hwnd: hwnd as isize, title: String::from_utf16_lossy(&buffer[..copied]), process: process_name(pid), pid }
    }
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = &mut *(lparam as *mut Vec<WindowInfo>);
    let is_tool_window = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0;
    if IsWindowVisible(hwnd) != 0 && !is_tool_window && GetWindow(hwnd, GW_OWNER).is_null() {
        let info = window_info(hwnd);
        if !info.title.trim().is_empty() {
            found.push(info);
        }
    }
    1
}

/// Ventanas abiertas y visibles (sin las auxiliares), sin contar las de esta misma aplicación.
pub fn visible_windows() -> Vec<WindowInfo> {
    let mut found: Vec<WindowInfo> = Vec::new();
    // SAFETY: el puntero a `found` solo se usa durante la llamada, que es síncrona.
    unsafe {
        EnumWindows(Some(collect_window), &mut found as *mut Vec<WindowInfo> as LPARAM);
    }
    let own = std::process::id();
    found.retain(|window| window.pid != own);
    found
}

/// Ventanas cuyo título contiene el texto (sin distinguir mayúsculas). Si alguna coincide exactamente, solo esa.
pub fn find_windows(windows: &[WindowInfo], query: &str) -> Vec<WindowInfo> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let exact: Vec<&WindowInfo> = windows.iter().filter(|w| w.title.to_lowercase() == query).collect();
    if !exact.is_empty() {
        return exact.into_iter().cloned().collect();
    }
    windows.iter().filter(|w| w.title.to_lowercase().contains(&query)).cloned().collect()
}

pub fn foreground_window() -> Option<WindowInfo> {
    // SAFETY: solo lectura.
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then(|| window_info(hwnd))
}

/// Trae la ventana al frente y espera a que de verdad lo esté. Windows solo lo permite mientras esta
/// aplicación tenga el foco (por ejemplo, justo después de que el usuario pulse «Confirmar»).
pub fn bring_to_front(window: &WindowInfo) -> Result<(), String> {
    let hwnd = window.hwnd as HWND;
    // SAFETY: operaciones de ventana estándar sobre un identificador; si no es válido, fallan sin efecto.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    let deadline = Instant::now() + Duration::from_millis(1_500);
    while Instant::now() < deadline {
        if foreground_window().is_some_and(|front| front.hwnd == window.hwnd) {
            sleep(Duration::from_millis(120)); // margen para que la ventana acepte teclas
            return Ok(());
        }
        sleep(Duration::from_millis(40));
    }
    Err(format!("No pude traer «{}» al frente. Haz clic en esa ventana y vuelve a intentarlo.", window.title))
}

/// Comprueba que la ventana al frente sigue siendo la esperada y que sigue siendo segura.
pub fn ensure_foreground(window: &WindowInfo) -> Result<(), String> {
    match foreground_window() {
        Some(front) if front.hwnd == window.hwnd => check_target(&front),
        _ => Err("La ventana dejó de estar al frente; no escribí nada.".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Teclado
// ---------------------------------------------------------------------------------------------

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    // SAFETY: `inputs` es un trozo válido de estructuras INPUT y el tamaño es el de una de ellas.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err("Windows rechazó las teclas (¿la ventana se ejecuta como administrador?).".into())
    }
}

/// Escribe el texto en la ventana que tenga el foco. Cada trozo comprueba antes que no ha cambiado.
pub fn type_text(window: &WindowInfo, text: &str) -> Result<usize, String> {
    validate_text(text)?;
    let mut typed = 0;
    let chars: Vec<char> = text.chars().filter(|c| *c != '\r').collect();
    for chunk in chars.chunks(20) {
        ensure_foreground(window)?;
        let mut inputs = Vec::new();
        for &c in chunk {
            match c {
                '\n' => {
                    inputs.push(key_input(VK_RETURN, 0, 0));
                    inputs.push(key_input(VK_RETURN, 0, KEYEVENTF_KEYUP));
                }
                '\t' => {
                    inputs.push(key_input(VK_TAB, 0, 0));
                    inputs.push(key_input(VK_TAB, 0, KEYEVENTF_KEYUP));
                }
                _ => {
                    let mut units = [0u16; 2];
                    for unit in c.encode_utf16(&mut units) {
                        inputs.push(key_input(0, *unit, KEYEVENTF_UNICODE));
                        inputs.push(key_input(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    }
                }
            }
        }
        send(&inputs)?;
        typed += chunk.len();
        sleep(Duration::from_millis(15));
    }
    Ok(typed)
}

/// Pulsa una combinación: baja las teclas en orden y las suelta al revés.
pub fn press_combo(window: &WindowInfo, combo: &Combo) -> Result<(), String> {
    ensure_foreground(window)?;
    let keys = combo.keys();
    let mut inputs: Vec<INPUT> = keys.iter().map(|&vk| key_input(vk, 0, 0)).collect();
    inputs.extend(keys.iter().rev().map(|&vk| key_input(vk, 0, KEYEVENTF_KEYUP)));
    send(&inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, process: &str) -> WindowInfo {
        WindowInfo { hwnd: 1, title: title.into(), process: process.into(), pid: 4 }
    }

    #[test]
    fn los_terminales_y_administradores_estan_bloqueados() {
        for process in ["cmd", "powershell", "pwsh", "WindowsTerminal", "wt.exe", "regedit", "taskmgr", "consent", "ryan", "zen-pet", "systemsettings", "bash"] {
            assert!(is_blocked_process(process), "{process}");
            assert!(check_target(&window("Cualquier cosa", process)).is_err(), "{process}");
        }
        for process in ["notepad", "chrome", "spotify", "code", "explorer", "msedge", "winword"] {
            assert!(!is_blocked_process(process), "{process}");
            assert!(check_target(&window("Documento", process)).is_ok(), "{process}");
        }
    }

    #[test]
    fn las_ventanas_de_contrasenas_y_pagos_estan_bloqueadas() {
        for title in ["Iniciar sesión - Microsoft", "Banco Santander - Chrome", "Enter your PASSWORD", "Seguridad de Windows", "Control de cuentas de usuario", "Checkout - Tienda", "KeePass"] {
            assert!(is_sensitive_title(title), "{title}");
            assert!(check_target(&window(title, "chrome")).is_err(), "{title}");
        }
        for title in ["Sin título - Bloc de notas", "Informe.docx - Word", "Spotify Premium", "Receta de pasta"] {
            assert!(!is_sensitive_title(title), "{title}");
        }
    }

    #[test]
    fn un_programa_ilegible_se_rechaza() {
        assert!(check_target(&window("Algo", "")).unwrap_err().contains("administrador"));
    }

    #[test]
    fn valida_el_texto_a_escribir() {
        for ok in ["hola", "línea 1\nlínea 2", "con\ttab", "ñandú 😀", &"a".repeat(MAX_TYPED_CHARS)] {
            assert!(validate_text(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", "\u{7}", "esc\u{1b}[0m", "nulo\0", &"a".repeat(MAX_TYPED_CHARS + 1)] {
            assert!(validate_text(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn interpreta_combinaciones_de_teclas() {
        let combo = parse_combo("Ctrl+Shift+T").unwrap();
        assert_eq!(combo.display(), "Ctrl+Mayús+T");
        assert_eq!(combo.keys(), vec![VK_CONTROL, VK_SHIFT, b'T' as u16]);
        assert_eq!(parse_combo("enter").unwrap().keys(), vec![VK_RETURN]);
        assert_eq!(parse_combo(" ALT + Left ").unwrap().keys(), vec![VK_MENU, VK_LEFT]);
        assert_eq!(parse_combo("f5").unwrap().keys(), vec![0x74]);
        assert_eq!(parse_combo("ctrl+9").unwrap().keys(), vec![VK_CONTROL, b'9' as u16]);
        assert_eq!(parse_combo("ctrl+c").unwrap().display(), "Ctrl+C");
    }

    #[test]
    fn rechaza_combinaciones_peligrosas_o_mal_escritas() {
        for bad in [
            "", "ctrl+", "+c", "win+r", "windows+d", "super+l", "meta+e", "alt+f4", "Alt+F4", "ctrl+alt+delete", "ctrl+alt+t", "ctrl+shift+esc",
            "ctrl+ctrl+c", "ctrl+f13", "ctrl+f0", "ctrl+cc", "hyper+x", "ctrl+ñ", "ctrl+c+v",
        ] {
            assert!(parse_combo(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn busca_ventanas_por_titulo() {
        let all = vec![window("Sin título - Bloc de notas", "notepad"), window("Notas - OneNote", "onenote"), window("Notas", "notes"), window("Spotify", "spotify")];
        assert_eq!(find_windows(&all, "bloc de notas").len(), 1);
        assert_eq!(find_windows(&all, "notas").len(), 1, "una coincidencia exacta gana a las parciales");
        assert_eq!(find_windows(&all, "NOTA").len(), 3);
        assert!(find_windows(&all, "zzz").is_empty());
        assert!(find_windows(&all, "  ").is_empty());
    }

    #[test]
    fn la_lista_real_de_ventanas_no_incluye_la_propia() {
        let own = std::process::id();
        assert!(visible_windows().iter().all(|w| w.pid != own && !w.title.trim().is_empty()));
    }
}
