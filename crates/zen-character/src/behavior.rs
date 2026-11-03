use std::time::{Duration, Instant};

use zen_core::events::Event;

use crate::state::{on_event, State};

const SLEEP_AFTER: Duration = Duration::from_secs(180);
const REACTION_TIME: Duration = Duration::from_millis(1800);
const INTERACTION_TIME: Duration = Duration::from_millis(1200);
/// Si nadie atiende el aviso, se calma solo (el recordatorio sigue en el chat).
const ALERT_TIME: Duration = Duration::from_secs(90);
// Red de seguridad: si nunca llega el evento de fin, la mascota no se queda pensando para siempre.
const BUSY_TIMEOUT: Duration = Duration::from_secs(60);

/// Estado del personaje más los temporizadores. No conoce la IA ni la UI: solo recibe eventos y tiempo.
pub struct Character {
    state: State,
    entered_at: Instant,
    last_activity: Instant,
}

impl Character {
    pub fn new(now: Instant) -> Self {
        Self { state: State::Idle, entered_at: now, last_activity: now }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Aplica un evento. Devuelve el nuevo estado solo si cambió.
    pub fn handle(&mut self, event: &Event, now: Instant) -> Option<State> {
        // Lo que la mascota hace por su cuenta no cuenta como actividad: si no, nunca llegaría a dormirse.
        if !matches!(event, Event::WalkStarted { .. } | Event::WalkFinished | Event::RandomAnimation) {
            self.last_activity = now;
        }
        self.set(on_event(self.state, event), now)
    }

    /// Avanza los temporizadores. Debe llamarse periódicamente.
    pub fn tick(&mut self, now: Instant) -> Option<State> {
        let in_state = now.duration_since(self.entered_at);
        let idle_for = now.duration_since(self.last_activity);
        let next = match self.state {
            State::Success | State::Happy | State::Surprised if in_state >= REACTION_TIME => State::Idle,
            State::Interacting if in_state >= INTERACTION_TIME => State::Idle,
            State::Alert if in_state >= ALERT_TIME => State::Idle,
            State::Thinking | State::Working if in_state >= BUSY_TIMEOUT => State::Idle,
            State::Idle if idle_for >= SLEEP_AFTER => State::Sleeping,
            other => other,
        };
        self.set(next, now)
    }

    fn set(&mut self, next: State, now: Instant) -> Option<State> {
        if next == self.state {
            return None;
        }
        self.state = next;
        self.entered_at = now;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> String {
        name.to_string()
    }

    #[test]
    fn flujo_normal_de_una_tarea() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        assert_eq!(c.handle(&Event::UserMessageReceived { text: "hola".into() }, t0), Some(State::Thinking));
        assert_eq!(c.handle(&Event::ToolStarted { tool: tool("x") }, t0), Some(State::Working));
        assert_eq!(c.handle(&Event::ToolCompleted { tool: tool("x") }, t0), Some(State::Success));
        assert_eq!(c.tick(t0 + REACTION_TIME), Some(State::Idle));
    }

    #[test]
    fn un_fallo_sorprende_y_luego_vuelve_a_idle() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        c.handle(&Event::ToolFailed { tool: tool("x"), error: "e".into() }, t0);
        assert_eq!(c.state(), State::Surprised);
        assert_eq!(c.tick(t0 + REACTION_TIME), Some(State::Idle));
    }

    #[test]
    fn duerme_tras_inactividad_y_despierta_al_abrir_el_chat() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        assert_eq!(c.tick(t0 + SLEEP_AFTER), Some(State::Sleeping));
        assert_eq!(c.handle(&Event::ChatOpened, t0 + SLEEP_AFTER), Some(State::Idle));
    }

    #[test]
    fn click_dormido_sorprende_y_click_despierto_interactua() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        assert_eq!(c.handle(&Event::CharacterClicked, t0), Some(State::Interacting));
        assert_eq!(c.tick(t0 + INTERACTION_TIME), Some(State::Idle));
        c.tick(t0 + INTERACTION_TIME + SLEEP_AFTER);
        assert_eq!(c.state(), State::Sleeping);
        assert_eq!(c.handle(&Event::CharacterClicked, t0 + INTERACTION_TIME + SLEEP_AFTER), Some(State::Surprised));
    }

    #[test]
    fn el_click_no_interrumpe_el_trabajo() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        c.handle(&Event::ToolStarted { tool: tool("x") }, t0);
        assert_eq!(c.handle(&Event::CharacterClicked, t0), None);
        assert_eq!(c.state(), State::Working);
    }

    #[test]
    fn cancelar_una_accion_devuelve_a_idle() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        c.handle(&Event::ToolConfirmationRequired { tool: tool("x") }, t0);
        assert_eq!(c.state(), State::Thinking);
        assert_eq!(c.handle(&Event::ToolCancelled { tool: tool("x") }, t0), Some(State::Idle));
    }

    #[test]
    fn un_paseo_empieza_desde_idle_y_termina_en_idle() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        assert_eq!(c.handle(&Event::WalkStarted { direction: 1, distance: 100 }, t0), Some(State::Walking));
        assert_eq!(c.handle(&Event::WalkFinished, t0), Some(State::Idle));
    }

    #[test]
    fn no_pasea_ni_hace_animaciones_si_esta_ocupada_o_dormida() {
        let t0 = Instant::now();
        let walk = Event::WalkStarted { direction: -1, distance: 100 };
        let mut busy = Character::new(t0);
        busy.handle(&Event::AssistantThinking, t0);
        assert_eq!(busy.handle(&walk, t0), None);
        assert_eq!(busy.handle(&Event::RandomAnimation, t0), None);
        let mut asleep = Character::new(t0);
        asleep.tick(t0 + SLEEP_AFTER);
        assert_eq!(asleep.handle(&walk, t0 + SLEEP_AFTER), None);
        assert_eq!(asleep.state(), State::Sleeping);
    }

    #[test]
    fn interrumpir_un_paseo_con_un_click_un_arrastre_o_el_chat() {
        let t0 = Instant::now();
        let walk = Event::WalkStarted { direction: 1, distance: 100 };
        for (interruption, expected) in [
            (Event::CharacterClicked, State::Interacting),
            (Event::CharacterDragged, State::Interacting),
            (Event::ChatOpened, State::Idle),
        ] {
            let mut c = Character::new(t0);
            c.handle(&walk, t0);
            assert_eq!(c.handle(&interruption, t0), Some(expected), "{interruption:?}");
        }
    }

    #[test]
    fn la_animacion_espontanea_alegra_y_vuelve_a_idle() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        assert_eq!(c.handle(&Event::RandomAnimation, t0), Some(State::Happy));
        assert_eq!(c.tick(t0 + REACTION_TIME), Some(State::Idle));
    }

    #[test]
    fn pasear_y_jugar_no_impiden_que_se_duerma() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        let mut now = t0;
        // Cada 30 s hace algo por su cuenta; aun así, al cumplirse el tiempo sin actividad real, duerme.
        while now < t0 + SLEEP_AFTER {
            now += Duration::from_secs(30);
            c.handle(&Event::WalkStarted { direction: 1, distance: 50 }, now);
            c.handle(&Event::WalkFinished, now);
            c.handle(&Event::RandomAnimation, now);
            c.tick(now + REACTION_TIME);
        }
        assert_eq!(c.tick(now + REACTION_TIME), Some(State::Sleeping));
    }

    #[test]
    fn arrastrar_a_la_mascota_dormida_la_sorprende() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        c.tick(t0 + SLEEP_AFTER);
        assert_eq!(c.handle(&Event::CharacterDragged, t0 + SLEEP_AFTER), Some(State::Surprised));
    }
    fn reminder() -> Event {
        Event::ReminderDue { text: "descanso".into() }
    }

    #[test]
    fn un_recordatorio_despierta_a_la_mascota_y_la_pone_en_alerta() {
        let t0 = Instant::now();
        for setup in [None, Some(Event::WalkStarted { direction: 1, distance: 50 }), Some(Event::RandomAnimation)] {
            let mut c = Character::new(t0);
            if let Some(event) = &setup {
                c.handle(event, t0);
            }
            assert_eq!(c.handle(&reminder(), t0), Some(State::Alert), "{setup:?}");
        }
        let mut asleep = Character::new(t0);
        asleep.tick(t0 + SLEEP_AFTER);
        assert_eq!(asleep.handle(&reminder(), t0 + SLEEP_AFTER), Some(State::Alert));
    }

    #[test]
    fn la_bateria_baja_avisa_pero_no_interrumpe_y_la_carga_completa_alegra() {
        let t0 = Instant::now();
        let low = Event::BatteryLow { percent: 15 };
        assert_eq!(Character::new(t0).handle(&low, t0), Some(State::Alert));
        let mut busy = Character::new(t0);
        busy.handle(&Event::AssistantThinking, t0);
        assert_eq!(busy.handle(&low, t0), None, "no interrumpe una tarea");
        assert_eq!(Character::new(t0).handle(&Event::BatteryCharged, t0), Some(State::Happy));
        assert_eq!(Character::new(t0).handle(&Event::RestSuggested { minutes: 90 }, t0), Some(State::Alert));
        let mut progress = Character::new(t0);
        assert_eq!(progress.handle(&Event::TaskProgress { step: 1, total: 3, tool: "x".into() }, t0), None, "el progreso no cambia el estado");
    }

    #[test]
    fn un_recordatorio_no_interrumpe_lo_que_hace_para_el_usuario() {
        let t0 = Instant::now();
        for busy in [Event::AssistantThinking, Event::ToolStarted { tool: tool("x") }] {
            let mut c = Character::new(t0);
            c.handle(&busy, t0);
            let before = c.state();
            assert_eq!(c.handle(&reminder(), t0), None);
            assert_eq!(c.state(), before);
        }
    }

    #[test]
    fn la_alerta_se_apaga_al_hacerle_caso_o_con_el_tiempo() {
        let t0 = Instant::now();
        for attention in [Event::CharacterClicked, Event::CharacterDragged, Event::ChatOpened] {
            let mut c = Character::new(t0);
            c.handle(&reminder(), t0);
            assert_eq!(c.handle(&attention, t0), Some(State::Idle), "{attention:?}");
        }
        let mut c = Character::new(t0);
        c.handle(&reminder(), t0);
        assert_eq!(c.tick(t0 + ALERT_TIME - Duration::from_secs(1)), None, "sigue avisando");
        assert_eq!(c.tick(t0 + ALERT_TIME), Some(State::Idle));
        // Y una alerta repetida no reinicia nada.
        let mut again = Character::new(t0);
        again.handle(&reminder(), t0);
        assert_eq!(again.handle(&reminder(), t0), None);
    }

    #[test]
    fn la_alerta_cuenta_como_actividad_y_no_pasea_ni_duerme() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        let t1 = t0 + SLEEP_AFTER - Duration::from_secs(5);
        c.handle(&reminder(), t1);
        c.handle(&Event::CharacterClicked, t1);
        // Tras atenderla no se duerme de inmediato: el aviso reinició el contador de inactividad.
        assert_eq!(c.tick(t1 + Duration::from_secs(30)), None);
        assert_eq!(c.handle(&Event::WalkStarted { direction: 1, distance: 50 }, t1), Some(State::Walking));
    }

    #[test]
    fn el_estado_ocupado_expira_si_no_llega_el_final() {
        let t0 = Instant::now();
        let mut c = Character::new(t0);
        c.handle(&Event::AssistantThinking, t0);
        assert_eq!(c.tick(t0 + BUSY_TIMEOUT), Some(State::Idle));
    }
}