use zen_core::events::Event;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Walking,
    Sleeping,
    Thinking,
    Working,
    Success,
    Happy,
    Surprised,
    Interacting,
    /// Avisa de algo (un recordatorio) hasta que el usuario le hace caso.
    Alert,
}

impl State {
    /// Nombre estable que usa el frontend como `data-state`.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Walking => "walking",
            State::Sleeping => "sleeping",
            State::Thinking => "thinking",
            State::Working => "working",
            State::Success => "success",
            State::Happy => "happy",
            State::Surprised => "surprised",
            State::Interacting => "interacting",
            State::Alert => "alert",
        }
    }
}

/// Transición provocada por un evento. Lo que no está listado no cambia el estado.
pub fn on_event(state: State, event: &Event) -> State {
    use Event::*;
    use State::*;
    match (state, event) {
        // Hacerle caso (click, arrastre o abrir el chat) apaga el aviso.
        (Alert, CharacterClicked | CharacterDragged | ChatOpened) => Idle,
        (Thinking | Working | Alert, BatteryLow { .. } | RestSuggested { .. }) => state,
        (_, BatteryLow { .. } | RestSuggested { .. }) => Alert,
        (Idle | Walking | Sleeping, BatteryCharged | DownloadFinished { .. }) => Happy,
        (Idle | Walking | Sleeping, UsbConnected { .. }) => Surprised,
        // El aviso previo de un recordatorio también llama la atención, pero sin interrumpir lo que haces.
        (Thinking | Working | Alert, ReminderSoon { .. }) => state,
        (_, ReminderSoon { .. }) => Alert,
        // Un aviso no interrumpe lo que está haciendo para el usuario; sí despierta o detiene el resto.
        (Thinking | Working | Alert, ReminderDue { .. }) => state,
        (_, ReminderDue { .. }) => Alert,
        (Sleeping, CharacterClicked | CharacterDragged) => Surprised,
        (Thinking | Working, CharacterClicked | CharacterDragged) => state,
        (_, CharacterClicked | CharacterDragged) => Interacting,
        (Sleeping | Walking, ChatOpened) => Idle,
        // Los paseos y las animaciones espontáneas solo empiezan desde el reposo.
        (Idle, WalkStarted { .. }) => Walking,
        (Walking, WalkFinished) => Idle,
        (Idle, RandomAnimation) => Happy,
        (
            _,
            UserMessageReceived { .. }
            | AssistantThinking
            | ToolRequested { .. }
            | ToolConfirmationRequired { .. },
        ) => Thinking,
        (_, ToolStarted { .. }) => Working,
        (_, ToolCompleted { .. }) => Success,
        (_, ToolFailed { .. } | AssistantFailed { .. }) => Surprised,
        (_, ToolCancelled { .. }) => Idle,
        (Thinking, AssistantReplied) => Happy,
        _ => state,
    }
}