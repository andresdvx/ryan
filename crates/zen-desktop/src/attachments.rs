//! Imágenes que el usuario adjunta a un mensaje (una captura pegada, una ventana). Llegan del chat como
//! texto en base64, así que se comprueba todo antes de dárselas al modelo: tipo permitido, tamaño y que
//! el contenido sea de verdad una imagen PNG o JPEG (no un archivo cualquiera con otro nombre).

use base64::Engine;
use zen_core::ai::Image;

/// Imágenes por mensaje.
pub const MAX_IMAGES: usize = 3;
/// Tamaño máximo de cada imagen ya decodificada.
pub const MAX_IMAGE_BYTES: usize = 6 * 1024 * 1024;

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];

/// Comprueba una imagen recibida y la devuelve lista para enviarse al modelo.
pub fn validate_image(mime: &str, data: &str) -> Result<Image, String> {
    let data = data.trim();
    // El tamaño en base64 se mira antes de decodificar, para no reservar memoria por un texto enorme.
    if data.len() > MAX_IMAGE_BYTES * 4 / 3 + 8 {
        return Err("La imagen es demasiado grande.".into());
    }
    let magic = match mime {
        "image/png" => PNG_MAGIC,
        "image/jpeg" => JPEG_MAGIC,
        _ => return Err("Solo se admiten imágenes PNG o JPEG.".into()),
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "La imagen no es válida.".to_string())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("La imagen es demasiado grande.".into());
    }
    if !bytes.starts_with(magic) {
        return Err("El archivo no es una imagen del tipo indicado.".into());
    }
    Ok(Image { mime: mime.to_string(), base64: data.to_string() })
}

/// Valida todas las imágenes de un mensaje (`(tipo, contenido en base64)`).
pub fn validate_images(items: &[(String, String)]) -> Result<Vec<Image>, String> {
    if items.len() > MAX_IMAGES {
        return Err(format!("Solo puedes adjuntar hasta {MAX_IMAGES} imágenes por mensaje."));
    }
    items.iter().map(|(mime, data)| validate_image(mime, data)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn png() -> String {
        encode(&[PNG_MAGIC, b"resto de la imagen"].concat())
    }

    #[test]
    fn acepta_png_y_jpeg_reales() {
        let image = validate_image("image/png", &png()).unwrap();
        assert_eq!((image.mime.as_str(), image.base64.as_str()), ("image/png", png().as_str()));
        assert!(validate_image("image/jpeg", &encode(&[JPEG_MAGIC, b"datos"].concat())).is_ok());
        assert!(validate_image("image/png", &format!("  {}\n", png())).is_ok(), "los espacios alrededor se ignoran");
    }

    #[test]
    fn rechaza_tipos_contenido_falso_y_texto_roto() {
        assert!(validate_image("image/gif", &png()).is_err());
        assert!(validate_image("text/html", &png()).is_err());
        assert!(validate_image("image/png", "esto no es base64!!").is_err());
        assert!(validate_image("image/png", "").is_err());
        // Un archivo cualquiera con la etiqueta de imagen no pasa.
        assert!(validate_image("image/png", &encode(b"MZ ejecutable disfrazado")).is_err());
        assert!(validate_image("image/jpeg", &png()).is_err(), "el contenido debe coincidir con el tipo");
    }

    #[test]
    fn limita_el_tamano_y_la_cantidad() {
        let big = encode(&[PNG_MAGIC, &vec![0u8; MAX_IMAGE_BYTES]].concat());
        assert!(validate_image("image/png", &big).is_err());
        assert!(validate_image("image/png", &"A".repeat(MAX_IMAGE_BYTES * 2)).is_err(), "un texto enorme ni se decodifica");
        let ok = ("image/png".to_string(), png());
        assert_eq!(validate_images(&vec![ok.clone(); MAX_IMAGES]).unwrap().len(), MAX_IMAGES);
        assert!(validate_images(&vec![ok; MAX_IMAGES + 1]).is_err());
        assert!(validate_images(&[]).unwrap().is_empty());
    }
}
