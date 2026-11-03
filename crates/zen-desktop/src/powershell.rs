//! Ejecución de guiones FIJOS de PowerShell. Es la única puerta a PowerShell del programa y solo
//! admite constantes del propio código: lo único que se interpola alguna vez es un número ya validado.
//! Ningún texto que venga de la IA o del usuario llega nunca aquí.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// No se pudo lanzar PowerShell.
    Spawn(String),
    /// Tardó más de lo permitido y se cerró.
    Timeout,
    /// Terminó con error (por ejemplo, la clase WMI no existe en este equipo).
    Failed(String),
}

fn powershell() -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
        .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe")
}

/// Ejecuta `script` y devuelve su salida estándar, ya como texto UTF-8 y sin espacios sobrantes.
pub(crate) fn run_fixed(script: &str, timeout: Duration) -> Result<String, ScriptError> {
    // La salida se fuerza a UTF-8 para conservar tildes y eñes.
    let script = format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {script}");
    let mut child = Command::new(powershell())
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ScriptError::Spawn(error.to_string()))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                return Err(ScriptError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(error) => return Err(ScriptError::Failed(error.to_string())),
        }
    }
    let output = child.wait_with_output().map_err(|error| ScriptError::Failed(error.to_string()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}').to_string())
    } else {
        Err(ScriptError::Failed(String::from_utf8_lossy(&output.stderr).trim().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devuelve_la_salida_en_utf8() {
        assert_eq!(run_fixed("Write-Output 'Calculadora ñandú ✔'", Duration::from_secs(20)).unwrap(), "Calculadora ñandú ✔");
    }

    #[test]
    fn distingue_errores_de_tiempo_agotado() {
        assert!(matches!(run_fixed("throw 'falla'", Duration::from_secs(20)), Err(ScriptError::Failed(_))));
        assert_eq!(run_fixed("Start-Sleep -Seconds 30", Duration::from_millis(600)), Err(ScriptError::Timeout));
    }
}
