use std::path::{Path, PathBuf};

use crate::names::validate_name;

const MAX_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Desktop,
    Documents,
    Downloads,
}

impl Location {
    pub const KEYS: [&'static str; 3] = ["desktop", "documents", "downloads"];

    pub fn from_key(key: &str) -> Option<Self> {
        match key.to_ascii_lowercase().as_str() {
            "desktop" => Some(Self::Desktop),
            "documents" => Some(Self::Documents),
            "downloads" => Some(Self::Downloads),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Documents => "documents",
            Self::Downloads => "downloads",
        }
    }
}

/// Las únicas carpetas donde las herramientas de archivos pueden actuar.
#[derive(Debug, Clone)]
pub struct Locations {
    desktop: PathBuf,
    documents: PathBuf,
    downloads: PathBuf,
}

impl Locations {
    pub fn new(desktop: PathBuf, documents: PathBuf, downloads: PathBuf) -> Self {
        Self { desktop, documents, downloads }
    }

    /// Usa las carpetas conocidas de Windows (respeta redirecciones como OneDrive).
    pub fn from_system() -> Option<Self> {
        Some(Self::new(dirs::desktop_dir()?, dirs::document_dir()?, dirs::download_dir()?))
    }

    fn root(&self, location: Location) -> &Path {
        match location {
            Location::Desktop => &self.desktop,
            Location::Documents => &self.documents,
            Location::Downloads => &self.downloads,
        }
    }

    /// Carpeta de destino: la ubicación o una subcarpeta ya existente dentro de ella.
    /// Cada tramo de la subcarpeta es un nombre simple y el resultado se comprueba
    /// una vez resueltos enlaces simbólicos, así que no se puede salir de la ubicación.
    pub fn resolve_dir(&self, location: Location, subfolder: Option<&str>) -> Result<PathBuf, String> {
        let root = self
            .root(location)
            .canonicalize()
            .map_err(|_| format!("la ubicación «{}» no existe", location.key()))?;
        let Some(subfolder) = subfolder else {
            return Ok(root);
        };

        let parts: Vec<&str> = subfolder.split(['/', '\\']).collect();
        if parts.len() > MAX_DEPTH {
            return Err(format!("la subcarpeta tiene más de {MAX_DEPTH} niveles"));
        }
        let mut dir = root.clone();
        for part in parts {
            validate_name(part).map_err(|reason| format!("subcarpeta inválida «{part}»: {reason}"))?;
            dir.push(part);
        }
        let dir = dir
            .canonicalize()
            .map_err(|_| format!("la subcarpeta «{subfolder}» no existe en {}", location.key()))?;
        if !dir.starts_with(&root) || !dir.is_dir() {
            return Err("la subcarpeta queda fuera de la ubicación permitida".into());
        }
        Ok(dir)
    }
}

impl Locations {
    /// ¿Está esta ruta (existente o no) dentro de alguna ubicación permitida, una vez resueltos los enlaces?
    pub fn contains(&self, path: &Path) -> bool {
        let Some(resolved) = resolve_leniently(path) else { return false };
        [Location::Desktop, Location::Documents, Location::Downloads]
            .iter()
            .filter_map(|&location| self.root(location).canonicalize().ok())
            .any(|root| resolved.starts_with(root))
    }
}

/// Ruta absoluta y sin enlaces aunque el final no exista: se resuelve el ancestro existente más
/// cercano y se le añade lo que falta. Un `..` en la parte inexistente se rechaza.
fn resolve_leniently(path: &Path) -> Option<PathBuf> {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = current.canonicalize() {
            return Some(missing.iter().rev().fold(real, |full, part| full.join(part)));
        }
        missing.push(current.file_name()?);
        current = current.parent()?;
    }
}

/// Ruta legible: quita el prefijo `\\?\` que añade `canonicalize` en Windows.
pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_locations;

    #[test]
    fn contains_acepta_lo_de_dentro_y_rechaza_lo_de_fuera() {
        let t = temp_locations();
        let inside = t.locations_root("desktop").join("nuevo.txt");
        assert!(t.locations.contains(&inside), "un archivo que aún no existe pero cuelga de una ubicación");
        assert!(t.locations.contains(&t.locations_root("documents")));
        assert!(!t.locations.contains(&t.locations_root("data").join("x.json")));
        assert!(!t.locations.contains(&t.locations_root("desktop").join("..").join("data").join("x")));
        assert!(!t.locations.contains(Path::new("C:\\Windows\\System32\\cmd.exe")));
        assert!(!t.locations.contains(Path::new("relativa")));
        // Rutas cuyo final ya no existe (carpeta renombrada o borrada después): dentro o fuera según su ancestro.
        let gone = t.locations_root("desktop").join("Otros").join("Hondo").join("a.txt");
        assert!(t.locations.contains(&gone));
        let escape = t.locations_root("desktop").join("Otros").join("..").join("..").join("data").join("a.txt");
        assert!(!t.locations.contains(&escape), "un .. que sale de la ubicación sigue rechazado");
    }

    #[test]
    fn resuelve_la_raiz_y_subcarpetas_existentes() {
        let t = temp_locations();
        std::fs::create_dir_all(t.locations_root("desktop").join("a").join("b")).unwrap();
        assert!(t.locations.resolve_dir(Location::Desktop, None).is_ok());
        assert!(t.locations.resolve_dir(Location::Desktop, Some("a/b")).is_ok());
        assert!(t.locations.resolve_dir(Location::Desktop, Some("a\\b")).is_ok());
    }

    #[test]
    fn no_permite_salir_de_la_ubicacion() {
        let t = temp_locations();
        for sub in ["..", "../documents", "a/../..", "C:\\Windows", "\\Windows", "/etc", "a//b", ""] {
            assert!(t.locations.resolve_dir(Location::Desktop, Some(sub)).is_err(), "{sub:?}");
        }
    }

    #[test]
    fn rechaza_subcarpetas_inexistentes_y_demasiado_profundas() {
        let t = temp_locations();
        assert!(t.locations.resolve_dir(Location::Desktop, Some("no_existe")).is_err());
        assert!(t.locations.resolve_dir(Location::Desktop, Some("a/b/c/d/e")).is_err());
    }
}