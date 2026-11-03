//! Decide cuándo «está sonando música» a partir de lecturas sueltas de los controles multimedia de Windows
//! (Spotify, Apple Music, pestañas del navegador, VLC…). Un cambio de pista pasa por un instante sin sonido,
//! así que se exigen varias lecturas seguidas para dar por empezada o acabada la música.

use crate::media_session::{self, PlayState};

/// Lecturas seguidas con sonido para dar la música por empezada…
const START_AFTER: u8 = 2;
/// …y sin sonido para darla por terminada (más, para no soltar los auriculares entre dos canciones).
const STOP_AFTER: u8 = 4;

#[derive(Debug, Default)]
pub struct MusicDetector {
    playing: bool,
    streak: u8,
}

impl MusicDetector {
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Una lectura nueva. Devuelve `Some(true/false)` solo cuando cambia el estado.
    pub fn observe(&mut self, heard: bool) -> Option<bool> {
        if heard == self.playing {
            self.streak = 0;
            return None;
        }
        self.streak += 1;
        let needed = if heard { START_AFTER } else { STOP_AFTER };
        if self.streak >= needed {
            self.playing = heard;
            self.streak = 0;
            return Some(heard);
        }
        None
    }
}

/// ¿Hay algo reproduciéndose ahora mismo en cualquier aplicación? Sin sesiones o con error, no.
pub fn something_playing() -> bool {
    media_session::all_playing().map(|sessions| sessions.iter().any(|now| now.state == PlayState::Playing)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empieza_tras_dos_lecturas_con_sonido() {
        let mut detector = MusicDetector::default();
        assert_eq!(detector.observe(true), None);
        assert_eq!(detector.observe(true), Some(true));
        assert!(detector.is_playing());
        assert_eq!(detector.observe(true), None, "no repite");
    }

    #[test]
    fn un_cambio_de_pista_no_cuenta_como_parada() {
        let mut detector = MusicDetector::default();
        detector.observe(true);
        detector.observe(true);
        for _ in 0..(STOP_AFTER - 1) {
            assert_eq!(detector.observe(false), None);
        }
        assert_eq!(detector.observe(true), None, "volvió el sonido a tiempo");
        assert!(detector.is_playing());
        for _ in 0..(STOP_AFTER - 1) {
            assert_eq!(detector.observe(false), None, "la racha de silencio empieza de cero");
        }
    }

    #[test]
    fn termina_tras_varias_lecturas_en_silencio() {
        let mut detector = MusicDetector::default();
        detector.observe(true);
        detector.observe(true);
        for _ in 0..(STOP_AFTER - 1) {
            assert_eq!(detector.observe(false), None);
        }
        assert_eq!(detector.observe(false), Some(false));
        assert!(!detector.is_playing());
    }

    #[test]
    fn un_blip_de_sonido_no_activa_nada() {
        let mut detector = MusicDetector::default();
        assert_eq!(detector.observe(true), None);
        assert_eq!(detector.observe(false), None);
        assert_eq!(detector.observe(true), None);
        assert!(!detector.is_playing());
    }

    #[test]
    fn consultar_los_controles_no_falla() {
        let _ = something_playing();
    }
}
