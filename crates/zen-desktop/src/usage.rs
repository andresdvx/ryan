//! Cómo usas el equipo: cuánto llevas sin parar, cuándo vuelves de una ausencia y cuánto hace que no juegas con la
//! mascota. Solo mide el tiempo desde la última pulsación o movimiento del ratón (nunca qué se pulsa ni dónde);
//! la decisión de cuándo comentar algo (`UsageTracker`) es lógica pura y se prueba con tiempos inventados.

use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

/// Una pausa de al menos esto corta la racha de trabajo.
pub const BREAK_IDLE_SECS: u64 = 5 * 60;
/// Se sugiere descansar tras tanto tiempo seguido, y de nuevo cada tanto si sigues.
pub const REST_AFTER_SECS: u64 = 90 * 60;
pub const REST_REPEAT_SECS: u64 = 60 * 60;
/// Una ausencia de al menos esto se saluda al volver.
pub const AWAY_SECS: u64 = 30 * 60;
/// Con el equipo en uso y sin tocar a la mascota durante tanto, se aburre (como mucho una vez cada tanto).
pub const BORED_AFTER_SECS: u64 = 75 * 60;
pub const BORED_REPEAT_SECS: u64 = 3 * 60 * 60;
/// Por debajo de esto se considera que estás usando el equipo ahora mismo.
const ACTIVE_IDLE_SECS: u64 = 60;

/// Segundos desde la última pulsación de tecla o movimiento de ratón.
pub fn idle_seconds() -> u64 {
    // SAFETY: estructura propia con su tamaño indicado; GetTickCount no tiene requisitos.
    unsafe {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        // Ambos valores son milisegundos de 32 bits que dan la vuelta cada ~49 días: la resta con wrap es correcta.
        (GetTickCount().wrapping_sub(info.dwTime) / 1000) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageNotice {
    /// Llevas `minutes` minutos seguidos sin una pausa.
    TakeBreak { minutes: u64 },
    /// Vuelves tras `away_minutes` minutos fuera.
    WelcomeBack { away_minutes: u64 },
    /// Llevas un buen rato en el equipo sin hacerle caso a la mascota.
    Bored,
}

#[derive(Debug, Default)]
pub struct UsageTracker {
    streak_start: Option<u64>,
    last_rest_notice: Option<u64>,
    longest_idle: u64,
    last_interaction: Option<u64>,
    last_bored_notice: Option<u64>,
    started: Option<u64>,
}

impl UsageTracker {
    /// El usuario hizo algo con la mascota (clic, abrir el chat, escribirle).
    pub fn note_interaction(&mut self, now: u64) {
        self.last_interaction = Some(now);
    }

    /// Llamar cada poco con los segundos de inactividad y el reloj (en segundos, solo creciente).
    pub fn observe(&mut self, idle_secs: u64, now: u64) -> Option<UsageNotice> {
        let started = *self.started.get_or_insert(now);
        self.last_interaction.get_or_insert(started);

        if idle_secs >= BREAK_IDLE_SECS {
            // Pausa: se corta la racha y se recuerda lo larga que es la ausencia.
            self.streak_start = None;
            self.last_rest_notice = None;
            self.longest_idle = self.longest_idle.max(idle_secs);
            return None;
        }

        let mut notice = None;
        if idle_secs < ACTIVE_IDLE_SECS && self.longest_idle >= AWAY_SECS {
            notice = Some(UsageNotice::WelcomeBack { away_minutes: self.longest_idle / 60 });
            // Tras una ausencia larga empieza todo de cero: ni descanso ni aburrimiento inmediatos.
            self.last_interaction = Some(now);
        }
        if idle_secs < ACTIVE_IDLE_SECS {
            self.longest_idle = 0;
        }
        if notice.is_some() {
            self.streak_start = Some(now);
            return notice;
        }

        let streak = *self.streak_start.get_or_insert(now);
        let worked = now.saturating_sub(streak);
        if worked >= REST_AFTER_SECS && self.last_rest_notice.is_none_or(|at| now.saturating_sub(at) >= REST_REPEAT_SECS) {
            self.last_rest_notice = Some(now);
            return Some(UsageNotice::TakeBreak { minutes: worked / 60 });
        }

        let since_pet = now.saturating_sub(self.last_interaction.unwrap_or(now));
        if since_pet >= BORED_AFTER_SECS && self.last_bored_notice.is_none_or(|at| now.saturating_sub(at) >= BORED_REPEAT_SECS) {
            self.last_bored_notice = Some(now);
            // Quedarse sin hacer caso no es motivo para repetirlo enseguida.
            self.last_interaction = Some(now);
            return Some(UsageNotice::Bored);
        }
        None
    }
}

/// «1 h 30 min», «45 min».
pub fn describe_minutes(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60;

    /// Simula `minutes` minutos de uso continuo, comprobando cada 30 s; devuelve los avisos con su minuto.
    fn work(tracker: &mut UsageTracker, from_min: u64, minutes: u64, pet_every: Option<u64>) -> Vec<(u64, UsageNotice)> {
        let mut notices = Vec::new();
        for step in 0..minutes * 2 {
            let now = from_min * MIN + step * 30;
            if let Some(every) = pet_every {
                if step % (every * 2) == 0 {
                    tracker.note_interaction(now);
                }
            }
            if let Some(notice) = tracker.observe(5, now) {
                notices.push((now / MIN, notice));
            }
        }
        notices
    }

    #[test]
    fn sugiere_descansar_a_la_hora_y_media_y_luego_cada_hora() {
        let mut tracker = UsageTracker::default();
        let notices = work(&mut tracker, 0, 4 * 60, Some(20));
        let rests: Vec<(u64, UsageNotice)> = notices.into_iter().filter(|(_, n)| matches!(n, UsageNotice::TakeBreak { .. })).collect();
        assert_eq!(rests.len(), 3, "{rests:?}");
        assert_eq!(rests[0], (90, UsageNotice::TakeBreak { minutes: 90 }));
        assert_eq!(rests[1].0, 150);
        assert_eq!(rests[2].0, 210);
    }

    #[test]
    fn una_pausa_de_cinco_minutos_reinicia_la_racha() {
        let mut tracker = UsageTracker::default();
        assert!(work(&mut tracker, 0, 80, Some(20)).is_empty());
        // 6 minutos sin tocar nada.
        for step in 0..12 {
            assert_eq!(tracker.observe(6 * MIN, 80 * MIN + step * 30), None);
        }
        // Otros 80 minutos de trabajo: en total más de 90, pero no seguidos.
        assert!(work(&mut tracker, 86, 80, Some(20)).is_empty());
    }

    #[test]
    fn saluda_al_volver_de_una_ausencia_larga_pero_no_de_una_corta() {
        let mut tracker = UsageTracker::default();
        work(&mut tracker, 0, 10, Some(5));
        // Ausencia de 45 minutos.
        assert_eq!(tracker.observe(10 * MIN, 20 * MIN), None);
        assert_eq!(tracker.observe(45 * MIN, 55 * MIN), None);
        assert_eq!(tracker.observe(3, 55 * MIN + 30), Some(UsageNotice::WelcomeBack { away_minutes: 45 }));
        assert_eq!(tracker.observe(4, 56 * MIN), None, "solo una vez");
        // Ausencia corta: sin saludo.
        let mut short = UsageTracker::default();
        short.observe(5, 0);
        assert_eq!(short.observe(10 * MIN, 12 * MIN), None);
        assert_eq!(short.observe(2, 13 * MIN), None);
    }

    #[test]
    fn se_aburre_si_no_le_haces_caso_pero_no_se_repite_enseguida() {
        let mut tracker = UsageTracker::default();
        let notices = work(&mut tracker, 0, 74, None);
        assert!(notices.iter().all(|(_, n)| !matches!(n, UsageNotice::Bored)), "aún no");
        let notices = work(&mut tracker, 74, 6, None);
        assert_eq!(notices.iter().filter(|(_, n)| *n == UsageNotice::Bored).count(), 1, "{notices:?}");
        let later = work(&mut tracker, 80, 120, None);
        assert!(later.iter().all(|(_, n)| !matches!(n, UsageNotice::Bored)), "no insiste en las dos horas siguientes: {later:?}");
    }

    #[test]
    fn jugar_con_ella_evita_que_se_aburra() {
        let mut tracker = UsageTracker::default();
        let notices = work(&mut tracker, 0, 80, Some(30));
        assert!(notices.iter().all(|(_, n)| !matches!(n, UsageNotice::Bored)), "{notices:?}");
    }

    #[test]
    fn describe_los_minutos() {
        assert_eq!((describe_minutes(45), describe_minutes(60), describe_minutes(90), describe_minutes(125)), ("45 min".into(), "1 h".into(), "1 h 30 min".into(), "2 h 5 min".into()));
    }

    #[test]
    fn la_lectura_real_de_inactividad_no_falla() {
        // Cualquier valor es válido; lo importante es que no entre en pánico ni devuelva algo absurdo (más de un año).
        assert!(idle_seconds() < 365 * 24 * 3600);
    }
}
