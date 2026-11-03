//! Aviso de recordatorios: un hilo comprueba cada segundo si venció alguno y, si es así, la mascota
//! avisa (mensaje en el chat y estado de alerta; el sonido lo pone `notify`).

use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use zen_core::events::Event;
use zen_desktop::{unix_now, Reminders};

/// Un recordatorio que venció hace más de esto (por ejemplo, con la aplicación cerrada) se avisa como atrasado.
const LATE_AFTER_SECS: u64 = 60;

pub fn spawn(app: AppHandle, reminders: Arc<Reminders>) {
    std::thread::spawn(move || loop {
        let now = unix_now();
        for reminder in reminders.take_due(now) {
            notify(&app, &reminder.text, now.saturating_sub(reminder.due));
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

fn notify(app: &AppHandle, text: &str, late_secs: u64) {
    let text = if late_secs > LATE_AFTER_SECS {
        format!("{text} (venció hace {})", zen_desktop::describe_span(late_secs))
    } else {
        text.to_string()
    };
    #[cfg(debug_assertions)]
    println!("recordatorio: {text}");
    // Primero el mensaje y el chat; después el evento, para que abrir el chat no apague la alerta recién creada.
    let _ = app.emit("reminder", &text);
    crate::show_chat_with(app, false);
    crate::publish(app, Event::ReminderDue { text });
}
