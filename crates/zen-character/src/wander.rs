use std::time::{Duration, Instant};

use zen_core::events::Event;

use crate::state::State;

const MIN_GAP_SECS: u32 = 15;
const MAX_GAP_SECS: u32 = 45;
const MIN_DISTANCE: u32 = 60;
const MAX_DISTANCE: u32 = 260;
/// Probabilidad (en %) de que lo espontáneo sea una animación en vez de un paseo.
const PLAY_CHANCE: u32 = 25;

/// Decide cuándo y qué hace la mascota por iniciativa propia. Solo produce eventos; mover la
/// ventana o leer el cursor corresponde a la capa de escritorio. No sabe nada de la IA.
pub struct Wander {
    rng: u64,
    next_at: Instant,
}

impl Wander {
    pub fn new(now: Instant, seed: u64) -> Self {
        let mut wander = Self { rng: seed | 1, next_at: now };
        wander.schedule(now);
        wander
    }

    /// `enabled` es falso si el usuario lo desactivó o hay algo que lo desaconseja (el chat abierto).
    pub fn poll(&mut self, state: State, now: Instant, enabled: bool) -> Option<Event> {
        if now < self.next_at {
            return None;
        }
        // Si no era buen momento, se vuelve a intentar más tarde en vez de actuar nada más recuperarlo.
        self.schedule(now);
        if !enabled || state != State::Idle {
            return None;
        }
        if self.between(0, 99) < PLAY_CHANCE {
            return Some(Event::RandomAnimation);
        }
        let direction = if self.between(0, 1) == 0 { -1 } else { 1 };
        Some(Event::WalkStarted { direction, distance: self.between(MIN_DISTANCE, MAX_DISTANCE) })
    }

    fn schedule(&mut self, now: Instant) {
        self.next_at = now + Duration::from_secs(self.between(MIN_GAP_SECS, MAX_GAP_SECS) as u64);
    }

    fn between(&mut self, low: u32, high: u32) -> u32 {
        low + self.next_u32() % (high - low + 1)
    }

    /// xorshift64*: suficiente para variar el comportamiento, sin dependencias.
    fn next_u32(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64, polls: usize) -> Vec<Event> {
        let t0 = Instant::now();
        let mut wander = Wander::new(t0, seed);
        (1..=polls).filter_map(|i| wander.poll(State::Idle, t0 + Duration::from_secs(60 * i as u64), true)).collect()
    }

    #[test]
    fn no_hace_nada_antes_de_tiempo() {
        let t0 = Instant::now();
        let mut wander = Wander::new(t0, 7);
        assert_eq!(wander.poll(State::Idle, t0, true), None);
        assert_eq!(wander.poll(State::Idle, t0 + Duration::from_secs(MIN_GAP_SECS as u64 - 1), true), None);
    }

    #[test]
    fn solo_actua_cuando_esta_en_reposo_y_habilitada() {
        let t0 = Instant::now();
        let late = t0 + Duration::from_secs(MAX_GAP_SECS as u64 + 1);
        for state in [State::Thinking, State::Working, State::Sleeping, State::Walking, State::Happy] {
            assert_eq!(Wander::new(t0, 7).poll(state, late, true), None, "{state:?}");
        }
        assert_eq!(Wander::new(t0, 7).poll(State::Idle, late, false), None);
        assert!(Wander::new(t0, 7).poll(State::Idle, late, true).is_some());
    }

    #[test]
    fn tras_un_momento_inoportuno_no_actua_de_golpe() {
        let t0 = Instant::now();
        let mut wander = Wander::new(t0, 7);
        let late = t0 + Duration::from_secs(MAX_GAP_SECS as u64 + 1);
        assert_eq!(wander.poll(State::Thinking, late, true), None);
        assert_eq!(wander.poll(State::Idle, late, true), None, "se reprograma en lugar de actuar de inmediato");
    }

    #[test]
    fn los_valores_siempre_estan_en_rango_y_hay_variedad() {
        let events = run(12345, 600);
        let (mut walks, mut plays, mut left, mut right) = (0, 0, 0, 0);
        for event in &events {
            match event {
                Event::WalkStarted { direction, distance } => {
                    walks += 1;
                    assert!((MIN_DISTANCE..=MAX_DISTANCE).contains(distance), "{distance}");
                    match direction {
                        -1 => left += 1,
                        1 => right += 1,
                        other => panic!("dirección inválida {other}"),
                    }
                }
                Event::RandomAnimation => plays += 1,
                other => panic!("evento inesperado {other:?}"),
            }
        }
        assert!(walks > plays && plays > 0, "más paseos que animaciones, pero de ambos ({walks} / {plays})");
        assert!(left > 0 && right > 0, "va en las dos direcciones ({left} / {right})");
    }

    #[test]
    fn es_reproducible_con_la_misma_semilla_y_distinta_con_otra() {
        assert_eq!(run(99, 100), run(99, 100));
        assert_ne!(run(99, 100), run(100, 100));
    }
}