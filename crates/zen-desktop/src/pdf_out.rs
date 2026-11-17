//! Escritor de PDF mínimo, sin dependencias: páginas A4 con texto (Helvetica y Courier, las fuentes que todo lector
//! de PDF trae) o con imágenes JPEG. Sirve para «convertir a PDF» desde texto, Markdown, HTML, Word, Excel o imágenes.
//!
//! Límite conocido: las fuentes estándar usan la codificación de Windows (Latin-1 más unos pocos símbolos), así que
//! tildes, eñes y la mayoría de signos salen bien, pero un carácter fuera de ella (emojis, chino, cirílico) sale como «?».

use crate::doc::{Block, Doc, DocImage};

const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 56.0;
const BODY_SIZE: f32 = 11.0;
const MONO_SIZE: f32 = 9.0;
const MONO_ADVANCE: f32 = 0.6;
/// Páginas como máximo: protege de un archivo enorme que llenaría el disco.
pub const MAX_PAGES: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Font {
    Regular,
    Bold,
    Mono,
}

impl Font {
    fn resource(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
            Font::Mono => "F3",
        }
    }
}

/// Anchos de Helvetica (milésimas de cuerpo) de los caracteres ASCII de 32 a 126.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584,
    556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278,
    469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260,
    334, 584,
];

/// Un carácter en la codificación de Windows (WinAnsi); `?` si no existe.
fn win_ansi(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{A0}'..='\u{FF}' => c as u32 as u8,
        '€' => 0x80,
        '‚' => 0x82,
        '„' => 0x84,
        '…' => 0x85,
        '†' => 0x86,
        '‡' => 0x87,
        '‰' => 0x89,
        '‹' => 0x8B,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' | '▪' | '●' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        '™' => 0x99,
        '›' => 0x9B,
        '\t' => b' ',
        _ => b'?',
    }
}

fn advance(c: char, font: Font, size: f32) -> f32 {
    let units = match font {
        Font::Mono => return size * MONO_ADVANCE,
        _ => match c {
            ' '..='~' => HELVETICA[c as usize - 32] as f32,
            _ => 556.0,
        },
    };
    let bold = if font == Font::Bold { 1.07 } else { 1.0 };
    units * bold * size / 1000.0
}

fn text_width(text: &str, font: Font, size: f32) -> f32 {
    text.chars().map(|c| advance(c, font, size)).sum()
}

/// Parte `text` en líneas que quepan en `max` puntos; una palabra más ancha que la línea se corta.
fn wrap(text: &str, font: Font, size: f32, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for hard in text.split('\n') {
        let mut line = String::new();
        let mut width = 0.0;
        for word in hard.split(' ') {
            let w = text_width(word, font, size);
            let space = advance(' ', font, size);
            if !line.is_empty() && width + space + w <= max {
                line.push(' ');
                line.push_str(word);
                width += space + w;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
            }
            if w <= max {
                line = word.to_string();
                width = w;
                continue;
            }
            for c in word.chars() {
                let cw = advance(c, font, size);
                if width + cw > max && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
                line.push(c);
                width += cw;
            }
        }
        lines.push(line);
    }
    lines
}

struct Item {
    font: Font,
    size: f32,
    x: f32,
    y: f32,
    text: String,
}

/// Una imagen colocada en una página.
struct Figure {
    image: usize,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct Layout<'a> {
    pages: Vec<Vec<Item>>,
    /// Las imágenes de cada página (en paralelo a `pages`) y las imágenes distintas del documento.
    figures: Vec<Vec<Figure>>,
    images: Vec<&'a DocImage>,
    /// Borde superior de la siguiente línea.
    cursor: f32,
}

impl<'a> Layout<'a> {
    fn new() -> Self {
        Self { pages: vec![Vec::new()], figures: vec![Vec::new()], images: Vec::new(), cursor: PAGE_H - MARGIN }
    }

    /// Pone una imagen a todo lo ancho que le deje la página (sin agrandarla) y la ajusta si es más alta que una hoja.
    fn figure(&mut self, image: &'a DocImage) {
        let (mut w, mut h) = image.size_pt();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let scale = ((PAGE_W - 2.0 * MARGIN) / w).min((PAGE_H - 2.0 * MARGIN - 4.0) / h).min(1.0);
        (w, h) = (w * scale, h * scale);
        self.gap(4.0);
        self.need(h);
        if self.pages.len() >= MAX_PAGES && self.cursor - h < MARGIN {
            return;
        }
        let index = match self.images.iter().position(|known| std::ptr::eq(*known, image)) {
            Some(index) => index,
            None => {
                self.images.push(image);
                self.images.len() - 1
            }
        };
        let y = self.cursor - h;
        self.figures.last_mut().expect("siempre hay una página").push(Figure { image: index, x: MARGIN, y, w, h });
        self.cursor -= h + 8.0;
    }

    fn at_top(&self) -> bool {
        self.cursor >= PAGE_H - MARGIN - 0.5
    }

    fn new_page(&mut self) {
        if self.pages.len() < MAX_PAGES {
            self.pages.push(Vec::new());
            self.figures.push(Vec::new());
        }
        self.cursor = PAGE_H - MARGIN;
    }

    /// Hace sitio para `height` puntos; si no caben, pasa a otra página.
    fn need(&mut self, height: f32) {
        if self.cursor - height < MARGIN && !self.at_top() {
            self.new_page();
        }
    }

    fn line(&mut self, text: &str, font: Font, size: f32, x: f32, leading: f32) {
        self.need(leading);
        if self.pages.len() >= MAX_PAGES && self.cursor - leading < MARGIN {
            return; // el documento es más largo de lo que se admite: se corta aquí
        }
        let y = self.cursor - size;
        self.pages.last_mut().expect("siempre hay una página").push(Item { font, size, x, y, text: text.to_string() });
        self.cursor -= leading;
    }

    fn gap(&mut self, points: f32) {
        if !self.at_top() {
            self.cursor -= points;
        }
    }
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 20.0,
        2 => 16.0,
        3 => 13.5,
        _ => 12.0,
    }
}

fn layout_doc(doc: &Doc) -> Layout<'_> {
    let mut layout = Layout::new();
    let width = PAGE_W - 2.0 * MARGIN;
    for block in doc {
        match block {
            Block::Heading(level, text) => {
                let size = heading_size(*level);
                layout.gap(10.0);
                layout.need(size * 1.25 * 2.0); // un título no se queda solo al final de la página
                for line in wrap(text, Font::Bold, size, width) {
                    layout.line(&line, Font::Bold, size, MARGIN, size * 1.25);
                }
                layout.gap(4.0);
            }
            Block::Para(text) => {
                for line in wrap(text, Font::Regular, BODY_SIZE, width) {
                    layout.line(&line, Font::Regular, BODY_SIZE, MARGIN, 15.0);
                }
                layout.gap(6.0);
            }
            Block::Bullet(text) => {
                let lines = wrap(text, Font::Regular, BODY_SIZE, width - 16.0);
                for (i, line) in lines.iter().enumerate() {
                    if i == 0 {
                        layout.need(15.0);
                        let y = layout.cursor - BODY_SIZE;
                        if let Some(page) = layout.pages.last_mut() {
                            page.push(Item { font: Font::Regular, size: BODY_SIZE, x: MARGIN + 2.0, y, text: "•".into() });
                        }
                    }
                    layout.line(line, Font::Regular, BODY_SIZE, MARGIN + 16.0, 15.0);
                }
                layout.gap(2.0);
            }
            Block::Pre(text) => {
                let max_chars = (width / (MONO_SIZE * MONO_ADVANCE)).floor() as usize;
                layout.gap(4.0);
                for hard in text.replace('\t', "    ").split('\n') {
                    let chars: Vec<char> = hard.chars().collect();
                    if chars.is_empty() {
                        layout.line("", Font::Mono, MONO_SIZE, MARGIN, 11.5);
                    }
                    for chunk in chars.chunks(max_chars.max(1)) {
                        layout.line(&chunk.iter().collect::<String>(), Font::Mono, MONO_SIZE, MARGIN, 11.5);
                    }
                }
                layout.gap(6.0);
            }
            Block::Image(image) => layout.figure(image),
            Block::Table(rows) => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                if columns == 0 {
                    continue;
                }
                let size = 8.5;
                let max_chars = (width / (size * MONO_ADVANCE)).floor() as usize;
                let cell_chars = ((max_chars.saturating_sub(3 * (columns - 1))) / columns).clamp(4, 40);
                let natural: Vec<usize> = (0..columns).map(|c| rows.iter().map(|r| r.get(c).map_or(0, |t| t.chars().count())).max().unwrap_or(0).clamp(1, cell_chars)).collect();
                layout.gap(4.0);
                for (r, row) in rows.iter().enumerate() {
                    let cells: Vec<String> = (0..columns)
                        .map(|c| {
                            let text: String = row.get(c).map_or("", String::as_str).replace('\n', " ");
                            let clipped: String = if text.chars().count() > natural[c] { text.chars().take(natural[c].saturating_sub(1)).chain(std::iter::once('…')).collect() } else { text };
                            format!("{clipped:<width$}", width = natural[c])
                        })
                        .collect();
                    layout.line(cells.join(" | ").trim_end(), Font::Mono, size, MARGIN, 11.0);
                    if r == 0 && rows.len() > 1 {
                        let rule: Vec<String> = natural.iter().map(|w| "-".repeat(*w)).collect();
                        layout.line(&rule.join("-+-"), Font::Mono, size, MARGIN, 11.0);
                    }
                }
                layout.gap(6.0);
            }
        }
    }
    layout
}

// ---------------------------------------------------------------------------------------------
// Estructura del archivo PDF
// ---------------------------------------------------------------------------------------------

struct Builder {
    objects: Vec<Vec<u8>>,
}

impl Builder {
    fn new() -> Self {
        Self { objects: Vec::new() }
    }

    fn add(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len()
    }

    fn reserve(&mut self) -> usize {
        self.add(Vec::new())
    }

    fn set(&mut self, id: usize, body: Vec<u8>) {
        self.objects[id - 1] = body;
    }

    fn finish(self) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.objects.len() + 1).as_bytes());
        out
    }
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

fn escape(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in text.chars() {
        match win_ansi(c) {
            b @ (b'(' | b')' | b'\\') => out.extend_from_slice(&[b'\\', b]),
            b @ 0x20..=0x7E => out.push(b),
            b => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
        }
    }
    out
}

/// Un documento con formato sencillo, en páginas A4.
pub fn doc_to_pdf(doc: &Doc) -> Vec<u8> {
    let layout = layout_doc(doc);
    let mut b = Builder::new();
    let catalog = b.reserve();
    let pages_id = b.reserve();
    let fonts: Vec<usize> = ["Helvetica", "Helvetica-Bold", "Courier"]
        .iter()
        .map(|name| b.add(format!("<< /Type /Font /Subtype /Type1 /BaseFont /{name} /Encoding /WinAnsiEncoding >>").into_bytes()))
        .collect();
    let mut kids = Vec::new();
    // Cada imagen distinta se incrusta una sola vez, aunque salga en varias páginas.
    let mut image_ids: Vec<Option<usize>> = vec![None; layout.images.len()];
    for (page, figures) in layout.pages.iter().zip(&layout.figures) {
        let mut content = Vec::new();
        let mut xobjects = String::new();
        for figure in figures {
            let id = *image_ids[figure.image].get_or_insert_with(|| {
                let image = layout.images[figure.image];
                b.add(stream(
                    &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", image.width, image.height),
                    &image.jpeg,
                ))
            });
            xobjects.push_str(&format!("/Im{} {id} 0 R ", figure.image));
            content.extend_from_slice(format!("q {:.2} 0 0 {:.2} {:.2} {:.2} cm /Im{} Do Q\n", figure.w, figure.h, figure.x, figure.y, figure.image).as_bytes());
        }
        for item in page {
            content.extend_from_slice(format!("BT /{} {:.2} Tf 1 0 0 1 {:.2} {:.2} Tm (", item.font.resource(), item.size, item.x, item.y).as_bytes());
            content.extend_from_slice(&escape(&item.text));
            content.extend_from_slice(b") Tj ET\n");
        }
        let content_id = b.add(stream("", &content));
        let page_id = b.add(
            format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 {} 0 R /F2 {} 0 R /F3 {} 0 R >> /XObject << {xobjects}>> >> /Contents {content_id} 0 R >>",
                fonts[0], fonts[1], fonts[2]
            )
            .into_bytes(),
        );
        kids.push(page_id);
    }
    b.set(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    let list: Vec<String> = kids.iter().map(|id| format!("{id} 0 R")).collect();
    b.set(pages_id, format!("<< /Type /Pages /Kids [{}] /Count {} >>", list.join(" "), kids.len()).into_bytes());
    b.finish()
}

/// Una imagen JPEG lista para incrustar: sus bytes y su tamaño en píxeles.
pub struct JpegPage {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Una página por imagen, del tamaño de un A4 (vertical u horizontal según la imagen) con la imagen ajustada.
pub fn images_to_pdf(images: &[JpegPage]) -> Vec<u8> {
    let mut b = Builder::new();
    let catalog = b.reserve();
    let pages_id = b.reserve();
    let mut kids = Vec::new();
    for image in images.iter().take(MAX_PAGES) {
        let (page_w, page_h) = if image.width > image.height { (PAGE_H, PAGE_W) } else { (PAGE_W, PAGE_H) };
        let frame = 28.0;
        let scale = ((page_w - 2.0 * frame) / image.width as f32).min((page_h - 2.0 * frame) / image.height as f32);
        let (w, h) = (image.width as f32 * scale, image.height as f32 * scale);
        let (x, y) = ((page_w - w) / 2.0, (page_h - h) / 2.0);
        let image_id = b.add(stream(
            &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", image.width, image.height),
            &image.jpeg,
        ));
        let content_id = b.add(stream("", format!("q {w:.2} 0 0 {h:.2} {x:.2} {y:.2} cm /Im0 Do Q").as_bytes()));
        let page_id = b.add(
            format!("<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {page_w} {page_h}] /Resources << /XObject << /Im0 {image_id} 0 R >> >> /Contents {content_id} 0 R >>").into_bytes(),
        );
        kids.push(page_id);
    }
    b.set(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    let list: Vec<String> = kids.iter().map(|id| format!("{id} 0 R")).collect();
    b.set(pages_id, format!("<< /Type /Pages /Kids [{}] /Count {} >>", list.join(" "), kids.len()).into_bytes());
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_tabla_de_anchos_cubre_el_ascii_imprimible() {
        assert_eq!(HELVETICA.len(), 95);
        assert_eq!(HELVETICA[b' ' as usize - 32], 278);
        assert_eq!(HELVETICA[b'W' as usize - 32], 944);
        assert_eq!(HELVETICA[b'i' as usize - 32], 222);
    }

    #[test]
    fn convierte_a_la_codificacion_de_windows() {
        assert_eq!(win_ansi('a'), b'a');
        assert_eq!(win_ansi('ñ'), 0xF1);
        assert_eq!(win_ansi('á'), 0xE1);
        assert_eq!(win_ansi('€'), 0x80);
        assert_eq!(win_ansi('—'), 0x97);
        assert_eq!(win_ansi('漢'), b'?');
        assert_eq!(win_ansi('😀'), b'?');
        assert_eq!(escape("a(b)\\ñ"), b"a\\(b\\)\\\\\\361");
    }

    #[test]
    fn el_ajuste_de_linea_respeta_el_ancho_y_corta_palabras_largas() {
        let lines = wrap("uno dos tres cuatro cinco seis siete ocho nueve diez", Font::Regular, 11.0, 80.0);
        assert!(lines.len() > 3);
        for line in &lines {
            assert!(text_width(line, Font::Regular, 11.0) <= 80.0 + 0.01, "{line:?}");
        }
        assert_eq!(lines.join(" "), "uno dos tres cuatro cinco seis siete ocho nueve diez");
        let long = wrap(&"x".repeat(100), Font::Regular, 11.0, 80.0);
        assert!(long.len() > 1 && long.iter().all(|l| text_width(l, Font::Regular, 11.0) <= 80.0 + 0.01));
        assert_eq!(wrap("a\n\nb", Font::Regular, 11.0, 80.0), ["a", "", "b"], "los saltos de línea se conservan");
    }

    #[test]
    fn el_pdf_es_valido_y_se_puede_leer_de_vuelta() {
        let doc = vec![
            Block::Heading(1, "Informe de pruebas".into()),
            Block::Para("Hola ñandú, ¿qué tal? Esto es una prueba de párrafo con acentos.".into()),
            Block::Bullet("Primer punto".into()),
            Block::Pre("let x = 1;".into()),
            Block::Table(vec![vec!["Nombre".into(), "Edad".into()], vec!["Ana".into(), "30".into()]]),
        ];
        let pdf = doc_to_pdf(&doc);
        assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
        let text = pdf_extract::extract_text_from_mem(&pdf).expect("el PDF generado debe poder leerse");
        for expected in ["Informe de pruebas", "Hola", "nd", "Primer punto", "let x = 1;", "Nombre", "Ana"] {
            assert!(text.contains(expected), "falta «{expected}» en {text:?}");
        }
    }

    #[test]
    fn un_documento_largo_ocupa_varias_paginas() {
        let doc: Doc = (0..200).map(|i| Block::Para(format!("Párrafo número {i} con algo de texto para ocupar sitio en la hoja."))).collect();
        let layout = layout_doc(&doc);
        assert!(layout.pages.len() >= 3, "páginas: {}", layout.pages.len());
        for page in &layout.pages {
            assert!(page.iter().all(|i| i.y >= MARGIN - 1.0 && i.y <= PAGE_H - MARGIN + 1.0), "nada se sale de los márgenes");
        }
        let pdf = doc_to_pdf(&doc);
        assert!(String::from_utf8_lossy(&pdf).contains(&format!("/Count {}", layout.pages.len())));
        assert_eq!(doc_to_pdf(&Vec::new()).windows(8).filter(|w| *w == b"/Type /P").count() >= 1, true, "un documento vacío da una página en blanco");
    }

    #[test]
    fn el_pdf_de_imagenes_lleva_una_pagina_por_imagen() {
        let img = || JpegPage { jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9], width: 400, height: 200 };
        let tall = JpegPage { jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9], width: 200, height: 400 };
        let pdf = images_to_pdf(&[img(), tall]);
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2") && text.contains("/DCTDecode"));
        assert!(text.contains("/MediaBox [0 0 842 595]") && text.contains("/MediaBox [0 0 595 842]"), "horizontal y vertical");
    }

    fn fake_image(width: u32, height: u32, extent: Option<(u32, u32)>) -> DocImage {
        DocImage { jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9], width, height, extent }
    }

    #[test]
    fn las_imagenes_de_un_documento_entran_en_la_pagina() {
        let doc: Doc = vec![
            Block::Para("Antes de la imagen".into()),
            Block::Image(fake_image(200, 100, None)),
            Block::Image(fake_image(4000, 1000, None)), // más ancha que la hoja: se reduce
            Block::Image(fake_image(100, 3000, None)),  // más alta que la hoja: se reduce y pasa de página
            Block::Para("Después".into()),
        ];
        let layout = layout_doc(&doc);
        let figures: Vec<&Figure> = layout.figures.iter().flatten().collect();
        assert_eq!(figures.len(), 3);
        for figure in &figures {
            assert!(figure.x >= MARGIN - 0.01 && figure.x + figure.w <= PAGE_W - MARGIN + 0.01, "cabe a lo ancho: {}", figure.w);
            assert!(figure.y >= MARGIN - 0.01 && figure.y + figure.h <= PAGE_H - MARGIN + 0.01, "cabe a lo alto");
        }
        assert!((figures[0].w - 150.0).abs() < 0.01 && (figures[0].h - 75.0).abs() < 0.01, "una imagen pequeña no se agranda");
        assert!(layout.pages.len() >= 2, "la imagen muy alta no cabe junto a las demás");
        let pdf = doc_to_pdf(&doc);
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("/Subtype /Image").count(), 3);
        assert!(text.contains("/XObject << /Im0") && text.contains("/DCTDecode") && text.contains(" Do Q"));
        assert_eq!(layout.figures.len(), layout.pages.len(), "una lista de imágenes por página");
    }

    #[test]
    fn el_tamano_original_de_word_manda_sobre_los_pixeles() {
        let from_word = fake_image(1000, 500, Some((2_540_000, 1_270_000))); // 200 x 100 pt
        assert_eq!(from_word.size_pt(), (200.0, 100.0));
        let doc: Doc = vec![Block::Image(from_word)];
        let layout = layout_doc(&doc);
        let figure = &layout.figures[0][0];
        assert!((figure.w - 200.0).abs() < 0.01 && (figure.h - 100.0).abs() < 0.01);
    }
}
