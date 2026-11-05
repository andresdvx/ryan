//! Personajes disponibles. La lista vive en `assets/characters.json` y es la misma que lee la
//! interfaz: para añadir un personaje basta con soltar su carpeta de frames en `assets/` y
//! añadir una entrada al manifiesto.

use std::sync::OnceLock;

use serde_json::Value;

const MANIFEST: &str = include_str!("../../assets/characters.json");

#[derive(Debug, Clone, PartialEq)]
pub struct Character {
    pub id: String,
    pub name: String,
}

struct Catalog {
    default: String,
    characters: Vec<Character>,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        parse(MANIFEST).expect("assets/characters.json no es válido (lo comprueba una prueba al compilar)")
    })
}

fn parse(text: &str) -> Option<Catalog> {
    let value: Value = serde_json::from_str(text).ok()?;
    let characters: Vec<Character> = value["characters"]
        .as_array()?
        .iter()
        .map(|entry| Some(Character { id: entry["id"].as_str()?.to_string(), name: entry["name"].as_str()?.to_string() }))
        .collect::<Option<_>>()?;
    let default = value["default"].as_str()?.to_string();
    (characters.iter().any(|c| c.id == default)).then_some(Catalog { default, characters })
}

pub fn default_id() -> &'static str {
    &catalog().default
}

pub fn is_known(id: &str) -> bool {
    catalog().characters.iter().any(|character| character.id == id)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn assets_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("assets")
    }

    #[test]
    fn el_manifiesto_es_valido_y_el_personaje_por_defecto_existe() {
        let catalog = catalog();
        assert!(catalog.characters.len() >= 2);
        assert!(is_known(default_id()));
        assert!(is_known("peter_panda") && is_known("drako") && !is_known("zen") && !is_known("poring") && !is_known("fairy"));
        assert!(!is_known("") && !is_known("../drako") && !is_known("Drako"), "los identificadores son exactos");
    }

    #[test]
    fn los_identificadores_son_unicos_y_seguros() {
        let ids: Vec<&str> = catalog().characters.iter().map(|c| c.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "identificadores repetidos");
        for id in ids {
            assert!(!id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{id}");
        }
    }

    /// Cada personaje debe tener en disco todos los frames que declara (y ninguna ruta que se salga de `assets/`).
    #[test]
    fn cada_personaje_tiene_todos_sus_frames() {
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        for entry in manifest["characters"].as_array().unwrap() {
            let id = entry["id"].as_str().unwrap();
            let folder = entry["folder"].as_str().unwrap();
            let pattern = entry["pattern"].as_str().unwrap();
            let frames = entry["frames"].as_u64().unwrap();
            let preview = entry["preview_frame"].as_u64().unwrap();
            let speed = entry["speed"].as_f64().unwrap();
            assert!(!folder.contains("..") && !folder.starts_with('/') && !folder.contains(':'), "{id}: carpeta insegura");
            assert!(pattern.contains("{n}") || pattern.contains("{nn}"), "{id}: el patrón no numera los frames");
            assert!(frames >= 1 && (1..=frames).contains(&preview), "{id}: frame de vista previa fuera de rango");
            assert!((0.2..=5.0).contains(&speed), "{id}: velocidad absurda");
            for n in 1..=frames {
                let name = pattern.replace("{nn}", &format!("{n:02}")).replace("{n}", &n.to_string());
                let path = assets_dir().join(folder).join(&name);
                assert!(path.is_file(), "{id}: falta el frame {n}: {}", path.display());
            }
            for color in ["accent", "accent2"] {
                let value = entry[color].as_str().unwrap_or("#000000");
                assert!(value.len() == 7 && value.starts_with('#') && value[1..].chars().all(|c| c.is_ascii_hexdigit()), "{id}: {color} no es un color #rrggbb");
            }
            if let Some(phones) = entry.get("headphones") {
                let (x, y, width) = (phones["x"].as_f64().unwrap(), phones["y"].as_f64().unwrap(), phones["width"].as_f64().unwrap());
                assert!((0.0..=100.0).contains(&x) && (0.0..=100.0).contains(&y) && (10.0..=90.0).contains(&width), "{id}: auriculares fuera del sprite");
                assert!(assets_dir().join("headphones").join(format!("{id}.png")).is_file(), "{id}: falta assets/headphones/{id}.png");
            }
            for (state, spec) in entry["states"].as_object().into_iter().flatten() {
                if let Some(ms) = spec.get("ms") {
                    assert!(ms.as_u64().is_some_and(|ms| (20..=3000).contains(&ms)), "{id}/{state}: «ms» debe ser un entero entre 20 y 3000");
                }
                // Una animación de estado con carpeta propia (la que crea tools/import_sprites.py) debe tener todos sus archivos.
                if spec.get("sequence").is_none() {
                    let state_folder = spec["folder"].as_str().unwrap_or_else(|| panic!("{id}/{state}: falta «folder» o «sequence»"));
                    let state_pattern = spec["pattern"].as_str().unwrap_or_else(|| panic!("{id}/{state}: falta «pattern»"));
                    let state_frames = spec["frames"].as_u64().unwrap_or(0);
                    assert!(!state_folder.contains("..") && !state_folder.starts_with('/') && !state_folder.contains(':'), "{id}/{state}: carpeta insegura");
                    assert!(state_pattern.contains("{n}") || state_pattern.contains("{nn}"), "{id}/{state}: el patrón no numera los frames");
                    assert!(state_frames >= 1, "{id}/{state}: sin frames");
                    for n in 1..=state_frames {
                        let name = state_pattern.replace("{nn}", &format!("{n:02}")).replace("{n}", &n.to_string());
                        let path = assets_dir().join(state_folder).join(&name);
                        assert!(path.is_file(), "{id}/{state}: falta el frame {n}: {}", path.display());
                    }
                }
                // Una animación de estado sin carpeta propia reutiliza los frames del personaje.
                if let Some(sequence) = spec["sequence"].as_array() {
                    assert!(!sequence.is_empty(), "{id}/{state}: secuencia vacía");
                    for n in sequence {
                        let n = n.as_u64().unwrap_or(0);
                        assert!((1..=frames).contains(&n), "{id}/{state}: el frame {n} no existe");
                    }
                }
            }
        }
    }
}
