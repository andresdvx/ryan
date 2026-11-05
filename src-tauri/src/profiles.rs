//! Perfiles: modos que cambian a la vez cómo se comporta la mascota (paseos, comentarios, sonidos) y el tono
//! con el que responde la IA. Son un añadido por encima de los ajustes del usuario, no una copia: al volver a
//! «Normal» todo queda exactamente como lo había configurado.

/// Lo que un perfil fuerza. `None` deja el ajuste del usuario como está.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overrides {
    pub wander: Option<bool>,
    pub look_at_cursor: Option<bool>,
    pub usage_comments: Option<bool>,
    pub system_events: Option<bool>,
    pub music_reaction: Option<bool>,
    pub sound_enabled: Option<bool>,
    pub sound_volume: Option<u8>,
}

const NONE: Overrides = Overrides {
    wander: None,
    look_at_cursor: None,
    usage_comments: None,
    system_events: None,
    music_reaction: None,
    sound_enabled: None,
    sound_volume: None,
};

#[derive(Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub description: &'static str,
    /// Instrucción de tono que se añade a lo que la IA sabe de la situación (vacía en «Normal»).
    pub tone: &'static str,
    pub overrides: Overrides,
}

pub const DEFAULT_ID: &str = "normal";

pub const PROFILES: [Profile; 4] = [
    Profile {
        id: "normal",
        name: "Normal",
        icon: "🙂",
        description: "Tu configuración tal cual la dejaste.",
        tone: "",
        overrides: NONE,
    },
    Profile {
        id: "work",
        name: "Trabajo",
        icon: "💼",
        description: "Menos distracciones: sin paseos ni miradas, sonidos más suaves y respuestas concisas.",
        tone: "Estás en modo trabajo: responde de forma concisa y profesional, sin bromas ni emojis salvo que te los pidan, y ve al grano.",
        overrides: Overrides { wander: Some(false), look_at_cursor: Some(false), music_reaction: Some(false), sound_volume: Some(25), ..NONE },
    },
    Profile {
        id: "rest",
        name: "Descanso",
        icon: "🌴",
        description: "Más cercano y juguetón: pasea, escucha música contigo y te anima a hacer pausas.",
        tone: "Estás en modo descanso: habla de forma relajada, cercana y con humor ligero; puedes proponer una pausa, música o algo divertido.",
        overrides: Overrides { wander: Some(true), look_at_cursor: Some(true), usage_comments: Some(true), music_reaction: Some(true), ..NONE },
    },
    Profile {
        id: "focus",
        name: "Concentración",
        icon: "🔕",
        description: "Silencio total: sin sonidos, paseos ni comentarios. Los avisos se ven, pero no suenan.",
        tone: "Modo concentración: responde con muy pocas palabras y directamente; no hagas comentarios que no te pidan ni propongas cosas.",
        overrides: Overrides {
            wander: Some(false),
            look_at_cursor: Some(false),
            usage_comments: Some(false),
            system_events: Some(false),
            music_reaction: Some(false),
            sound_enabled: Some(false),
            ..NONE
        },
    },
];

pub fn find(id: &str) -> Option<&'static Profile> {
    PROFILES.iter().find(|profile| profile.id == id)
}

/// El perfil con ese identificador, o «Normal» si no existe (un archivo de ajustes editado a mano).
pub fn get(id: &str) -> &'static Profile {
    find(id).unwrap_or(&PROFILES[0])
}

impl Profile {
    /// Qué cambia este perfil, en palabras, para enseñarlo en la interfaz.
    pub fn changes(&self) -> Vec<String> {
        let o = &self.overrides;
        let on_off = |value: Option<bool>, on: &str, off: &str| value.map(|v| if v { on } else { off }.to_string());
        let mut changes: Vec<String> = [
            on_off(o.wander, "La mascota pasea por el escritorio", "La mascota se queda quieta"),
            on_off(o.look_at_cursor, "Mira hacia el cursor", "No mira hacia el cursor"),
            on_off(o.usage_comments, "Comenta cómo usas el equipo", "No comenta cómo usas el equipo"),
            on_off(o.system_events, "Avisa de descargas y unidades USB", "No avisa de descargas ni unidades USB"),
            on_off(o.music_reaction, "Se pone auriculares cuando suena música", "No reacciona a la música"),
            on_off(o.sound_enabled, "Los avisos suenan", "Los avisos no suenan"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if let Some(volume) = o.sound_volume {
            changes.push(format!("Volumen de los avisos al {volume} %"));
        }
        if !self.tone.is_empty() {
            changes.push("Cambia el tono de las respuestas".to_string());
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_perfiles_tienen_ids_unicos_y_el_normal_no_cambia_nada() {
        let mut ids: Vec<&str> = PROFILES.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PROFILES.len());
        assert_eq!(get(DEFAULT_ID).overrides, NONE);
        assert!(get(DEFAULT_ID).tone.is_empty() && get(DEFAULT_ID).changes().is_empty());
    }

    #[test]
    fn un_perfil_desconocido_vuelve_a_normal() {
        assert_eq!(get("inventado").id, DEFAULT_ID);
        assert!(find("inventado").is_none() && find("work").is_some());
    }

    #[test]
    fn cada_perfil_distinto_de_normal_explica_lo_que_cambia() {
        for profile in PROFILES.iter().filter(|p| p.id != DEFAULT_ID) {
            assert!(!profile.description.is_empty() && !profile.tone.is_empty(), "{}", profile.id);
            let changes = profile.changes();
            assert!(changes.len() >= 3 && changes.iter().all(|c| !c.is_empty()), "{}: {changes:?}", profile.id);
        }
        let focus = get("focus").changes();
        assert!(focus.iter().any(|c| c.contains("no suenan")) && focus.iter().any(|c| c.contains("quieta")), "{focus:?}");
        assert!(get("work").changes().iter().any(|c| c.contains("25 %")));
    }
}
