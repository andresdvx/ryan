//! Conversiones de lo que el usuario suelta sobre Ryan: imágenes entre formatos (y a PDF), texto, Markdown, HTML,
//! Word, PDF, Excel, CSV y JSON entre sí, y comprimir o extraer ZIP. Nunca se modifica ni se sobrescribe un archivo:
//! el resultado se escribe en la carpeta de salida con un nombre libre («informe (2).pdf» si ya existe uno).

use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader};
use serde_json::{json, Map, Value};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::doc::{self, Doc, DocImage};
use crate::incoming::{self, FileInfo, Kind};
use crate::pdf_out::{self, JpegPage};

/// Entradas que se comprimen o extraen como mucho, y bytes totales extraídos.
const MAX_ZIP_ENTRIES: usize = 20_000;
const MAX_UNZIP_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Lado mayor de «Reducir peso».
const SMALL_SIDE: u32 = 1280;
const SMALL_QUALITY: u8 = 80;
const PDF_IMAGE_QUALITY: u8 = 90;

/// Lo que salió de una conversión: lo creado y lo que no se pudo convertir (con el motivo).
#[derive(Debug, Default)]
pub struct Outcome {
    pub outputs: Vec<PathBuf>,
    pub failures: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Nombres libres
// ---------------------------------------------------------------------------------------------

/// Un nombre que no existe todavía en `dir`: «nombre.ext», «nombre (2).ext», «nombre (3).ext»…
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let stem = sanitize(stem);
    let name = |n: usize| match (n, ext.is_empty()) {
        (1, true) => stem.clone(),
        (1, false) => format!("{stem}.{ext}"),
        (n, true) => format!("{stem} ({n})"),
        (n, false) => format!("{stem} ({n}).{ext}"),
    };
    (1..).map(|n| dir.join(name(n))).find(|path| !path.exists()).expect("siempre hay un nombre libre")
}

/// Quita lo que Windows no admite en un nombre de archivo.
fn sanitize(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    if cleaned.is_empty() { "archivo".into() } else { cleaned }
}

fn write_new(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("no pude crear la carpeta de salida: {e}"))?;
    let path = unique_path(dir, stem, ext);
    // `create_new` falla si el archivo ya existe: nunca se sobrescribe nada, ni siquiera por una carrera.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("no pude guardar «{}»: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| format!("no pude escribir «{}»: {e}", path.display()))?;
    Ok(path)
}

// ---------------------------------------------------------------------------------------------
// CSV y JSON
// ---------------------------------------------------------------------------------------------

/// Separa un CSV en filas y celdas (comillas, saltos de línea dentro de una celda y separador `,` `;` o tabulador).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let first_line = text.lines().next().unwrap_or("");
    let delimiter = [',', ';', '\t'].into_iter().max_by_key(|d| first_line.matches(*d).count()).filter(|d| first_line.contains(*d)).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if cell.is_empty() => quoted = true,
            (c, false) if c == delimiter => row.push(std::mem::take(&mut cell)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    rows
}

pub fn write_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .map(|cell| if cell.contains([',', '"', '\n', '\r', ';']) { format!("\"{}\"", cell.replace('"', "\"\"")) } else { cell.clone() })
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

/// Una celda como valor JSON: los números y los true/false se reconocen; lo demás queda como texto.
fn json_value(cell: &str) -> Value {
    let plain_number = !cell.is_empty()
        && !cell.starts_with('+')
        && !(cell.len() > 1 && cell.starts_with('0') && !cell.starts_with("0."))
        && !(cell.starts_with("-0") && cell.len() > 2 && !cell.starts_with("-0."));
    if plain_number {
        if let Ok(n) = cell.parse::<i64>() {
            return json!(n);
        }
        if let Ok(n) = cell.parse::<f64>() {
            if n.is_finite() {
                return json!(n);
            }
        }
    }
    match cell {
        "true" => json!(true),
        "false" => json!(false),
        _ => json!(cell),
    }
}

/// Filas con encabezado → lista de objetos. Las columnas sin nombre se llaman «columna N».
pub fn rows_to_json(rows: &[Vec<String>]) -> Value {
    let Some((header, body)) = rows.split_first() else { return json!([]) };
    let names: Vec<String> = header.iter().enumerate().map(|(i, h)| if h.trim().is_empty() { format!("columna {}", i + 1) } else { h.trim().to_string() }).collect();
    Value::Array(
        body.iter()
            .map(|row| {
                let mut object = Map::new();
                for (i, name) in names.iter().enumerate() {
                    object.insert(name.clone(), json_value(row.get(i).map_or("", String::as_str)));
                }
                Value::Object(object)
            })
            .collect(),
    )
}

fn json_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Una lista de objetos → filas con encabezado (las columnas son la unión de las claves, en orden de aparición).
pub fn json_to_rows(value: &Value) -> Result<Vec<Vec<String>>, String> {
    let items = match value {
        Value::Array(items) => items,
        _ => return Err("el JSON debe ser una lista de objetos para pasarlo a CSV".into()),
    };
    let mut columns: Vec<String> = Vec::new();
    for item in items {
        let Value::Object(object) = item else { return Err("el JSON debe ser una lista de objetos para pasarlo a CSV".into()) };
        for key in object.keys() {
            if !columns.contains(key) {
                columns.push(key.clone());
            }
        }
    }
    if columns.is_empty() {
        return Err("el JSON no tiene datos que pasar a CSV".into());
    }
    let mut rows = vec![columns.clone()];
    for item in items {
        rows.push(columns.iter().map(|c| item.get(c).map_or(String::new(), json_cell)).collect());
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------------------------
// Imágenes
// ---------------------------------------------------------------------------------------------

fn load_image(path: &Path) -> Result<DynamicImage, String> {
    ImageReader::open(path)
        .map_err(|e| format!("no pude abrir la imagen: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("no pude leer la imagen: {e}"))?
        .decode()
        .map_err(|e| format!("no pude leer la imagen: {e}"))
}

/// Pone la imagen sobre fondo blanco (para formatos sin transparencia).
fn flatten(image: &DynamicImage) -> DynamicImage {
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let mix = |c: u8| ((c as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        rgb.put_pixel(x, y, image::Rgb([mix(pixel[0]), mix(pixel[1]), mix(pixel[2])]));
    }
    DynamicImage::ImageRgb8(rgb)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    flatten(image).write_with_encoder(JpegEncoder::new_with_quality(&mut Cursor::new(&mut bytes), quality)).map_err(|e| format!("no pude crear el JPG: {e}"))?;
    Ok(bytes)
}

/// Codifica en un formato por su extensión (`png`, `jpg`, `webp`, `bmp`, `gif`, `tiff`, `ico`).
fn encode_image(image: &DynamicImage, ext: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    let result = match ext {
        "jpg" | "jpeg" => return encode_jpeg(image, 92),
        "png" => image.write_to(&mut cursor, ImageFormat::Png),
        "webp" => DynamicImage::ImageRgba8(image.to_rgba8()).write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut cursor)),
        "bmp" => image.write_to(&mut cursor, ImageFormat::Bmp),
        "gif" => DynamicImage::ImageRgba8(image.to_rgba8()).write_to(&mut cursor, ImageFormat::Gif),
        "tiff" | "tif" => DynamicImage::ImageRgba8(image.to_rgba8()).write_to(&mut cursor, ImageFormat::Tiff),
        "ico" => {
            // Un icono mide como mucho 256 x 256.
            let fitted = if image.width().max(image.height()) > 256 { image.resize(256, 256, FilterType::Lanczos3) } else { image.clone() };
            DynamicImage::ImageRgba8(fitted.to_rgba8()).write_to(&mut cursor, ImageFormat::Ico)
        }
        other => return Err(format!("no sé crear imágenes {other}")),
    };
    result.map_err(|e| format!("no pude crear la imagen {}: {e}", ext.to_ascii_uppercase()))?;
    Ok(bytes)
}

fn convert_image(file: &FileInfo, target: &str, out_dir: &Path) -> Result<PathBuf, String> {
    let image = load_image(&file.path)?;
    let stem = file.stem();
    match target {
        "png" | "jpg" | "webp" | "bmp" | "gif" | "tiff" | "ico" => {
            let ext = if target == "tiff" { "tiff" } else { target };
            write_new(out_dir, &stem, ext, &encode_image(&image, target)?)
        }
        "pdf" => write_new(out_dir, &stem, "pdf", &pdf_out::images_to_pdf(&[jpeg_page(&image)?]))
            .map_err(|e| format!("{e}")),
        "img_small" => {
            let fitted = if image.width().max(image.height()) > SMALL_SIDE { image.resize(SMALL_SIDE, SMALL_SIDE, FilterType::Lanczos3) } else { image };
            write_new(out_dir, &format!("{stem} (ligera)"), "jpg", &encode_jpeg(&fitted, SMALL_QUALITY)?)
        }
        "img_half" => {
            let (w, h) = ((image.width() / 2).max(1), (image.height() / 2).max(1));
            let ext = match file.ext.as_str() {
                "jpeg" => "jpg",
                "tif" => "tiff",
                other => other,
            };
            write_new(out_dir, &format!("{stem} (50%)"), ext, &encode_image(&image.resize_exact(w, h, FilterType::Lanczos3), ext)?)
        }
        "img_strip" => {
            // Volver a codificar la imagen deja solo los píxeles: se pierden la ubicación, la cámara y demás metadatos.
            let ext = if matches!(file.ext.as_str(), "jpg" | "jpeg") { "jpg" } else { "png" };
            write_new(out_dir, &format!("{stem} (sin metadatos)"), ext, &encode_image(&image, ext)?)
        }
        other => Err(format!("no sé convertir una imagen a «{other}»")),
    }
}

/// Una imagen de un documento (PNG, JPG, GIF, WebP…) lista para incrustarla en un PDF o una página web: en JPEG y
/// de 2000 píxeles como máximo. Un formato que no se sabe leer (EMF, SVG…) da `None`.
pub(crate) fn doc_image_from_bytes(bytes: &[u8], extent: Option<(u32, u32)>) -> Option<DocImage> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = if image.width().max(image.height()) > 2000 { image.resize(2000, 2000, FilterType::Lanczos3) } else { image };
    Some(DocImage { jpeg: encode_jpeg(&image, 85).ok()?, width: image.width(), height: image.height(), extent })
}

fn jpeg_page(image: &DynamicImage) -> Result<JpegPage, String> {
    Ok(JpegPage { jpeg: encode_jpeg(image, PDF_IMAGE_QUALITY)?, width: image.width(), height: image.height() })
}

// ---------------------------------------------------------------------------------------------
// Documentos
// ---------------------------------------------------------------------------------------------

fn convert_document(file: &FileInfo, target: &str, out_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let stem = file.stem();
    // De texto plano a Markdown se copia tal cual: es lo más fiel.
    if target == "md" && matches!(file.kind, Kind::Text) {
        let (text, _) = incoming::read_text_file(&file.path)?;
        return Ok(vec![write_new(out_dir, &stem, "md", text.as_bytes())?]);
    }
    match (file.kind, target) {
        (Kind::Json, "csv") => {
            let (text, _) = incoming::read_text_file(&file.path)?;
            let value: Value = serde_json::from_str(&text).map_err(|e| format!("el JSON no es válido: {e}"))?;
            Ok(vec![write_new(out_dir, &stem, "csv", write_csv(&json_to_rows(&value)?).as_bytes())?])
        }
        (Kind::Json, "json_pretty" | "json_min") => {
            let (text, _) = incoming::read_text_file(&file.path)?;
            let value: Value = serde_json::from_str(&text).map_err(|e| format!("el JSON no es válido: {e}"))?;
            let (suffix, body) = if target == "json_pretty" { ("legible", serde_json::to_string_pretty(&value)) } else { ("compacto", serde_json::to_string(&value)) };
            Ok(vec![write_new(out_dir, &format!("{stem} ({suffix})"), "json", body.map_err(|e| e.to_string())?.as_bytes())?])
        }
        (Kind::Csv, "json") => {
            let (text, _) = incoming::read_text_file(&file.path)?;
            let rows = parse_csv(&text);
            if rows.is_empty() {
                return Err("el CSV está vacío".into());
            }
            let body = serde_json::to_string_pretty(&rows_to_json(&rows)).map_err(|e| e.to_string())?;
            Ok(vec![write_new(out_dir, &stem, "json", body.as_bytes())?])
        }
        (Kind::Spreadsheet, "csv") => {
            let sheets = crate::office::sheets(&file.path)?;
            let several = sheets.iter().filter(|(_, rows)| !rows.is_empty()).count() > 1;
            let mut outputs = Vec::new();
            for (name, rows) in sheets.into_iter().filter(|(_, rows)| !rows.is_empty()) {
                let name = if several { format!("{stem} - {name}") } else { stem.clone() };
                outputs.push(write_new(out_dir, &name, "csv", write_csv(&rows).as_bytes())?);
            }
            Ok(outputs)
        }
        (Kind::Spreadsheet, "json") => {
            let sheets = crate::office::sheets(&file.path)?;
            let mut object = Map::new();
            for (name, rows) in sheets.into_iter().filter(|(_, rows)| !rows.is_empty()) {
                object.insert(name, rows_to_json(&rows));
            }
            let body = serde_json::to_string_pretty(&Value::Object(object)).map_err(|e| e.to_string())?;
            Ok(vec![write_new(out_dir, &stem, "json", body.as_bytes())?])
        }
        _ => {
            let document: Doc = incoming::doc_of(file)?;
            let (ext, bytes) = match target {
                "txt" => ("txt", doc::to_text(&document).into_bytes()),
                "md" => ("md", doc::to_markdown(&document).into_bytes()),
                "html" => ("html", doc::to_html(&document, &stem).into_bytes()),
                "pdf" => ("pdf", pdf_out::doc_to_pdf(&document)),
                other => return Err(format!("no sé convertir «{}» a «{other}»", file.label())),
            };
            Ok(vec![write_new(out_dir, &stem, ext, &bytes)?])
        }
    }
}

// ---------------------------------------------------------------------------------------------
// ZIP
// ---------------------------------------------------------------------------------------------

fn add_to_zip(zip: &mut ZipWriter<File>, path: &Path, name: &str, count: &mut usize, skip: &Path) -> Result<(), String> {
    if *count >= MAX_ZIP_ENTRIES {
        return Err(format!("hay demasiados archivos (más de {MAX_ZIP_ENTRIES}) para comprimirlos"));
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("no pude leer «{}»: {e}", path.display()))?;
    if metadata.file_type().is_symlink() || path == skip {
        return Ok(()); // los enlaces no se siguen: podrían apuntar fuera de lo que el usuario soltó
    }
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(metadata.len() >= 0xFFFF_FFFF);
    if metadata.is_dir() {
        zip.add_directory(format!("{name}/"), options).map_err(|e| format!("no pude comprimir «{name}»: {e}"))?;
        *count += 1;
        let mut children: Vec<_> = std::fs::read_dir(path).map_err(|e| format!("no pude leer la carpeta «{name}»: {e}"))?.flatten().collect();
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            add_to_zip(zip, &child.path(), &format!("{name}/{}", child.file_name().to_string_lossy()), count, skip)?;
        }
    } else {
        zip.start_file(name, options).map_err(|e| format!("no pude comprimir «{name}»: {e}"))?;
        let mut source = File::open(path).map_err(|e| format!("no pude abrir «{}»: {e}", path.display()))?;
        std::io::copy(&mut source, zip).map_err(|e| format!("no pude comprimir «{name}»: {e}"))?;
        *count += 1;
    }
    Ok(())
}

fn zip_files(files: &[FileInfo], out_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("no pude crear la carpeta de salida: {e}"))?;
    let stem = if let [one] = files { one.stem() } else { "Archivos".to_string() };
    let path = unique_path(out_dir, &stem, "zip");
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("no pude crear el zip: {e}"))?;
    let mut zip = ZipWriter::new(file);
    let mut count = 0;
    let mut result = Ok(());
    for file in files {
        result = add_to_zip(&mut zip, &file.path, &sanitize(&file.name), &mut count, &path);
        if result.is_err() {
            break;
        }
    }
    let finished = result.and_then(|_| zip.finish().map(|_| ()).map_err(|e| format!("no pude terminar el zip: {e}")));
    if let Err(error) = finished {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn unzip_file(file: &FileInfo, out_dir: &Path) -> Result<PathBuf, String> {
    let source = File::open(&file.path).map_err(|e| format!("no pude abrir el zip: {e}"))?;
    let mut archive = ZipArchive::new(source).map_err(|_| "no parece un zip válido, o está protegido con contraseña".to_string())?;
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!("el zip tiene demasiados archivos (más de {MAX_ZIP_ENTRIES})"));
    }
    std::fs::create_dir_all(out_dir).map_err(|e| format!("no pude crear la carpeta de salida: {e}"))?;
    let destination = unique_path(out_dir, &file.stem(), "");
    std::fs::create_dir(&destination).map_err(|e| format!("no pude crear la carpeta: {e}"))?;
    let mut budget = MAX_UNZIP_BYTES;
    let result = (|| -> Result<(), String> {
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|e| format!("el zip está dañado o tiene contraseña: {e}"))?;
            let Some(relative) = entry.enclosed_name() else {
                return Err("el zip contiene una ruta insegura; no lo extraigo".into());
            };
            let target = destination.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&target).map_err(|e| format!("no pude crear una carpeta: {e}"))?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("no pude crear una carpeta: {e}"))?;
            }
            let mut out = File::create(&target).map_err(|e| format!("no pude crear un archivo: {e}"))?;
            let written = std::io::copy(&mut (&mut entry).take(budget), &mut out).map_err(|e| format!("no pude extraer un archivo: {e}"))?;
            budget -= written;
            if budget == 0 {
                return Err("el zip es demasiado grande al extraerlo (más de 4 GB)".into());
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }
    Ok(destination)
}

// ---------------------------------------------------------------------------------------------
// Punto de entrada
// ---------------------------------------------------------------------------------------------

/// Convierte lo soltado al formato `target` (un `id` de `incoming::offers_for`) y lo deja en `out_dir`.
/// Cada archivo se intenta por separado: lo que falla se cuenta aparte y no impide convertir el resto.
pub fn convert(files: &[FileInfo], target: &str, out_dir: &Path) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    match target {
        "zip" => outcome.outputs.push(zip_files(files, out_dir)?),
        "unzip" => {
            let [file] = files else { return Err("Para extraer, suelta un solo zip.".into()) };
            if file.kind != Kind::Zip {
                return Err(format!("«{}» no es un zip.", file.name));
            }
            outcome.outputs.push(unzip_file(file, out_dir)?);
        }
        "pdf_merge" => {
            let mut pages = Vec::new();
            for file in files {
                match load_image(&file.path).and_then(|image| jpeg_page(&image)) {
                    Ok(page) => pages.push(page),
                    Err(error) => outcome.failures.push(format!("«{}»: {error}", file.name)),
                }
            }
            if pages.is_empty() {
                return Err(outcome.failures.join(" "));
            }
            outcome.outputs.push(write_new(out_dir, "Imágenes", "pdf", &pdf_out::images_to_pdf(&pages))?);
        }
        _ => {
            for file in files {
                let result = match file.kind {
                    Kind::Image => convert_image(file, target, out_dir).map(|path| vec![path]),
                    Kind::Zip | Kind::Folder | Kind::Other => Err(format!("no sé convertir «{}» a «{target}»", file.label())),
                    _ => convert_document(file, target, out_dir),
                };
                match result {
                    Ok(paths) => outcome.outputs.extend(paths),
                    Err(error) => outcome.failures.push(format!("«{}»: {error}", file.name)),
                }
            }
        }
    }
    if outcome.outputs.is_empty() {
        return Err(if outcome.failures.is_empty() { "No se creó nada.".into() } else { outcome.failures.join(" ") });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zen-convert-{name}-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> FileInfo {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        incoming::inspect(&[path]).unwrap().remove(0)
    }

    fn png(dir: &Path, name: &str, w: u32, h: u32) -> FileInfo {
        let path = dir.join(name);
        image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 90, if x < 4 { 0 } else { 255 }])).save(&path).unwrap();
        incoming::inspect(&[path]).unwrap().remove(0)
    }

    #[test]
    fn los_nombres_nunca_pisan_a_otro_archivo() {
        let d = dir("nombres");
        std::fs::write(d.join("informe.pdf"), b"1").unwrap();
        assert_eq!(unique_path(&d, "informe", "pdf"), d.join("informe (2).pdf"));
        std::fs::write(d.join("informe (2).pdf"), b"2").unwrap();
        assert_eq!(unique_path(&d, "informe", "pdf"), d.join("informe (3).pdf"));
        assert_eq!(unique_path(&d, "nuevo", ""), d.join("nuevo"));
        assert_eq!(unique_path(&d, "a<b>:c?", "txt"), d.join("a_b__c_.txt"));
        assert_eq!(sanitize("..."), "archivo");
        let a = write_new(&d, "dato", "txt", b"uno").unwrap();
        let b = write_new(&d, "dato", "txt", b"dos").unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), b"uno", "el primero no se tocó");
    }

    #[test]
    fn el_csv_se_lee_con_comillas_saltos_y_separadores() {
        assert_eq!(parse_csv("a,b\n1,2\n"), vec![vec!["a", "b"], vec!["1", "2"]]);
        assert_eq!(parse_csv("nombre;edad\r\nAna;30\r\n"), vec![vec!["nombre", "edad"], vec!["Ana", "30"]], "punto y coma");
        assert_eq!(parse_csv("a\tb\n1\t2"), vec![vec!["a", "b"], vec!["1", "2"]], "tabulador y sin salto final");
        assert_eq!(parse_csv("x,y\n\"hola, mundo\",\"di \"\"hola\"\"\"\n\"dos\nlíneas\",z\n"), vec![vec!["x", "y"], vec!["hola, mundo", "di \"hola\""], vec!["dos\nlíneas", "z"]]);
        assert_eq!(parse_csv("\u{feff}a,b\n"), vec![vec!["a", "b"]], "quita la marca de orden de bytes");
        assert!(parse_csv("").is_empty() && parse_csv("\n\n").is_empty());
        let rows = vec![vec!["a".to_string(), "b,c".into()], vec!["d\"e".into(), "f".into()]];
        assert_eq!(parse_csv(&write_csv(&rows)), rows, "ida y vuelta");
    }

    #[test]
    fn csv_y_json_se_convierten_entre_si() {
        let rows = parse_csv("nombre,edad,activo,codigo\nAna,30,true,007\nLuis,,false,12.5\n");
        let json = rows_to_json(&rows);
        assert_eq!(json[0], json!({ "nombre": "Ana", "edad": 30, "activo": true, "codigo": "007" }), "un código con ceros a la izquierda sigue siendo texto");
        assert_eq!(json[1], json!({ "nombre": "Luis", "edad": "", "activo": false, "codigo": 12.5 }));
        assert_eq!(rows_to_json(&[vec!["".into(), "x".into()], vec!["1".into(), "2".into()]])[0], json!({ "columna 1": 1, "x": 2 }));
        let back = json_to_rows(&json!([{ "a": 1, "b": "x" }, { "a": 2, "c": null, "d": [1, 2] }])).unwrap();
        assert_eq!(back, vec![vec!["a", "b", "c", "d"], vec!["1", "x", "", ""], vec!["2", "", "", "[1,2]"]]);
        assert!(json_to_rows(&json!({ "a": 1 })).is_err() && json_to_rows(&json!([1, 2])).is_err() && json_to_rows(&json!([])).is_err());
    }

    #[test]
    fn convierte_una_imagen_a_cada_formato_y_se_puede_volver_a_abrir() {
        let d = dir("imagenes");
        let out = d.join("salida");
        let source = png(&d, "foto.png", 120, 80);
        for (target, ext) in [("jpg", "jpg"), ("webp", "webp"), ("bmp", "bmp"), ("gif", "gif"), ("tiff", "tiff"), ("ico", "ico")] {
            let outcome = convert(std::slice::from_ref(&source), target, &out).unwrap();
            let path = &outcome.outputs[0];
            assert_eq!(path.extension().unwrap(), ext, "{target}");
            let reopened = image::open(path).unwrap_or_else(|e| panic!("{target} no se puede abrir: {e}"));
            assert_eq!((reopened.width(), reopened.height()), (120, 80), "{target} conserva el tamaño");
        }
        let pdf = convert(std::slice::from_ref(&source), "pdf", &out).unwrap().outputs.remove(0);
        let bytes = std::fs::read(&pdf).unwrap();
        assert!(bytes.starts_with(b"%PDF") && String::from_utf8_lossy(&bytes).contains("/DCTDecode"));
        assert!(source.path.exists(), "el original no se toca");
    }

    #[test]
    fn reduce_peso_tamano_y_quita_metadatos() {
        let d = dir("optimizar");
        let out = d.join("salida");
        let big = png(&d, "grande.png", 2000, 1000);
        let small = convert(std::slice::from_ref(&big), "img_small", &out).unwrap().outputs.remove(0);
        assert!(small.file_name().unwrap().to_string_lossy().contains("(ligera)"));
        let reopened = image::open(&small).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (1280, 640));
        assert!(std::fs::metadata(&small).unwrap().len() < std::fs::metadata(&big.path).unwrap().len());
        let half = convert(std::slice::from_ref(&big), "img_half", &out).unwrap().outputs.remove(0);
        assert_eq!(image::image_dimensions(&half).unwrap(), (1000, 500));
        assert_eq!(half.extension().unwrap(), "png");
        let clean = convert(std::slice::from_ref(&big), "img_strip", &out).unwrap().outputs.remove(0);
        assert!(clean.file_name().unwrap().to_string_lossy().contains("(sin metadatos)"));
        // Un JPEG con un bloque de metadatos (EXIF) falso: tras limpiar ya no está.
        let jpeg_path = d.join("con_exif.jpg");
        let mut jpeg = encode_jpeg(&load_image(&big.path).unwrap(), 90).unwrap();
        jpeg.splice(2..2, [0xFF, 0xE1, 0x00, 0x10].into_iter().chain(*b"Exif\0\0SECRETO!!"));
        std::fs::write(&jpeg_path, &jpeg).unwrap();
        assert!(jpeg.windows(7).any(|w| w == b"SECRETO"));
        let cleaned = convert(&incoming::inspect(&[jpeg_path]).unwrap(), "img_strip", &out).unwrap().outputs.remove(0);
        assert!(!std::fs::read(cleaned).unwrap().windows(7).any(|w| w == b"SECRETO"), "los metadatos desaparecen");
    }

    #[test]
    fn une_varias_imagenes_en_un_pdf() {
        let d = dir("unir");
        let files = [png(&d, "1.png", 100, 50), png(&d, "2.png", 50, 100), put(&d, "falsa.png", b"no soy una imagen")];
        let outcome = convert(&files, "pdf_merge", &d.join("salida")).unwrap();
        let text = String::from_utf8_lossy(&std::fs::read(&outcome.outputs[0]).unwrap()).into_owned();
        assert!(text.contains("/Count 2"), "dos páginas: la falsa se salta");
        assert_eq!(outcome.failures.len(), 1);
        assert!(outcome.failures[0].contains("falsa.png"));
        assert!(convert(&files[2..], "pdf_merge", &d.join("salida")).is_err());
    }

    #[test]
    fn convierte_entre_texto_markdown_html_y_pdf() {
        let d = dir("docs");
        let out = d.join("salida");
        let md = put(&d, "nota.md", "# Título\n\nUn párrafo con **negrita**.\n\n- uno\n- dos\n".as_bytes());
        let txt = std::fs::read_to_string(&convert(std::slice::from_ref(&md), "txt", &out).unwrap().outputs[0]).unwrap();
        assert!(txt.starts_with("Título\n\nUn párrafo con negrita.") && !txt.contains("**"), "{txt:?}");
        let html = std::fs::read_to_string(&convert(std::slice::from_ref(&md), "html", &out).unwrap().outputs[0]).unwrap();
        assert!(html.contains("<h1>Título</h1>") && html.contains("<li>uno</li>"));
        let pdf = std::fs::read(&convert(std::slice::from_ref(&md), "pdf", &out).unwrap().outputs[0]).unwrap();
        assert!(pdf_extract::extract_text_from_mem(&pdf).unwrap().contains("Un párrafo") || pdf_extract::extract_text_from_mem(&pdf).unwrap().contains("rrafo"));
        let plain = put(&d, "cartas.txt", "Hola\nmundo\n\nFin".as_bytes());
        let as_md = convert(std::slice::from_ref(&plain), "md", &out).unwrap().outputs.remove(0);
        assert_eq!(std::fs::read_to_string(as_md).unwrap(), "Hola\nmundo\n\nFin", "texto a Markdown se copia tal cual");
        let page = put(&d, "web.html", b"<h1>Hola</h1><p>Mundo &amp; mas</p><script>x()</script>");
        let md_from_html = std::fs::read_to_string(&convert(std::slice::from_ref(&page), "md", &out).unwrap().outputs[0]).unwrap();
        assert_eq!(md_from_html, "# Hola\n\nMundo & mas\n");
        let error = convert(std::slice::from_ref(&plain), "csv", &out).unwrap_err();
        assert!(error.contains("no sé convertir"), "{error}");
    }

    #[test]
    fn convierte_csv_y_json_a_archivos() {
        let d = dir("datos");
        let out = d.join("salida");
        let csv = put(&d, "ventas.csv", b"producto;precio\nCafe;3.5\nTe;12\n");
        let json_path = convert(std::slice::from_ref(&csv), "json", &out).unwrap().outputs.remove(0);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(value, json!([{ "producto": "Cafe", "precio": 3.5 }, { "producto": "Te", "precio": 12 }]));
        let json = incoming::inspect(&[json_path]).unwrap().remove(0);
        let back = convert(std::slice::from_ref(&json), "csv", &out).unwrap().outputs.remove(0);
        assert_eq!(std::fs::read_to_string(back).unwrap(), "producto,precio\r\nCafe,3.5\r\nTe,12\r\n");
        let compact = convert(std::slice::from_ref(&json), "json_min", &out).unwrap().outputs.remove(0);
        assert_eq!(std::fs::read_to_string(compact).unwrap(), r#"[{"producto":"Cafe","precio":3.5},{"producto":"Te","precio":12}]"#);
        let broken = put(&d, "roto.json", b"{ no es json");
        assert!(convert(&[broken], "json_pretty", &out).unwrap_err().contains("no es válido"));
        let as_md = std::fs::read_to_string(&convert(std::slice::from_ref(&csv), "md", &out).unwrap().outputs[0]).unwrap();
        assert!(as_md.contains("| producto | precio |") && as_md.contains("| Cafe | 3.5 |"), "{as_md}");
    }

    #[test]
    fn comprime_y_extrae_sin_salirse_de_la_carpeta() {
        let d = dir("zip");
        let out = d.join("salida");
        std::fs::create_dir_all(d.join("carpeta").join("sub")).unwrap();
        std::fs::write(d.join("carpeta").join("a.txt"), "hola").unwrap();
        std::fs::write(d.join("carpeta").join("sub").join("b.txt"), "adiós ñ").unwrap();
        let folder = incoming::inspect(&[d.join("carpeta")]).unwrap().remove(0);
        let loose = put(&d, "suelto.txt", b"x");
        let zipped = convert(&[folder.clone(), loose.clone()], "zip", &out).unwrap().outputs.remove(0);
        assert_eq!(zipped.file_name().unwrap(), "Archivos.zip");
        let zip_info = incoming::inspect(&[zipped]).unwrap().remove(0);
        let extracted = convert(std::slice::from_ref(&zip_info), "unzip", &d.join("fuera")).unwrap().outputs.remove(0);
        assert_eq!(std::fs::read_to_string(extracted.join("carpeta").join("sub").join("b.txt")).unwrap(), "adiós ñ");
        assert_eq!(std::fs::read_to_string(extracted.join("suelto.txt")).unwrap(), "x");
        let single = convert(std::slice::from_ref(&folder), "zip", &out).unwrap().outputs.remove(0);
        assert_eq!(single.file_name().unwrap(), "carpeta.zip", "un solo elemento da su nombre al zip");
        // Un zip con una ruta que intenta salirse de la carpeta se rechaza.
        let evil = d.join("malo.zip");
        let mut zip = ZipWriter::new(File::create(&evil).unwrap());
        zip.start_file("../fuera.txt", SimpleFileOptions::default()).unwrap();
        zip.write_all(b"escape").unwrap();
        zip.finish().unwrap();
        let evil_info = incoming::inspect(&[evil]).unwrap().remove(0);
        let error = convert(std::slice::from_ref(&evil_info), "unzip", &d.join("destino")).unwrap_err();
        assert!(error.contains("insegura"), "{error}");
        assert!(!d.join("fuera.txt").exists() && !d.join("destino").join("fuera.txt").exists());
        assert!(!d.join("destino").join("malo").exists(), "no deja una carpeta a medias");
        assert!(convert(&[folder, loose], "unzip", &out).is_err());
    }

    #[test]
    fn convertir_varios_cuenta_lo_que_falla_sin_parar() {
        let d = dir("lote");
        let files = [put(&d, "a.md", b"# A"), put(&d, "b.exe", &[0, 1, 2]), put(&d, "c.md", b"# C")];
        let outcome = convert(&files, "txt", &d.join("salida")).unwrap();
        assert_eq!(outcome.outputs.len(), 2);
        assert_eq!(outcome.failures.len(), 1);
        assert!(outcome.failures[0].contains("b.exe"));
    }

    #[test]
    fn un_word_con_imagenes_las_conserva_en_pdf_y_html() {
        let d = dir("wordimg");
        let out = d.join("salida");
        let docx = d.join("informe.docx");
        crate::office::tests::docx_with_image(&docx);
        let file = incoming::inspect(&[docx]).unwrap().remove(0);

        let pdf_path = convert(std::slice::from_ref(&file), "pdf", &out).unwrap().outputs.remove(0);
        let pdf = std::fs::read(&pdf_path).unwrap();
        let raw = String::from_utf8_lossy(&pdf);
        assert!(raw.contains("/Subtype /Image") && raw.contains("/DCTDecode"), "la imagen está dentro del PDF");
        let text = pdf_extract::extract_text_from_mem(&pdf).unwrap();
        assert!(text.contains("Antes") && text.contains("Despu"), "y el texto también: {text:?}");

        let html = std::fs::read_to_string(&convert(std::slice::from_ref(&file), "html", &out).unwrap().outputs[0]).unwrap();
        assert!(html.contains("<img src=\"data:image/jpeg;base64,/9j/"), "en la página web va incrustada");
        assert!(html.find("Antes").unwrap() < html.find("<img").unwrap() && html.find("<img").unwrap() < html.find("Despu").unwrap(), "en su sitio");

        let md = std::fs::read_to_string(&convert(std::slice::from_ref(&file), "md", &out).unwrap().outputs[0]).unwrap();
        assert!(md.contains("Antes\n\n[imagen]\n\nDespués"), "{md:?}");
    }
}
