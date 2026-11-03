use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

/// Hechos que ocurren en la aplicación. Los módulos solo se comunican a través de ellos.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    CharacterClicked,
    /// El usuario empezó a arrastrar la mascota.
    CharacterDragged,
    /// La mascota decide pasear: dirección (-1 izquierda, 1 derecha) y distancia en píxeles.
    WalkStarted { direction: i32, distance: u32 },
    WalkFinished,
    /// Una animación espontánea (por ejemplo, un saltito).
    RandomAnimation,
    /// Venció un recordatorio.
    ReminderDue { text: String },
    /// La batería se está agotando (no está enchufada): porcentaje restante.
    BatteryLow { percent: u8 },
    /// Llevas mucho tiempo seguido ante el equipo: conviene descansar.
    RestSuggested { minutes: u64 },
    /// El portátil enchufado llegó a la carga completa.
    BatteryCharged,
    /// Terminó de descargarse un archivo en la carpeta Descargas.
    DownloadFinished { name: String },
    /// Se conectó una unidad extraíble (memoria USB, lector de tarjetas): su letra.
    UsbConnected { drive: char },
    /// Empezó a sonar música (o un vídeo) en el equipo, en cualquier aplicación.
    MusicStarted,
    /// Dejó de sonar todo lo que sonaba.
    MusicStopped,
    /// Un recordatorio vence en pocos minutos (por ejemplo, una reunión).
    ReminderSoon { text: String, minutes: u64 },
    /// Avance de una tarea de varios pasos: se va a ejecutar el paso `step` (desde 1) de `total`.
    TaskProgress { step: usize, total: usize, tool: String },
    ChatOpened,
    ChatClosed,
    UserMessageReceived { text: String },
    AssistantThinking,
    AssistantReplied,
    ToolRequested { tool: String },
    ToolConfirmationRequired { tool: String },
    ToolStarted { tool: String },
    ToolCompleted { tool: String },
    ToolFailed { tool: String, error: String },
    ToolCancelled { tool: String },
    AssistantFailed { error: String },
}

/// Bus de difusión: cada suscriptor recibe una copia de cada evento publicado.
#[derive(Default)]
pub struct EventBus {
    subscribers: Mutex<Vec<Sender<Event>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self) -> Receiver<Event> {
        let (tx, rx) = channel();
        self.subscribers.lock().expect("bus envenenado").push(tx);
        rx
    }

    pub fn publish(&self, event: Event) {
        // Los suscriptores cuyo receptor ya no existe se descartan.
        self.subscribers
            .lock()
            .expect("bus envenenado")
            .retain(|tx| tx.send(event.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todos_los_suscriptores_reciben_el_evento() {
        let bus = EventBus::new();
        let a = bus.subscribe();
        let b = bus.subscribe();
        bus.publish(Event::ChatOpened);
        assert_eq!(a.try_recv(), Ok(Event::ChatOpened));
        assert_eq!(b.try_recv(), Ok(Event::ChatOpened));
    }

    #[test]
    fn un_suscriptor_cerrado_no_bloquea_a_los_demas() {
        let bus = EventBus::new();
        drop(bus.subscribe());
        let vivo = bus.subscribe();
        bus.publish(Event::CharacterClicked);
        assert_eq!(vivo.try_recv(), Ok(Event::CharacterClicked));
    }
}