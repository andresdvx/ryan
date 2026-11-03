//! Captura de UNA ventana como imagen PNG reducida, para que un modelo con visión la mire.
//! Se usa `PrintWindow`, que dibuja la ventana aunque otra la tape (la propia mascota, por ejemplo).
//! Nada se guarda en disco: la imagen vive en memoria hasta que se envía al modelo.

use base64::Engine;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsIconic, IsWindow};

/// Lado mayor de la imagen que se envía. Más grande gasta más memoria y tiempo del modelo sin leerse mejor.
pub const MAX_SIDE: u32 = 1280;
/// Ventanas mayores que esto (por ejemplo, multimonitor) se rechazan antes de reservar memoria.
const MAX_CAPTURE_SIDE: i32 = 8192;
/// `PW_RENDERFULLCONTENT`: también captura contenido acelerado por GPU (navegadores, Spotify…).
const PW_RENDERFULLCONTENT: u32 = 2;
/// La imagen codificada no debería pasar de esto (base64 incluido llegaría a ~11 MB).
const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;

pub struct Shot {
    pub png: Vec<u8>,
    /// Tamaño de la imagen enviada.
    pub width: u32,
    pub height: u32,
    /// Tamaño real de la ventana.
    pub original: (u32, u32),
}

impl Shot {
    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.png)
    }
}

pub fn is_minimized(hwnd: isize) -> bool {
    // SAFETY: solo lectura sobre un identificador de ventana; si ya no existe devuelve falso.
    unsafe { IsIconic(hwnd as HWND) != 0 }
}

/// Dibuja la ventana en un mapa de bits y lo devuelve como píxeles RGB (3 bytes por píxel).
fn grab(hwnd: isize) -> Result<(Vec<u8>, u32, u32), String> {
    let hwnd = hwnd as HWND;
    // SAFETY: llamadas GDI estándar. Cada recurso que se crea (DC, mapa de bits) se libera siempre antes de salir.
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("La ventana ya no existe.".into());
        }
        if IsIconic(hwnd) != 0 {
            return Err("La ventana está minimizada; ábrela y vuelve a intentarlo.".into());
        }
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err("No pude medir la ventana.".into());
        }
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 || width > MAX_CAPTURE_SIDE || height > MAX_CAPTURE_SIDE {
            return Err(format!("La ventana tiene un tamaño que no puedo capturar ({width}×{height})."));
        }

        let screen = GetDC(std::ptr::null_mut());
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap);
        let painted = PrintWindow(hwnd, memory, PW_RENDERFULLCONTENT) != 0;

        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // negativo: filas de arriba abajo
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let mut bgra = vec![0u8; width as usize * height as usize * 4];
        let copied = painted && GetDIBits(memory, bitmap, 0, height as u32, bgra.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) != 0;

        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);

        if !copied {
            return Err("Windows no dejó capturar esa ventana.".into());
        }
        let rgb = bgra_to_rgb(&bgra);
        // Windows añade a cada lado un borde invisible (sombra y zona de redimensionar) que sale negro: se recorta.
        let mut frame: RECT = std::mem::zeroed();
        let measured = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS as u32, (&mut frame as *mut RECT).cast(), std::mem::size_of::<RECT>() as u32) == 0;
        if measured {
            let (x, y) = (frame.left - rect.left, frame.top - rect.top);
            let (visible_width, visible_height) = (frame.right - frame.left, frame.bottom - frame.top);
            if x >= 0 && y >= 0 && visible_width > 0 && visible_height > 0 && x + visible_width <= width && y + visible_height <= height {
                let cropped = crop(&rgb, width as u32, (x as u32, y as u32), (visible_width as u32, visible_height as u32));
                return Ok((cropped, visible_width as u32, visible_height as u32));
            }
        }
        Ok((rgb, width as u32, height as u32))
    }
}

/// Recorta un rectángulo (`origin`, `size`) de una imagen RGB de ancho `width`. El rectángulo debe caber en ella.
fn crop(rgb: &[u8], width: u32, origin: (u32, u32), size: (u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(size.0 as usize * size.1 as usize * 3);
    for row in origin.1..origin.1 + size.1 {
        let start = (row as usize * width as usize + origin.0 as usize) * 3;
        out.extend_from_slice(&rgb[start..start + size.0 as usize * 3]);
    }
    out
}

/// BGRA (como lo da GDI) a RGB. El canal alfa de `PrintWindow` no es fiable y se descarta.
fn bgra_to_rgb(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4).flat_map(|px| [px[2], px[1], px[0]]).collect()
}

/// Reduce la imagen para que su lado mayor no pase de `max_side`, promediando bloques de píxeles
/// (se lee mejor el texto pequeño que quedándose con un píxel de cada bloque).
pub fn downscale(rgb: &[u8], width: u32, height: u32, max_side: u32) -> (Vec<u8>, u32, u32) {
    let longest = width.max(height);
    if longest <= max_side || width == 0 || height == 0 {
        return (rgb.to_vec(), width, height);
    }
    let scale = max_side as f64 / longest as f64;
    let new_width = ((width as f64 * scale).round() as u32).max(1);
    let new_height = ((height as f64 * scale).round() as u32).max(1);
    let mut out = Vec::with_capacity(new_width as usize * new_height as usize * 3);
    for y in 0..new_height {
        let (y0, y1) = block(y, new_height, height);
        for x in 0..new_width {
            let (x0, x1) = block(x, new_width, width);
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy as usize * width as usize + sx as usize) * 3;
                    sum[0] += rgb[i] as u32;
                    sum[1] += rgb[i + 1] as u32;
                    sum[2] += rgb[i + 2] as u32;
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            out.extend(sum.iter().map(|channel| (channel / count) as u8));
        }
    }
    (out, new_width, new_height)
}

/// Rango de píxeles de origen que cubre el píxel `index` de destino (al menos uno).
fn block(index: u32, new_len: u32, old_len: u32) -> (u32, u32) {
    let start = (index as u64 * old_len as u64 / new_len as u64) as u32;
    let end = ((index as u64 + 1) * old_len as u64 / new_len as u64) as u32;
    (start.min(old_len - 1), end.clamp(start + 1, old_len))
}

pub fn encode_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| format!("no pude crear la imagen: {error}"))?;
    writer.write_image_data(rgb).map_err(|error| format!("no pude crear la imagen: {error}"))?;
    drop(writer);
    Ok(out)
}

/// Una ventana toda negra suele ser contenido protegido o que Windows no deja dibujar: no sirve de nada enviarla.
fn is_blank(rgb: &[u8]) -> bool {
    rgb.iter().all(|&byte| byte == 0)
}

pub fn capture_window(hwnd: isize) -> Result<Shot, String> {
    let (rgb, width, height) = grab(hwnd)?;
    if is_blank(&rgb) {
        return Err("La captura salió completamente negra: esa ventana no se deja capturar (contenido protegido o minimizado).".into());
    }
    let (small, new_width, new_height) = downscale(&rgb, width, height, MAX_SIDE);
    let png = encode_png(&small, new_width, new_height)?;
    if png.len() > MAX_PNG_BYTES {
        return Err("La captura pesa demasiado para enviarla.".into());
    }
    Ok(Shot { png, width: new_width, height: new_height, original: (width, height) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, color: [u8; 3]) -> Vec<u8> {
        (0..width * height).flat_map(|_| color).collect()
    }

    #[test]
    fn convierte_bgra_a_rgb_y_descarta_el_alfa() {
        assert_eq!(bgra_to_rgb(&[1, 2, 3, 255, 10, 20, 30, 0]), [3, 2, 1, 30, 20, 10]);
    }

    #[test]
    fn recorta_un_rectangulo_de_la_imagen() {
        // 3×2 píxeles numerados del 0 al 5 (en el canal rojo).
        let rgb: Vec<u8> = (0..6u8).flat_map(|i| [i, 0, 0]).collect();
        assert_eq!(crop(&rgb, 3, (1, 0), (2, 2)), [1, 0, 0, 2, 0, 0, 4, 0, 0, 5, 0, 0]);
        assert_eq!(crop(&rgb, 3, (0, 1), (3, 1)), [3, 0, 0, 4, 0, 0, 5, 0, 0]);
    }

    #[test]
    fn una_imagen_pequena_no_se_toca() {
        let rgb = solid(100, 50, [9, 9, 9]);
        assert_eq!(downscale(&rgb, 100, 50, 1280), (rgb, 100, 50));
    }

    #[test]
    fn reduce_manteniendo_la_proporcion_y_promedia_los_colores() {
        let (out, w, h) = downscale(&solid(2560, 1440, [200, 100, 50]), 2560, 1440, 1280);
        assert_eq!((w, h), (1280, 720));
        assert_eq!(out.len(), 1280 * 720 * 3);
        assert!(out.chunks(3).all(|px| px == [200, 100, 50]));
        // Un tablero blanco y negro de 1 píxel promedia a gris.
        let board: Vec<u8> = (0..4u32 * 4).flat_map(|i| if (i % 4 + i / 4) % 2 == 0 { [255; 3] } else { [0; 3] }).collect();
        let (gray, w, h) = downscale(&board, 4, 4, 2);
        assert_eq!((w, h), (2, 2));
        assert!(gray.iter().all(|&v| v == 127), "{gray:?}");
    }

    #[test]
    fn las_proporciones_raras_no_dan_tamanos_cero() {
        let (out, w, h) = downscale(&solid(5000, 3, [1, 2, 3]), 5000, 3, 1280);
        assert_eq!((w, h), (1280, 1));
        assert_eq!(out.len(), 1280 * 3);
    }

    #[test]
    fn el_png_es_valido_y_se_puede_leer_de_vuelta() {
        let rgb: Vec<u8> = (0..16u32 * 8).flat_map(|i| [(i % 256) as u8, 7, 200]).collect();
        let bytes = encode_png(&rgb, 16, 8).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        assert_eq!((info.width, info.height), (16, 8));
        assert_eq!(&buffer[..info.buffer_size()], rgb.as_slice());
    }

    #[test]
    fn una_captura_negra_se_detecta() {
        assert!(is_blank(&solid(4, 4, [0, 0, 0])));
        assert!(!is_blank(&solid(4, 4, [0, 0, 1])));
    }

    /// Captura de verdad la ventana cuyo título contiene `ZEN_TEST_WINDOW` y guarda el PNG en `ZEN_TEST_OUT`.
    /// Se ejecuta con `cargo test -p zen-desktop -- --ignored --nocapture captura_real`.
    #[test]
    #[ignore = "captura una ventana real"]
    fn captura_real_de_una_ventana() {
        let query = std::env::var("ZEN_TEST_WINDOW").expect("falta ZEN_TEST_WINDOW");
        let out = std::env::var("ZEN_TEST_OUT").expect("falta ZEN_TEST_OUT");
        let windows = crate::input::visible_windows();
        let window = crate::input::find_windows(&windows, &query).into_iter().next().expect("no hay ninguna ventana con ese título");
        let shot = capture_window(window.hwnd).expect("la captura debería funcionar");
        println!("«{}»: ventana {}×{} -> imagen {}×{} ({} KB)", window.title, shot.original.0, shot.original.1, shot.width, shot.height, shot.png.len() / 1024);
        std::fs::write(out, &shot.png).unwrap();
    }

    #[test]
    fn una_ventana_inexistente_da_error_claro() {
        assert!(capture_window(0x7fff_fff0).is_err());
    }
}
