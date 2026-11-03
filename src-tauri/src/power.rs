//! Vigila la batería y avisa cuando se agota o termina de cargarse.

use std::time::Duration;

use tauri::{AppHandle, Emitter};
use zen_core::events::Event;
use zen_desktop::power::{read_battery, BatteryNotice, BatteryWatcher};

use crate::settings::Settings;

const POLL: Duration = Duration::from_secs(30);

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let mut watcher = BatteryWatcher::default();
        loop {
            // Un equipo sin batería devuelve `None` siempre: el hilo duerme y no cuesta nada.
            if let Some(battery) = read_battery() {
                // Se sigue observando aunque los avisos estén apagados, para que al activarlos no salte uno viejo.
                let notice = watcher.observe(battery);
                if let (Some(notice), true) = (notice, Settings::load().battery_alerts) {
                    announce(&app, notice);
                }
            }
            std::thread::sleep(POLL);
        }
    });
}

fn announce(app: &AppHandle, notice: BatteryNotice) {
    match notice {
        BatteryNotice::Low(percent) => {
            let _ = app.emit("chat-log", format!("🔋 Batería baja: queda el {percent}%. Conecta el cargador."));
            let _ = app.emit("pet-alert-kind", "battery");
            crate::show_chat_with(app, false);
            crate::publish(app, Event::BatteryLow { percent });
        }
        BatteryNotice::Charged => {
            let _ = app.emit("chat-log", "🔌 La batería está al 100%. Ya puedes desconectar el cargador.");
            crate::publish(app, Event::BatteryCharged);
        }
    }
}
