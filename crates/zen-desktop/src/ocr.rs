//! Reconocimiento de texto (OCR) con el motor que trae Windows: lee el texto de una imagen o de un PDF escaneado.
//! Todo ocurre en el equipo: no se envía nada a ningún servicio. Los PDF se pintan página a página con el
//! visor de PDF de Windows y cada página se pasa por el OCR. Necesita que Windows tenga el reconocimiento de
//! texto de algún idioma del usuario (viene con los paquetes de idioma).

use std::path::Path;

use image::{imageops::FilterType, DynamicImage};
use windows::core::HSTRING;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};
use windows::Storage::StorageFile;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

/// Páginas de un PDF que se leen como mucho.
pub const MAX_PDF_PAGES: usize = 40;
/// Ancho (en píxeles) con el que se pinta una página de PDF: unos 240 ppp en un A4, lo que mejor lee el OCR.
const PAGE_RENDER_WIDTH: u32 = 2000;
/// Una imagen pequeña se amplía hasta este lado mayor: el OCR lee mal el texto muy pequeño.
const MIN_SIDE: u32 = 1600;
const MAX_UPSCALE: f32 = 3.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrOutput {
    pub text: String,
    /// Idioma con el que se leyó, por ejemplo «español (España)».
    pub language: String,
    /// Páginas leídas (1 para una imagen).
    pub pages: usize,
    /// El PDF tenía más páginas de las que se leen.
    pub cut: bool,
}

fn fail(what: &str, error: impl std::fmt::Display) -> String {
    format!("{what}: {error}")
}

/// Los hilos que usan Windows Runtime deben estar inicializados. Es seguro repetirlo.
fn ensure_initialized() {
    // SAFETY: inicialización estándar del hilo actual; un error (p. ej. «ya inicializado») no impide seguir.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

const NO_ENGINE: &str = "Windows no tiene el reconocimiento de texto (OCR) de tu idioma. Instálalo en Configuración > Hora e idioma > Idioma y región > tu idioma > Opciones de idioma > «Reconocimiento de texto»";

fn engine() -> Result<OcrEngine, String> {
    ensure_initialized();
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(engine);
    }
    for tag in ["es", "en-US"] {
        if let Some(engine) = Language::CreateLanguage(&HSTRING::from(tag)).ok().and_then(|language| OcrEngine::TryCreateFromLanguage(&language).ok()) {
            return Ok(engine);
        }
    }
    Err(NO_ENGINE.into())
}

fn language_name(engine: &OcrEngine) -> String {
    engine.RecognizerLanguage().and_then(|language| language.DisplayName()).map(|name| name.to_string()).unwrap_or_else(|_| "Windows".into())
}

/// ¿Puede este equipo hacer OCR? Devuelve el idioma que usaría, o por qué no.
pub fn available() -> Result<String, String> {
    engine().map(|engine| language_name(&engine))
}

/// Una imagen ya decodificada como mapa de bits de Windows, ajustada a lo que el OCR admite.
fn bitmap_from_image(image: &DynamicImage) -> Result<SoftwareBitmap, String> {
    let max_side = OcrEngine::MaxImageDimension().unwrap_or(10_000).max(1000);
    let (w, h) = (image.width(), image.height());
    let side = w.max(h);
    let scale = if side > max_side {
        max_side as f32 / side as f32
    } else if side < MIN_SIDE {
        (MIN_SIDE as f32 / side as f32).min(MAX_UPSCALE)
    } else {
        1.0
    };
    let image = if (scale - 1.0).abs() > f32::EPSILON {
        image.resize_exact(((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1), FilterType::Lanczos3)
    } else {
        image.clone()
    };
    // Sobre fondo blanco (una imagen con transparencia daría un fondo negro) y en el orden de bytes de Windows (BGRA).
    let rgba = image.to_rgba8();
    let mut bgra = Vec::with_capacity(rgba.len());
    for pixel in rgba.pixels() {
        let alpha = pixel[3] as u32;
        let mix = |c: u8| ((c as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        bgra.extend_from_slice(&[mix(pixel[2]), mix(pixel[1]), mix(pixel[0]), 255]);
    }
    let writer = DataWriter::new().map_err(|e| fail("no pude preparar la imagen", e))?;
    writer.WriteBytes(&bgra).map_err(|e| fail("no pude preparar la imagen", e))?;
    let buffer = writer.DetachBuffer().map_err(|e| fail("no pude preparar la imagen", e))?;
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, image.width() as i32, image.height() as i32).map_err(|e| fail("no pude preparar la imagen", e))
}

/// El texto de un mapa de bits, línea a línea.
fn recognize(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<String, String> {
    let result = engine.RecognizeAsync(bitmap).and_then(|op| op.join()).map_err(|e| fail("el OCR falló", e))?;
    let lines = result.Lines().map_err(|e| fail("el OCR falló", e))?;
    let mut text = String::new();
    for line in lines {
        if let Ok(line) = line.Text() {
            text.push_str(line.to_string().trim());
            text.push('\n');
        }
    }
    Ok(text.trim().to_string())
}

/// El texto de una imagen (cualquier formato que sepa abrir la aplicación).
pub fn ocr_image(path: &Path) -> Result<OcrOutput, String> {
    let engine = engine()?;
    let image = image::ImageReader::open(path)
        .map_err(|e| fail("no pude abrir la imagen", e))?
        .with_guessed_format()
        .map_err(|e| fail("no pude leer la imagen", e))?
        .decode()
        .map_err(|e| fail("no pude leer la imagen", e))?;
    ocr_decoded(&engine, &image)
}

fn ocr_decoded(engine: &OcrEngine, image: &DynamicImage) -> Result<OcrOutput, String> {
    let text = recognize(engine, &bitmap_from_image(image)?)?;
    Ok(OcrOutput { text, language: language_name(engine), pages: 1, cut: false })
}

/// Los bytes de lo escrito en una corriente de memoria de Windows.
fn stream_bytes(stream: &InMemoryRandomAccessStream) -> Result<Vec<u8>, String> {
    let size = stream.Size().map_err(|e| fail("no pude leer la página", e))? as u32;
    let input = stream.GetInputStreamAt(0).map_err(|e| fail("no pude leer la página", e))?;
    let reader = DataReader::CreateDataReader(&input).map_err(|e| fail("no pude leer la página", e))?;
    reader.LoadAsync(size).and_then(|op| op.join()).map_err(|e| fail("no pude leer la página", e))?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).map_err(|e| fail("no pude leer la página", e))?;
    Ok(bytes)
}

/// El texto de un PDF escaneado: cada página (hasta `MAX_PDF_PAGES`) se pinta como imagen y se lee con el OCR.
pub fn ocr_pdf(path: &Path) -> Result<OcrOutput, String> {
    let engine = engine()?;
    let absolute = std::fs::canonicalize(path).map_err(|e| fail("no pude abrir el PDF", e))?;
    let absolute = absolute.display().to_string();
    let absolute = absolute.strip_prefix(r"\\?\").unwrap_or(&absolute).to_string();
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(absolute)).and_then(|op| op.join()).map_err(|e| fail("no pude abrir el PDF", e))?;
    let document = PdfDocument::LoadFromFileAsync(&file)
        .and_then(|op| op.join())
        .map_err(|_| "no pude abrir el PDF: está dañado o protegido con contraseña".to_string())?;
    let total = document.PageCount().map_err(|e| fail("no pude leer el PDF", e))? as usize;
    let mut pages = Vec::new();
    for index in 0..total.min(MAX_PDF_PAGES) {
        let page = document.GetPage(index as u32).map_err(|e| fail("no pude leer una página", e))?;
        let options = PdfPageRenderOptions::new().map_err(|e| fail("no pude pintar la página", e))?;
        options.SetDestinationWidth(PAGE_RENDER_WIDTH).map_err(|e| fail("no pude pintar la página", e))?;
        let stream = InMemoryRandomAccessStream::new().map_err(|e| fail("no pude pintar la página", e))?;
        page.RenderWithOptionsToStreamAsync(&stream, &options).and_then(|op| op.join()).map_err(|e| fail("no pude pintar la página", e))?;
        let image = image::load_from_memory(&stream_bytes(&stream)?).map_err(|e| fail("no pude leer la página pintada", e))?;
        let text = ocr_decoded(&engine, &image)?.text;
        if !text.is_empty() {
            pages.push(text);
        }
    }
    let text = pages.join("\n\n");
    if text.trim().is_empty() {
        return Err("el OCR no encontró texto en el PDF".into());
    }
    Ok(OcrOutput { text, language: language_name(&engine), pages: total.min(MAX_PDF_PAGES), cut: total > MAX_PDF_PAGES })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::pdf_out::{images_to_pdf, JpegPage};

    pub(crate) fn fixture() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("ocr_texto.png")
    }

    /// Estas pruebas necesitan el OCR de Windows; si el equipo no lo tiene (o no tiene un idioma compatible) se saltan.
    pub(crate) fn engine_or_skip() -> Option<String> {
        match available() {
            Ok(language) => Some(language),
            Err(reason) => {
                eprintln!("prueba de OCR omitida: {reason}");
                None
            }
        }
    }

    fn assert_recognized(text: &str) {
        let flat = text.to_lowercase();
        for word in ["hola", "mundo", "2026", "prueba", "reconocimiento", "4521"] {
            assert!(flat.contains(word), "el OCR no leyó «{word}»: {text:?}");
        }
    }

    #[test]
    fn lee_el_texto_de_una_imagen() {
        let Some(language) = engine_or_skip() else { return };
        let output = ocr_image(&fixture()).unwrap();
        assert_recognized(&output.text);
        assert_eq!((output.pages, output.cut), (1, false));
        assert!(!language.is_empty() && output.language == language);
    }

    /// Un PDF que es solo una foto de texto, como el que sale de un escáner (sin capa de texto).
    pub(crate) fn write_scanned_pdf(path: &Path) {
        let image = image::open(fixture()).unwrap();
        let mut jpeg = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        std::fs::write(path, images_to_pdf(&[JpegPage { jpeg, width: image.width(), height: image.height() }])).unwrap();
    }

    #[test]
    fn lee_un_pdf_escaneado_que_no_tiene_texto() {
        let Some(_) = engine_or_skip() else { return };
        let path = std::env::temp_dir().join(format!("zen-ocr-{}-{:?}.pdf", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        write_scanned_pdf(&path);
        assert!(crate::office::pdf_text(&path).unwrap_err().starts_with(crate::office::NO_TEXT), "el PDF no tiene capa de texto");
        let output = ocr_pdf(&path).unwrap();
        assert_recognized(&output.text);
        assert_eq!((output.pages, output.cut), (1, false));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn un_pdf_roto_o_una_imagen_inexistente_dan_un_error_claro() {
        let Some(_) = engine_or_skip() else { return };
        let broken = std::env::temp_dir().join(format!("zen-ocr-roto-{}.pdf", std::process::id()));
        std::fs::write(&broken, b"%PDF-1.4 basura").unwrap();
        assert!(ocr_pdf(&broken).is_err());
        assert!(ocr_image(Path::new("C:\\no\\existe.png")).unwrap_err().contains("no pude abrir"));
        let _ = std::fs::remove_file(&broken);
    }

    #[test]
    fn una_imagen_sin_texto_da_texto_vacio_y_una_transparente_se_lee_sobre_blanco() {
        let Some(_) = engine_or_skip() else { return };
        let engine = engine().unwrap();
        let blank = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(600, 200, image::Rgb([255, 255, 255])));
        assert_eq!(ocr_decoded(&engine, &blank).unwrap().text, "");
        // El mismo texto con fondo transparente: debe leerse igual que con fondo blanco.
        let mut transparent = image::open(fixture()).unwrap().to_rgba8();
        for pixel in transparent.pixels_mut() {
            if pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240 {
                pixel[3] = 0;
            }
        }
        assert_recognized(&ocr_decoded(&engine, &DynamicImage::ImageRgba8(transparent)).unwrap().text);
    }

    #[test]
    fn el_ajuste_de_tamano_amplia_lo_pequeno_y_reduce_lo_enorme() {
        // No necesita el OCR: solo comprueba que los mapas de bits se crean con el tamaño esperado.
        ensure_initialized();
        let small = DynamicImage::ImageRgb8(image::RgbImage::new(400, 100));
        let bitmap = bitmap_from_image(&small).unwrap();
        assert_eq!((bitmap.PixelWidth().unwrap(), bitmap.PixelHeight().unwrap()), (1200, 300), "x3 como máximo");
        let normal = DynamicImage::ImageRgb8(image::RgbImage::new(1800, 900));
        assert_eq!(bitmap_from_image(&normal).unwrap().PixelWidth().unwrap(), 1800, "un tamaño normal no se toca");
        let huge = DynamicImage::ImageRgb8(image::RgbImage::new(11_000, 40));
        assert!(bitmap_from_image(&huge).unwrap().PixelWidth().unwrap() <= 10_000);
    }
}
