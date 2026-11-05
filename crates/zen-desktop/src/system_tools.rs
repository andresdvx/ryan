//! Información del equipo, brillo de la pantalla y portapapeles.

use std::time::Duration;

use serde_json::{json, Value};
use windows_registry::LOCAL_MACHINE;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use zen_core::tools::{RiskLevel, Tool, ToolError, ToolOutput};

use crate::args::{check_keys, invalid};
use crate::clock::LocalNow;

const MIN_BRIGHTNESS: i64 = 10;
const MAX_CLIPBOARD_CHARS: usize = 10_000;
const MAX_CLIPBOARD_WRITE_CHARS: usize = 100_000;

// ---------------------------------------------------------------------------------------------
// Información del equipo
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Battery {
    percent: u8,
    charging: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    os: String,
    cores: usize,
    memory_total: u64,
    memory_used_percent: u32,
    battery: Option<Battery>,
    disk_free: u64,
    disk_total: u64,
    uptime_secs: u64,
    local_time: String,
    brightness: Option<u8>,
}

fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.1} GB", value / GIB)
    } else {
        format!("{:.0} MB", value / MIB)
    }
}

fn format_uptime(secs: u64) -> String {
    let (days, hours, minutes) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
    match (days, hours) {
        (0, 0) => format!("{minutes} min"),
        (0, _) => format!("{hours} h {minutes} min"),
        _ => format!("{days} d {hours} h"),
    }
}

/// Windows 11 sigue llamándose «Windows 10» en el registro; la versión real se distingue por la compilación.
fn os_name(product: &str, build: u32) -> String {
    if build >= 22_000 {
        product.replace("Windows 10", "Windows 11")
    } else {
        product.to_string()
    }
}

impl Snapshot {
    fn describe(&self) -> String {
        let mut lines = vec![
            format!("Sistema: {}", self.os),
            format!("Hora local: {}", self.local_time),
            format!("Procesador: {} núcleos lógicos", self.cores),
            format!("Memoria: {} en total, {}% en uso", format_bytes(self.memory_total), self.memory_used_percent),
            format!(
                "Disco del sistema: {} libres de {}",
                format_bytes(self.disk_free),
                format_bytes(self.disk_total)
            ),
            format!("Encendido desde hace: {}", format_uptime(self.uptime_secs)),
        ];
        lines.push(match &self.battery {
            Some(battery) => format!("Batería: {}% ({})", battery.percent, if battery.charging { "cargando o conectada" } else { "con batería" }),
            None => "Batería: no tiene (equipo de escritorio)".into(),
        });
        if let Some(level) = self.brightness {
            lines.push(format!("Brillo de la pantalla: {level}%"));
        }
        lines.join("\n")
    }

    fn to_json(&self) -> Value {
        json!({
            "os": self.os,
            "local_time": self.local_time,
            "logical_cores": self.cores,
            "memory_total_bytes": self.memory_total,
            "memory_used_percent": self.memory_used_percent,
            "disk_free_bytes": self.disk_free,
            "disk_total_bytes": self.disk_total,
            "uptime_seconds": self.uptime_secs,
            "battery": self.battery.as_ref().map(|b| json!({ "percent": b.percent, "charging": b.charging })),
            "brightness_percent": self.brightness,
        })
    }
}

fn read_os() -> String {
    let Ok(key) = LOCAL_MACHINE.open("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion") else {
        return "Windows".into();
    };
    let product = key.get_string("ProductName").unwrap_or_else(|_| "Windows".into());
    let build: u32 = key.get_string("CurrentBuild").ok().and_then(|b| b.parse().ok()).unwrap_or(0);
    let version = key.get_string("DisplayVersion").map(|v| format!(" {v}")).unwrap_or_default();
    format!("{}{version} (compilación {build})", os_name(&product, build))
}

fn read_memory() -> (u64, u32) {
    // SAFETY: `status` es una estructura propia, con `dwLength` inicializado como exige la API.
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut status) != 0 {
            (status.ullTotalPhys, status.dwMemoryLoad)
        } else {
            (0, 0)
        }
    }
}

fn read_battery() -> Option<Battery> {
    // SAFETY: estructura propia que la API rellena.
    unsafe {
        let mut status: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut status) == 0 {
            return None;
        }
        // BatteryFlag: 128 = no hay batería, 255 = desconocido. BatteryLifePercent: 255 = desconocido.
        if status.BatteryFlag == 128 || status.BatteryFlag == 255 || status.BatteryLifePercent > 100 {
            return None;
        }
        Some(Battery { percent: status.BatteryLifePercent, charging: status.ACLineStatus == 1 })
    }
}

fn read_disk() -> (u64, u64) {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(std::iter::once(0)).collect();
    let (mut free, mut total, mut free_total) = (0u64, 0u64, 0u64);
    // SAFETY: `root` termina en NUL y los tres destinos son variables locales válidas.
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut free_total) };
    if ok != 0 {
        (free, total)
    } else {
        (0, 0)
    }
}

fn read_local_time() -> String {
    LocalNow::now().describe()
}

fn snapshot() -> Snapshot {
    let (memory_total, memory_used_percent) = read_memory();
    let (disk_free, disk_total) = read_disk();
    Snapshot {
        os: read_os(),
        cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        memory_total,
        memory_used_percent,
        battery: read_battery(),
        disk_free,
        disk_total,
        // SAFETY: no recibe argumentos.
        uptime_secs: unsafe { GetTickCount64() } / 1000,
        local_time: read_local_time(),
        brightness: brightness::current().ok(),
    }
}

/// Estado resumido del equipo para la pestaña «Hoy». No incluye el brillo, que es lento de leer y aquí se consulta a menudo.
pub fn system_status() -> Value {
    let (memory_total, memory_used_percent) = read_memory();
    let (disk_free, disk_total) = read_disk();
    json!({
        "memory_total_bytes": memory_total,
        "memory_used_percent": memory_used_percent,
        "disk_free_bytes": disk_free,
        "disk_total_bytes": disk_total,
        // SAFETY: no recibe argumentos.
        "uptime_seconds": unsafe { GetTickCount64() } / 1000,
        "battery": read_battery().map(|b| json!({ "percent": b.percent, "charging": b.charging })),
    })
}

pub struct SystemInfo;

impl Tool for SystemInfo {
    fn name(&self) -> &'static str {
        "system_info"
    }

    fn description(&self) -> &'static str {
        "Informa del equipo: versión de Windows, hora local, memoria, espacio libre del disco, batería, tiempo \
         encendido y brillo de la pantalla. Solo lee; no incluye nombres de usuario ni de equipo."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &[])
    }

    fn preview(&self, _args: &Value) -> String {
        "Consultar la información del equipo".into()
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        check_keys(args, &[])?;
        let snapshot = snapshot();
        Ok(ToolOutput { summary: snapshot.describe(), data: snapshot.to_json() })
    }
}

// ---------------------------------------------------------------------------------------------
// Brillo (solo pantallas internas de portátil: Windows solo lo expone por WMI)
// ---------------------------------------------------------------------------------------------

mod brightness {
    use super::*;

    use crate::powershell::{run_fixed, ScriptError};

    const POWERSHELL_TIMEOUT: Duration = Duration::from_secs(15);

    /// Los guiones son constantes del programa; lo único que se interpola es un entero ya validado.
    fn run(script: &str) -> Result<String, String> {
        run_fixed(script, POWERSHELL_TIMEOUT).map_err(|error| match error {
            ScriptError::Spawn(reason) => format!("no se pudo consultar el brillo: {reason}"),
            ScriptError::Timeout => "la consulta del brillo tardó demasiado".to_string(),
            ScriptError::Failed(_) => {
                "este equipo no permite cambiar el brillo desde aquí (solo funciona con la pantalla interna de un portátil)".to_string()
            }
        })
    }

    pub fn current() -> Result<u8, String> {
        let text = run("$b = Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction Stop | Select-Object -First 1; $b.CurrentBrightness")?;
        text.parse::<u8>().ok().filter(|level| *level <= 100).ok_or_else(|| "no se pudo leer el brillo actual".to_string())
    }

    pub fn set(level: u8) -> Result<(), String> {
        debug_assert!(level <= 100);
        run(&format!(
            "Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightnessMethods -ErrorAction Stop | Invoke-CimMethod -MethodName WmiSetBrightness -Arguments @{{Timeout=[uint32]1; Brightness=[byte]{level}}} | Out-Null"
        ))
        .map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrightnessChange {
    To(u8),
    By(i8),
}

fn parse_brightness(args: &Value) -> Result<BrightnessChange, ToolError> {
    check_keys(args, &["level", "change"])?;
    match (args.get("level").filter(|v| !v.is_null()), args.get("change").filter(|v| !v.is_null())) {
        (Some(level), None) => match level.as_i64() {
            Some(n) if (MIN_BRIGHTNESS..=100).contains(&n) => Ok(BrightnessChange::To(n as u8)),
            _ => Err(invalid(format!("«level» debe ser un entero entre {MIN_BRIGHTNESS} y 100"))),
        },
        (None, Some(change)) => match change.as_i64() {
            Some(n) if n != 0 && (-50..=50).contains(&n) => Ok(BrightnessChange::By(n as i8)),
            _ => Err(invalid("«change» debe ser un entero distinto de 0 entre -50 y 50")),
        },
        _ => Err(invalid("indica exactamente uno: «level» (nivel final) o «change» (subir o bajar)")),
    }
}

/// Nivel final, sin dejar la pantalla prácticamente apagada.
fn target_brightness(current: u8, change: BrightnessChange) -> u8 {
    match change {
        BrightnessChange::To(level) => level,
        BrightnessChange::By(delta) => (current as i16 + delta as i16).clamp(MIN_BRIGHTNESS as i16, 100) as u8,
    }
}

pub struct SetBrightness;

impl Tool for SetBrightness {
    fn name(&self) -> &'static str {
        "set_brightness"
    }

    fn description(&self) -> &'static str {
        "Cambia el brillo de la pantalla interna de un portátil. Usa «level» (10-100) para un nivel concreto o \
         «change» (de -50 a 50) para subir o bajar desde el actual. Nunca baja de 10 %."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "level": { "type": "integer", "minimum": MIN_BRIGHTNESS, "maximum": 100, "description": "Brillo final en porcentaje." },
                "change": { "type": "integer", "minimum": -50, "maximum": 50, "description": "Puntos a subir (positivo) o bajar (negativo)." }
            }
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_brightness(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_brightness(args) {
            Ok(BrightnessChange::To(level)) => format!("Poner el brillo de la pantalla al {level}%"),
            Ok(BrightnessChange::By(delta)) if delta > 0 => format!("Subir el brillo {delta} puntos"),
            Ok(BrightnessChange::By(delta)) => format!("Bajar el brillo {} puntos", -(delta as i16)),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let change = parse_brightness(args)?;
        let before = brightness::current().map_err(ToolError::Failed)?;
        let after = target_brightness(before, change);
        brightness::set(after).map_err(ToolError::Failed)?;
        Ok(ToolOutput {
            summary: format!("Brillo: {before}% → {after}%"),
            data: json!({ "before": before, "after": after }),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Portapapeles
// ---------------------------------------------------------------------------------------------

fn clipboard() -> Result<arboard::Clipboard, ToolError> {
    arboard::Clipboard::new().map_err(|error| ToolError::Failed(format!("No se pudo abrir el portapapeles: {error}")))
}

fn clipboard_text() -> Result<String, ToolError> {
    match clipboard()?.get_text() {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Err(ToolError::Failed("No hay texto en el portapapeles.".into())),
        Err(error) => Err(ToolError::Failed(format!("No se pudo leer el portapapeles: {error}"))),
    }
}

pub struct ReadClipboard;

impl Tool for ReadClipboard {
    fn name(&self) -> &'static str {
        "read_clipboard"
    }

    fn description(&self) -> &'static str {
        "Lee el texto que el usuario tiene copiado en el portapapeles (máximo 10 000 caracteres). Pide \
         confirmación porque el contenido se envía al servicio de IA."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    /// El portapapeles puede contener contraseñas o datos privados: el usuario decide cada vez.
    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Confirm
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        check_keys(args, &[])
    }

    fn preview(&self, args: &Value) -> String {
        if let Err(error) = check_keys(args, &[]) {
            return error.to_string();
        }
        match clipboard_text() {
            Ok(text) => format!(
                "Leer tu portapapeles ({} caracteres) y enviar su contenido al servicio de IA.\nRevisa que no contenga contraseñas ni datos privados.",
                text.chars().count()
            ),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        check_keys(args, &[])?;
        let text = clipboard_text()?;
        let total = text.chars().count();
        let truncated = total > MAX_CLIPBOARD_CHARS;
        let content: String = text.chars().take(MAX_CLIPBOARD_CHARS).collect();
        Ok(ToolOutput {
            summary: format!("Leí tu portapapeles ({total} caracteres{})", if truncated { ", recortado" } else { "" }),
            data: json!({ "text": content, "total_chars": total, "truncated": truncated }),
        })
    }
}

fn parse_write(args: &Value) -> Result<String, ToolError> {
    check_keys(args, &["text"])?;
    let text = crate::args::required_str(args, "text")?;
    let length = text.chars().count();
    if length == 0 || length > MAX_CLIPBOARD_WRITE_CHARS {
        return Err(invalid(format!("el texto debe tener entre 1 y {MAX_CLIPBOARD_WRITE_CHARS} caracteres")));
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(invalid("el texto contiene caracteres de control no permitidos"));
    }
    Ok(text.to_string())
}

pub struct WriteClipboard;

impl Tool for WriteClipboard {
    fn name(&self) -> &'static str {
        "write_clipboard"
    }

    fn description(&self) -> &'static str {
        "Copia un texto al portapapeles del usuario para que pueda pegarlo donde quiera. Sustituye lo que \
         hubiera copiado."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": "Texto a copiar." } },
            "required": ["text"]
        })
    }

    fn risk(&self, _args: &Value) -> RiskLevel {
        RiskLevel::Safe
    }

    fn validate(&self, args: &Value) -> Result<(), ToolError> {
        parse_write(args).map(|_| ())
    }

    fn preview(&self, args: &Value) -> String {
        match parse_write(args) {
            Ok(text) => format!("Copiar {} caracteres al portapapeles", text.chars().count()),
            Err(error) => error.to_string(),
        }
    }

    fn execute(&self, args: &Value) -> Result<ToolOutput, ToolError> {
        let text = parse_write(args)?;
        clipboard()?
            .set_text(text.clone())
            .map_err(|error| ToolError::Failed(format!("No se pudo escribir en el portapapeles: {error}")))?;
        Ok(ToolOutput {
            summary: format!("Copié {} caracteres al portapapeles", text.chars().count()),
            data: json!({ "chars": text.chars().count() }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_tools::MediaControl;

    fn sample() -> Snapshot {
        Snapshot {
            os: "Windows 11 Home 26H2 (compilación 26300)".into(),
            cores: 16,
            memory_total: 16 * 1024 * 1024 * 1024,
            memory_used_percent: 62,
            battery: Some(Battery { percent: 79, charging: false }),
            disk_free: 120 * 1024 * 1024 * 1024,
            disk_total: 475 * 1024 * 1024 * 1024,
            uptime_secs: 2 * 86_400 + 5 * 3_600 + 600,
            local_time: "lunes 2 de noviembre de 2026, 21:30".into(),
            brightness: Some(60),
        }
    }

    #[test]
    fn formatea_bytes_y_tiempos() {
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512 MB");
        assert_eq!(format_uptime(125), "2 min");
        assert_eq!(format_uptime(3_600 * 3 + 60 * 7), "3 h 7 min");
        assert_eq!(format_uptime(2 * 86_400 + 5 * 3_600 + 600), "2 d 5 h");
    }

    #[test]
    fn windows_11_se_distingue_por_la_compilacion() {
        assert_eq!(os_name("Windows 10 Home", 26_300), "Windows 11 Home");
        assert_eq!(os_name("Windows 10 Pro", 19_045), "Windows 10 Pro");
    }

    #[test]
    fn la_descripcion_incluye_lo_importante_y_nada_personal() {
        let text = sample().describe();
        for expected in ["Windows 11", "16.0 GB", "62%", "120.0 GB libres", "2 d 5 h", "Batería: 79% (con batería)", "Brillo de la pantalla: 60%", "21:30"] {
            assert!(text.contains(expected), "falta «{expected}» en:\n{text}");
        }
        let json = sample().to_json().to_string().to_lowercase();
        for private in ["username", "computername", "usuario", "andre"] {
            assert!(!json.contains(private), "no debe incluir {private}");
        }
        let desktop = Snapshot { battery: None, brightness: None, ..sample() };
        assert!(desktop.describe().contains("no tiene (equipo de escritorio)") && !desktop.describe().contains("Brillo"));
    }

    #[test]
    fn valida_el_brillo() {
        assert_eq!(parse_brightness(&json!({ "level": 60 })).unwrap(), BrightnessChange::To(60));
        assert_eq!(parse_brightness(&json!({ "change": -20 })).unwrap(), BrightnessChange::By(-20));
        for bad in [
            json!({}), json!({ "level": 5 }), json!({ "level": 101 }), json!({ "level": "60" }), json!({ "level": 50.5 }),
            json!({ "change": 0 }), json!({ "change": 51 }), json!({ "change": -51 }),
            json!({ "level": 50, "change": 5 }), json!({ "level": 50, "extra": 1 }),
        ] {
            assert!(parse_brightness(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn el_brillo_nunca_deja_la_pantalla_apagada_ni_pasa_de_100() {
        assert_eq!(target_brightness(50, BrightnessChange::To(80)), 80);
        assert_eq!(target_brightness(50, BrightnessChange::By(20)), 70);
        assert_eq!(target_brightness(15, BrightnessChange::By(-50)), MIN_BRIGHTNESS as u8);
        assert_eq!(target_brightness(90, BrightnessChange::By(50)), 100);
    }

    #[test]
    fn el_texto_del_portapapeles_se_valida() {
        assert_eq!(parse_write(&json!({ "text": "hola\nmundo\tok" })).unwrap(), "hola\nmundo\tok");
        for bad in [json!({ "text": "" }), json!({ "text": "a\u{0}b" }), json!({ "text": "x\u{1b}[31m" }), json!({}), json!({ "text": 5 }), json!({ "text": "x", "otro": 1 })] {
            assert!(parse_write(&bad).is_err(), "{bad}");
        }
        assert!(parse_write(&json!({ "text": "x".repeat(MAX_CLIPBOARD_WRITE_CHARS + 1) })).is_err());
    }

    #[test]
    fn leer_el_portapapeles_pide_confirmacion_y_el_resto_es_seguro() {
        assert_eq!(ReadClipboard.risk(&json!({})), RiskLevel::Confirm);
        for risk in [WriteClipboard.risk(&json!({})), SystemInfo.risk(&json!({})), SetBrightness.risk(&json!({}))] {
            assert_eq!(risk, RiskLevel::Safe);
        }
        assert!(SystemInfo.validate(&json!({ "algo": 1 })).is_err());
        assert!(ReadClipboard.validate(&json!({ "algo": 1 })).is_err());
    }

    /// Lecturas reales de este equipo: `cargo test -p zen-desktop -- --ignored --nocapture sistema_real`.
    #[test]
    #[ignore = "depende del equipo"]
    fn sistema_real_muestra_lo_que_lee() {
        let output = SystemInfo.execute(&json!({})).unwrap();
        println!("{}", output.summary);
        println!("{}", output.data);
    }

    /// Cambia el brillo real y lo devuelve a como estaba: `-- --ignored --nocapture brillo_real`.
    #[test]
    #[ignore = "cambia el brillo de la pantalla"]
    fn brillo_real_ida_y_vuelta() {
        let before = brightness::current().expect("este equipo debe permitir leer el brillo");
        let delta = if before <= 80 { 10 } else { -10 };
        let output = SetBrightness.execute(&json!({ "change": delta })).unwrap();
        println!("{}", output.summary);
        std::thread::sleep(Duration::from_millis(600));
        let moved = brightness::current().unwrap();
        brightness::set(before).unwrap();
        std::thread::sleep(Duration::from_millis(600));
        let restored = brightness::current().unwrap();
        println!("antes {before}, movido {moved}, restaurado {restored}");
        assert_eq!(moved as i16, before as i16 + delta as i16);
        assert_eq!(restored, before);
    }

    /// Pulsa de verdad las teclas de volumen (sube 3 pasos y los baja): `-- --ignored volumen_real`.
    #[test]
    #[ignore = "pulsa las teclas de volumen"]
    fn volumen_real_sube_y_baja() {
        MediaControl.execute(&json!({ "action": "volume_up", "steps": 3 })).unwrap();
        std::thread::sleep(Duration::from_secs(3));
        MediaControl.execute(&json!({ "action": "volume_down", "steps": 3 })).unwrap();
        std::thread::sleep(Duration::from_secs(2));
    }

    /// Escribe y lee el portapapeles real, y deja el contenido que tenía el usuario.
    #[test]
    #[ignore = "usa el portapapeles real"]
    fn portapapeles_real_ida_y_vuelta() {
        let mut board = arboard::Clipboard::new().unwrap();
        let previous = board.get_text().ok();
        WriteClipboard.execute(&json!({ "text": "prueba de Zen Pet ✔ ñ\nsegunda línea" })).unwrap();
        let read = ReadClipboard.execute(&json!({})).unwrap();
        assert_eq!(read.data["text"], "prueba de Zen Pet ✔ ñ\nsegunda línea");
        match previous {
            Some(text) => board.set_text(text).unwrap(),
            None => board.clear().unwrap(),
        }
    }
}
