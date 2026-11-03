use std::fs;
use std::io;
use std::path::Path;

/// Copia `assets/` (fuente de verdad de los sprites) a `ui/assets/`, que es lo que Tauri empaqueta.
/// Solo reescribe lo que cambió, para no provocar recompilaciones innecesarias.
fn mirror(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            mirror(&entry.path(), &target)?;
        } else {
            let unchanged = fs::read(&target).ok() == fs::read(entry.path()).ok();
            if !unchanged {
                fs::copy(entry.path(), &target)?;
            }
        }
    }
    Ok(())
}

fn main() {
    println!("cargo:rerun-if-changed=../assets");
    let assets = Path::new("../assets");
    if assets.is_dir() {
        mirror(assets, Path::new("../ui/assets")).expect("no se pudieron copiar los assets a ui/assets");
    }
    tauri_build::build()
}