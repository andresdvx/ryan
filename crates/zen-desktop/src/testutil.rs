use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::locations::Locations;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Carpetas temporales aisladas por prueba; se borran al terminar.
pub struct TempLocations {
    base: PathBuf,
    pub locations: Locations,
}

impl TempLocations {
    pub fn locations_root(&self, key: &str) -> PathBuf {
        self.base.join(key)
    }
}

impl Drop for TempLocations {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

pub fn temp_locations() -> TempLocations {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("zenpet-test-{}-{id}", std::process::id()));
    for key in ["desktop", "documents", "downloads", "data"] {
        std::fs::create_dir_all(base.join(key)).unwrap();
    }
    let locations = Locations::new(base.join("desktop"), base.join("documents"), base.join("downloads"));
    TempLocations { base, locations }
}