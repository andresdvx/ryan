//! Ánimo del asistente: según la hora, la batería y cuánto llevas trabajando, ajusta el tono con el que habla.
//! Es solo una indicación de estilo para la IA (no cambia lo que hace ni sus reglas) y es lógica pura:
//! se prueba con datos inventados. De lo que haces solo se usa la duración de la racha, nunca el contenido.

use crate::power::Battery;

/// Por debajo de este porcentaje (sin enchufar) el asistente se vuelve más escueto.
const LOW_BATTERY_PERCENT: u8 = 20;
/// Tras tanto trabajo seguido se preocupa por ti; antes de eso solo va directo al grano.
const BUSY_MINUTES: u64 = 45;
const TIRED_MINUTES: u64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoodInput {
    /// Hora local, de 0 a 23.
    pub hour: u8,
    pub battery: Option<Battery>,
    /// Minutos seguidos sin una pausa (0 si no se sabe).
    pub worked_minutes: u64,
}

fn time_of_day(hour: u8) -> &'static str {
    match hour {
        0..=4 => "Es de madrugada: habla bajito y con calma; si el usuario lleva rato despierto, anímalo con cariño a descansar.",
        5..=11 => "Es por la mañana: tono fresco y animado.",
        12..=17 => "Es por la tarde: tono amable y cercano.",
        _ => "Es de noche: tono tranquilo y relajado.",
    }
}

/// Texto para añadir al contexto de la IA; nunca vacío.
pub fn mood_section(input: MoodInput) -> String {
    let mut lines = vec![time_of_day(input.hour).to_string()];
    if let Some(battery) = input.battery.filter(|b| !b.plugged_in && b.percent <= LOW_BATTERY_PERCENT) {
        lines.push(format!(
            "La batería del equipo está baja ({}%): sé aún más breve y, si encaja, recuérdale conectar el cargador sin insistir.",
            battery.percent
        ));
    }
    if input.worked_minutes >= TIRED_MINUTES {
        lines.push(format!(
            "Lleva {} sin pausa: muestra una preocupación cariñosa y sugiere un descanso, sin sermonear ni repetirlo si ya lo dijiste.",
            crate::usage::describe_minutes(input.worked_minutes)
        ));
    } else if input.worked_minutes >= BUSY_MINUTES {
        lines.push("Lleva un buen rato trabajando: sé eficiente y ve directo al grano.".to_string());
    }
    format!(
        "Ánimo del momento (solo cambia tu tono; nunca lo que haces, tus reglas ni la veracidad de lo que cuentas):\n- {}",
        lines.join("\n- ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(hour: u8, battery: Option<Battery>, worked_minutes: u64) -> MoodInput {
        MoodInput { hour, battery, worked_minutes }
    }

    #[test]
    fn el_tono_cambia_con_la_hora() {
        let tone = |hour| mood_section(input(hour, None, 0));
        assert!(tone(3).contains("madrugada"));
        assert!(tone(8).contains("mañana"));
        assert!(tone(15).contains("tarde"));
        assert!(tone(22).contains("noche"));
        assert!(tone(0).contains("madrugada") && tone(23).contains("noche") && tone(5).contains("mañana") && tone(12).contains("tarde"));
    }

    #[test]
    fn la_bateria_baja_solo_importa_sin_enchufar() {
        let low = Battery { percent: 12, plugged_in: false };
        assert!(mood_section(input(10, Some(low), 0)).contains("12%"));
        assert!(!mood_section(input(10, Some(Battery { percent: 12, plugged_in: true }), 0)).contains("batería"));
        assert!(!mood_section(input(10, Some(Battery { percent: 60, plugged_in: false }), 0)).contains("batería"));
        assert!(!mood_section(input(10, None, 0)).contains("batería"));
    }

    #[test]
    fn el_cansancio_depende_de_la_racha() {
        assert!(!mood_section(input(10, None, 20)).contains("trabajando"));
        assert!(mood_section(input(10, None, 60)).contains("directo al grano"));
        let tired = mood_section(input(10, None, 100));
        assert!(tired.contains("1 h 40 min") && tired.contains("descanso"), "{tired}");
    }

    #[test]
    fn recuerda_que_es_solo_estilo() {
        assert!(mood_section(input(10, None, 0)).contains("solo cambia tu tono"));
    }
}
