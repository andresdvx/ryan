//! Hora local y aviso sonoro. El resto del programa no usa el reloj directamente: así los cálculos
//! de recordatorios se pueden probar con horas inventadas.

use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;

const DAYS: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];
const MONTHS: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];
const DAY_SECS: u64 = 86_400;

/// Fecha y hora local del usuario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalNow {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// 0 = domingo.
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl LocalNow {
    pub fn now() -> Self {
        // SAFETY: estructura propia que la API rellena.
        let time = unsafe {
            let mut time: SYSTEMTIME = std::mem::zeroed();
            GetLocalTime(&mut time);
            time
        };
        Self {
            year: time.wYear,
            month: time.wMonth as u8,
            day: time.wDay as u8,
            weekday: time.wDayOfWeek as u8,
            hour: time.wHour as u8,
            minute: time.wMinute as u8,
            second: time.wSecond as u8,
        }
    }

    pub fn seconds_of_day(&self) -> u64 {
        self.hour as u64 * 3_600 + self.minute as u64 * 60 + self.second as u64
    }

    /// «domingo 1 de noviembre de 2026, 22:40»
    pub fn describe(&self) -> String {
        format!(
            "{} {} de {} de {}, {:02}:{:02}",
            DAYS[self.weekday as usize % 7],
            self.day,
            MONTHS[(self.month as usize).saturating_sub(1) % 12],
            self.year,
            self.hour,
            self.minute
        )
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Segundos hasta la próxima vez que el reloj marque `hour:minute` (siempre al menos 1, como mucho 24 h).
pub fn seconds_until(now: &LocalNow, hour: u8, minute: u8) -> u64 {
    let target = hour as u64 * 3_600 + minute as u64 * 60;
    let current = now.seconds_of_day();
    if target > current {
        target - current
    } else {
        target + DAY_SECS - current
    }
}

/// Una duración legible: «45 s», «15 min», «2 h 5 min», «3 d 1 h».
pub fn describe_span(secs: u64) -> String {
    match secs {
        0..=89 => format!("{secs} s"),
        90..=3_599 => format!("{} min", (secs + 30) / 60),
        3_600..=86_399 => {
            let (hours, minutes) = (secs / 3_600, secs % 3_600 / 60);
            if minutes == 0 {
                format!("{hours} h")
            } else {
                format!("{hours} h {minutes} min")
            }
        }
        _ => format!("{} d {} h", secs / DAY_SECS, secs % DAY_SECS / 3_600),
    }
}

fn format_delay(secs: u64) -> String {
    format!("en {}", describe_span(secs))
}

/// «a las 22:55 (en 15 min)», «mañana a las 08:00 (en 9 h 20 min)», «el miércoles a las 08:00 (…)».
pub fn describe_due(now: &LocalNow, delay_secs: u64) -> String {
    let from_midnight = now.seconds_of_day() + delay_secs;
    let days_ahead = from_midnight / DAY_SECS;
    let clock = from_midnight % DAY_SECS;
    let time = format!("{:02}:{:02}", clock / 3_600, clock % 3_600 / 60);
    let when = match days_ahead {
        0 => format!("a las {time}"),
        1 => format!("mañana a las {time}"),
        n => format!("el {} a las {time}", DAYS[((now.weekday as u64 + n) % 7) as usize]),
    };
    format!("{when} ({})", format_delay(delay_secs))
}

/// El sonido de aviso del esquema de sonidos de Windows del usuario.
pub fn play_alert_sound() {
    // SAFETY: `MessageBeep` solo recibe un entero.
    unsafe {
        MessageBeep(MB_ICONASTERISK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u8, minute: u8, second: u8) -> LocalNow {
        // 2026-11-01 fue domingo.
        LocalNow { year: 2026, month: 11, day: 1, weekday: 0, hour, minute, second }
    }

    #[test]
    fn describe_la_fecha_en_espanol() {
        assert_eq!(at(22, 40, 5).describe(), "domingo 1 de noviembre de 2026, 22:40");
        let monday = LocalNow { weekday: 1, day: 2, month: 3, hour: 7, minute: 5, ..at(0, 0, 0) };
        assert_eq!(monday.describe(), "lunes 2 de marzo de 2026, 07:05");
    }

    #[test]
    fn calcula_los_segundos_hasta_una_hora_del_reloj() {
        assert_eq!(seconds_until(&at(22, 40, 0), 22, 55), 15 * 60);
        assert_eq!(seconds_until(&at(22, 40, 30), 22, 41), 30);
        assert_eq!(seconds_until(&at(22, 40, 0), 8, 0), 9 * 3_600 + 20 * 60, "pasa a la mañana siguiente");
        assert_eq!(seconds_until(&at(22, 40, 30), 22, 40), DAY_SECS - 30, "ya pasó este minuto: mañana");
        assert_eq!(seconds_until(&at(22, 40, 0), 22, 40), DAY_SECS, "justo ahora cuenta como mañana");
        assert_eq!(seconds_until(&at(0, 0, 0), 23, 59), 23 * 3_600 + 59 * 60);
        for (h, m, s) in [(0, 0, 0), (12, 30, 30), (23, 59, 59)] {
            for (th, tm) in [(0, 0), (6, 15), (23, 59)] {
                let wait = seconds_until(&at(h, m, s), th, tm);
                assert!((1..=DAY_SECS).contains(&wait), "{h}:{m}:{s} -> {th}:{tm} = {wait}");
            }
        }
    }

    #[test]
    fn describe_cuando_vence() {
        assert_eq!(describe_due(&at(22, 40, 0), 15 * 60), "a las 22:55 (en 15 min)");
        assert_eq!(describe_due(&at(22, 40, 0), 45), "a las 22:40 (en 45 s)");
        assert_eq!(describe_due(&at(22, 40, 0), 9 * 3_600 + 20 * 60), "mañana a las 08:00 (en 9 h 20 min)");
        assert_eq!(describe_due(&at(10, 0, 0), 2 * 3_600), "a las 12:00 (en 2 h)");
        assert_eq!(describe_due(&at(22, 0, 0), 3 * DAY_SECS), "el miércoles a las 22:00 (en 3 d 0 h)");
    }

    #[test]
    fn describe_duraciones() {
        assert_eq!(describe_span(0), "0 s");
        assert_eq!(describe_span(89), "89 s");
        assert_eq!(describe_span(90), "2 min");
        assert_eq!(describe_span(2 * 3_600 + 5 * 60), "2 h 5 min");
        assert_eq!(describe_span(3 * DAY_SECS + 3_600), "3 d 1 h");
    }

    #[test]
    fn el_reloj_real_es_coherente() {
        let now = LocalNow::now();
        assert!((2024..2100).contains(&now.year) && (1..=12).contains(&now.month) && (1..=31).contains(&now.day));
        assert!(now.hour < 24 && now.minute < 60 && now.second < 61 && now.weekday < 7);
        assert!(unix_now() > 1_700_000_000);
    }
}
