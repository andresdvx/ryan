//! Lectura de documentos de oficina: Word (.docx), PowerPoint (.pptx), Excel (.xlsx y compatibles) y PDF.
//! Los .docx y .pptx son zips de XML; aquí se lee solo el texto visible. Para Excel se usa `calamine` y para PDF
//! `pdf-extract`. Nunca se ejecuta nada del archivo: solo se extrae texto.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use crate::doc::{decode_entities, Block, Doc, DocImage};

/// Un XML interno más grande que esto se rechaza (un zip puede esconder gigas tras un archivo pequeño).
const MAX_XML_BYTES: u64 = 48 * 1024 * 1024;
/// Filas y columnas que se leen de cada hoja de Excel.
pub const MAX_ROWS: usize = 5000;
pub const MAX_COLS: usize = 100;
/// Cómo empieza el error de un PDF sin capa de texto (un escaneo): `incoming` lo reconoce para probar con el OCR.
pub const NO_TEXT: &str = "el PDF no tiene texto seleccionable";
/// Un PDF más grande que esto no se lee.
const MAX_PDF_BYTES: u64 = 80 * 1024 * 1024;

/// Imágenes que se leen de un documento como mucho, y tamaño máximo de cada una.
const MAX_DOC_IMAGES: usize = 200;
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

pub type Sheet = (String, Vec<Vec<String>>);

fn open_zip(path: &Path) -> Result<ZipArchive<File>, String> {
    let file = File::open(path).map_err(|e| format!("no pude abrir el archivo: {e}"))?;
    ZipArchive::new(file).map_err(|_| "el archivo está dañado o no es del tipo que dice ser".to_string())
}

fn read_entry(archive: &mut ZipArchive<File>, name: &str) -> Result<String, String> {
    let entry = archive.by_name(name).map_err(|_| format!("no encuentro «{name}» dentro del archivo"))?;
    if entry.size() > MAX_XML_BYTES {
        return Err("el contenido del documento es demasiado grande".into());
    }
    let mut text = String::new();
    entry.take(MAX_XML_BYTES).read_to_string(&mut text).map_err(|_| format!("«{name}» no es texto válido"))?;
    Ok(text)
}

// ---------------------------------------------------------------------------------------------
// XML mínimo
// ---------------------------------------------------------------------------------------------

enum Tok<'a> {
    Open { name: &'a str, attrs: &'a str, empty: bool },
    Close(&'a str),
    Text(&'a str),
}

fn tokens(xml: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut rest = xml;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            out.push(Tok::Text(rest));
            break;
        };
        if open > 0 {
            out.push(Tok::Text(&rest[..open]));
        }
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            let end = after.find("]]>").unwrap_or(after.len());
            out.push(Tok::Text(&after[..end]));
            rest = after.get(end + 3..).unwrap_or("");
            continue;
        }
        let Some(close) = rest.find('>') else { break };
        let inner = &rest[1..close];
        rest = &rest[close + 1..];
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Tok::Close(name.trim()));
            continue;
        }
        let empty = inner.ends_with('/');
        let inner = inner.trim_end_matches('/');
        let (name, attrs) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
        out.push(Tok::Open { name, attrs, empty });
    }
    out
}

fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("{key}=\"");
    let start = attrs.find(&pattern)? + pattern.len();
    let end = attrs[start..].find('"')?;
    Some(&attrs[start..start + end])
}

/// Nivel de título de un estilo de Word («Heading2», «Ttulo2», «Title»…), si lo es.
fn heading_level(style: &str) -> Option<u8> {
    let lower = style.to_lowercase();
    if lower == "title" || lower == "ttulo" || lower == "titulo" {
        return Some(1);
    }
    let named = ["heading", "ttulo", "titulo", "título"].iter().any(|prefix| lower.starts_with(prefix));
    let digits: String = lower.chars().filter(char::is_ascii_digit).collect();
    if named {
        digits.parse::<u8>().ok().map(|n| n.clamp(1, 6))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Word y PowerPoint
// ---------------------------------------------------------------------------------------------

/// Qué archivo de `word/` es cada imagen del documento (`rId5` → `word/media/image1.png`).
fn docx_images(archive: &mut ZipArchive<File>) -> HashMap<String, String> {
    let Ok(xml) = read_entry(archive, "word/_rels/document.xml.rels") else { return HashMap::new() };
    let mut map = HashMap::new();
    for tok in tokens(&xml) {
        let Tok::Open { name: "Relationship", attrs, .. } = tok else { continue };
        let (Some(id), Some(target), Some(kind)) = (attr(attrs, "Id"), attr(attrs, "Target"), attr(attrs, "Type")) else { continue };
        if !kind.ends_with("/image") || attr(attrs, "TargetMode") == Some("External") {
            continue;
        }
        let path = match target.strip_prefix('/') {
            Some(absolute) => absolute.to_string(),
            None => format!("word/{target}"),
        };
        if !path.contains("..") {
            map.insert(id.to_string(), path);
        }
    }
    map
}

/// Una imagen de dentro del .docx, o `None` si no se puede leer (un formato raro, demasiado grande…).
fn docx_image(archive: &mut ZipArchive<File>, path: &str, extent: Option<(u32, u32)>) -> Option<DocImage> {
    let mut entry = archive.by_name(path).ok()?;
    if entry.size() > MAX_IMAGE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    (&mut entry).take(MAX_IMAGE_BYTES).read_to_end(&mut bytes).ok()?;
    crate::convert::doc_image_from_bytes(&bytes, extent)
}

pub fn docx_doc(path: &Path) -> Result<Doc, String> {
    let mut archive = open_zip(path)?;
    let xml = read_entry(&mut archive, "word/document.xml")?;
    let image_files = docx_images(&mut archive);
    let mut extent: Option<(u32, u32)> = None;
    let mut found_images = 0usize;
    // Word guarda muchas imágenes y cuadros dos veces: la versión moderna y otra de respaldo para programas antiguos.
    let mut in_fallback = false;
    // Las imágenes de un párrafo van justo después de su texto; las de una tabla, después de la tabla.
    let mut para_images: Vec<DocImage> = Vec::new();
    let mut table_images: Vec<DocImage> = Vec::new();
    let mut doc = Doc::new();
    let mut para = String::new();
    let mut level: Option<u8> = None;
    let mut bullet = false;
    let mut in_text = false;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    for tok in tokens(&xml) {
        match &tok {
            Tok::Open { name: "mc:Fallback", empty: false, .. } => {
                in_fallback = true;
                continue;
            }
            Tok::Close("mc:Fallback") => {
                in_fallback = false;
                continue;
            }
            _ if in_fallback => continue,
            _ => {}
        }
        match tok {
            Tok::Open { name: "wp:extent", attrs, .. } => {
                extent = attr(attrs, "cx").and_then(|v| v.parse().ok()).zip(attr(attrs, "cy").and_then(|v| v.parse().ok()));
            }
            Tok::Open { name: "a:blip", attrs, .. } | Tok::Open { name: "v:imagedata", attrs, .. } => {
                let id = attr(attrs, "r:embed").or_else(|| attr(attrs, "r:id"));
                if let Some(file) = id.and_then(|id| image_files.get(id)) {
                    if found_images < MAX_DOC_IMAGES {
                        if let Some(image) = docx_image(&mut archive, file, extent.take()) {
                            found_images += 1;
                            if table.is_some() { table_images.push(image) } else { para_images.push(image) }
                        }
                    }
                }
            }
            Tok::Open { name: "w:p", empty, .. } => {
                para.clear();
                level = None;
                bullet = false;
                extent = None;
                if empty {
                    continue;
                }
            }
            Tok::Open { name: "w:pStyle", attrs, .. } => level = attr(attrs, "w:val").and_then(heading_level),
            Tok::Open { name: "w:numPr", .. } => bullet = true,
            Tok::Open { name: "w:t", empty: false, .. } => in_text = true,
            Tok::Close("w:t") => in_text = false,
            Tok::Open { name: "w:tab", .. } => para.push('\t'),
            Tok::Open { name: "w:br" | "w:cr", .. } => para.push('\n'),
            Tok::Text(text) if in_text => para.push_str(&decode_entities(text)),
            Tok::Close("w:p") => {
                let text = para.trim().to_string();
                if table.is_some() {
                    if !text.is_empty() {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&text.replace('\n', " "));
                    }
                } else {
                    if !text.is_empty() {
                        doc.push(match (level, bullet) {
                            (Some(level), _) => Block::Heading(level, text),
                            (None, true) => Block::Bullet(text),
                            _ => Block::Para(text),
                        });
                    }
                    doc.extend(para_images.drain(..).map(Block::Image));
                }
                para_images.clear();
                para.clear();
            }
            Tok::Open { name: "w:tbl", .. } => table = Some(Vec::new()),
            Tok::Close("w:tbl") => {
                if let Some(rows) = table.take() {
                    doc.push(Block::Table(rows));
                }
                doc.extend(table_images.drain(..).map(Block::Image));
            }
            Tok::Open { name: "w:tr", .. } => row = Vec::new(),
            Tok::Close("w:tr") => {
                if let Some(rows) = table.as_mut() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            Tok::Open { name: "w:tc", .. } => cell.clear(),
            Tok::Close("w:tc") => row.push(std::mem::take(&mut cell)),
            _ => {}
        }
    }
    if doc.is_empty() {
        return Err("el documento no tiene texto ni imágenes".into());
    }
    Ok(doc)
}

pub fn pptx_doc(path: &Path) -> Result<Doc, String> {
    let mut archive = open_zip(path)?;
    let mut slides: Vec<(u32, String)> = Vec::new();
    for index in 0..archive.len() {
        let Ok(entry) = archive.by_index(index) else { continue };
        let Ok(name) = entry.name().map(|n| n.to_string()) else { continue };
        let number = name.strip_prefix("ppt/slides/slide").and_then(|rest| rest.strip_suffix(".xml")).and_then(|n| n.parse::<u32>().ok());
        if let Some(number) = number {
            slides.push((number, name));
        }
    }
    slides.sort();
    let mut doc = Doc::new();
    for (number, name) in slides {
        let xml = read_entry(&mut archive, &name)?;
        doc.push(Block::Heading(2, format!("Diapositiva {number}")));
        let mut para = String::new();
        let mut in_text = false;
        for tok in tokens(&xml) {
            match tok {
                Tok::Open { name: "a:p", .. } => para.clear(),
                Tok::Open { name: "a:t", empty: false, .. } => in_text = true,
                Tok::Close("a:t") => in_text = false,
                Tok::Open { name: "a:br", .. } => para.push('\n'),
                Tok::Text(text) if in_text => para.push_str(&decode_entities(text)),
                Tok::Close("a:p") => {
                    if !para.trim().is_empty() {
                        doc.push(Block::Para(para.trim().to_string()));
                    }
                    para.clear();
                }
                _ => {}
            }
        }
    }
    if doc.iter().all(|b| matches!(b, Block::Heading(..))) {
        return Err("la presentación no tiene texto".into());
    }
    Ok(doc)
}

// ---------------------------------------------------------------------------------------------
// Excel
// ---------------------------------------------------------------------------------------------

fn cell_text(cell: &calamine::Data) -> String {
    use calamine::Data;
    match cell {
        Data::Empty => String::new(),
        Data::String(text) => text.clone(),
        Data::Float(number) if number.fract() == 0.0 && number.abs() < 1e15 => format!("{}", *number as i64),
        other => other.to_string(),
    }
}

/// Las hojas de un libro de Excel como filas de texto (con tope de filas y columnas, sin filas vacías al final).
pub fn sheets(path: &Path) -> Result<Vec<Sheet>, String> {
    use calamine::{open_workbook_auto, Reader};
    let mut workbook = open_workbook_auto(path).map_err(|e| format!("no pude abrir el libro de Excel: {e}"))?;
    let mut out = Vec::new();
    for name in workbook.sheet_names() {
        let range = workbook.worksheet_range(&name).map_err(|e| format!("no pude leer la hoja «{name}»: {e}"))?;
        let mut rows: Vec<Vec<String>> = range.rows().take(MAX_ROWS).map(|row| row.iter().take(MAX_COLS).map(cell_text).collect()).collect();
        while rows.last().is_some_and(|row| row.iter().all(String::is_empty)) {
            rows.pop();
        }
        out.push((name, rows));
    }
    if out.iter().all(|(_, rows)| rows.is_empty()) {
        return Err("el libro de Excel está vacío".into());
    }
    Ok(out)
}

/// Un libro de Excel como documento: un título por hoja y su tabla.
pub fn xlsx_doc(path: &Path) -> Result<Doc, String> {
    let mut doc = Doc::new();
    let sheets = sheets(path)?;
    let several = sheets.len() > 1;
    for (name, rows) in sheets {
        if several {
            doc.push(Block::Heading(2, name));
        }
        if !rows.is_empty() {
            doc.push(Block::Table(rows));
        }
    }
    Ok(doc)
}

// ---------------------------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------------------------

/// El texto de un PDF. Un PDF escaneado (solo imágenes) no tiene texto que extraer.
pub fn pdf_text(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("no pude abrir el PDF: {e}"))?.len();
    if size > MAX_PDF_BYTES {
        return Err("el PDF es demasiado grande (más de 80 MB)".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("no pude leer el PDF: {e}"))?;
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes));
    let text = match result {
        Ok(Ok(text)) => text,
        Ok(Err(error)) => return Err(format!("no pude leer el PDF ({error}); puede estar protegido con contraseña o dañado")),
        Err(_) => return Err("este PDF tiene un formato que no sé leer".into()),
    };
    let mut cleaned = String::new();
    let mut blank = 0;
    for line in text.replace("\r\n", "\n").lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank == 1 {
                cleaned.push('\n');
            }
        } else {
            blank = 0;
            cleaned.push_str(line);
            cleaned.push('\n');
        }
    }
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        return Err(format!("{NO_TEXT} (parece un escaneo)"));
    }
    Ok(cleaned)
}

/// El texto de un PDF. Si no tiene capa de texto (un escaneo), se prueba con el OCR de Windows; en ese caso se devuelve
/// también una nota para el usuario que dice cómo se leyó.
pub fn pdf_text_or_ocr(path: &Path) -> Result<(String, Option<String>), String> {
    match pdf_text(path) {
        Ok(text) => Ok((text, None)),
        Err(error) if error.starts_with(NO_TEXT) => match crate::ocr::ocr_pdf(path) {
            Ok(output) => {
                let mut note = format!("Era un PDF escaneado: leí su texto con el OCR de Windows ({}).", output.language);
                if output.cut {
                    note.push_str(&format!(" Solo leí las primeras {} páginas.", crate::ocr::MAX_PDF_PAGES));
                }
                Ok((output.text, Some(note)))
            }
            Err(ocr_error) => Err(format!("{error}. Probé con el OCR de Windows y tampoco pude: {ocr_error}")),
        },
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zen-office-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, body) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn lee_titulos_parrafos_listas_y_tablas_de_un_docx() {
        let path = temp_file("prueba.docx");
        let xml = r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Informe &amp; datos</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">Hola </w:t></w:r><w:r><w:t>mundo</w:t></w:r></w:p>
            <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>Punto uno</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Nombre</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Edad</w:t></w:r></w:p></w:tc></w:tr>
            <w:tr><w:tc><w:p><w:r><w:t>Ana</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>30</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            <w:p><w:pPr><w:pStyle w:val="Ttulo2"/></w:pPr><w:r><w:t>Final</w:t></w:r></w:p>
            </w:body></w:document>"#;
        write_zip(&path, &[("word/document.xml", xml)]);
        let doc = docx_doc(&path).unwrap();
        assert_eq!(doc[0], Block::Heading(1, "Informe & datos".into()));
        assert_eq!(doc[1], Block::Para("Hola mundo".into()));
        assert_eq!(doc[2], Block::Bullet("Punto uno".into()));
        assert_eq!(doc[3], Block::Table(vec![vec!["Nombre".into(), "Edad".into()], vec!["Ana".into(), "30".into()]]));
        assert_eq!(doc[4], Block::Heading(2, "Final".into()));
        assert_eq!(doc.len(), 5);
    }

    #[test]
    fn un_docx_roto_o_vacio_da_un_error_claro() {
        let path = temp_file("roto.docx");
        std::fs::write(&path, b"esto no es un zip").unwrap();
        assert!(docx_doc(&path).unwrap_err().contains("dañado"));
        let empty = temp_file("vacio.docx");
        write_zip(&empty, &[("word/document.xml", "<w:document><w:body/></w:document>")]);
        assert!(docx_doc(&empty).unwrap_err().contains("no tiene texto"));
        let wrong = temp_file("otro.docx");
        write_zip(&wrong, &[("otra/cosa.xml", "<a/>")]);
        assert!(docx_doc(&wrong).unwrap_err().contains("no encuentro"));
    }

    #[test]
    fn lee_el_texto_de_cada_diapositiva_en_orden() {
        let path = temp_file("prueba.pptx");
        let slide = |text: &str| format!(r#"<p:sld><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:sld>"#);
        write_zip(&path, &[("ppt/slides/slide10.xml", &slide("Diez")), ("ppt/slides/slide2.xml", &slide("Dos")), ("ppt/slides/slide1.xml", &slide("Uno"))]);
        let doc = pptx_doc(&path).unwrap();
        let texts: Vec<String> = doc.iter().map(|b| match b { Block::Heading(_, t) | Block::Para(t) => t.clone(), _ => String::new() }).collect();
        assert_eq!(texts, ["Diapositiva 1", "Uno", "Diapositiva 2", "Dos", "Diapositiva 10", "Diez"]);
    }

    #[test]
    fn lee_las_hojas_de_un_xlsx() {
        let path = temp_file("prueba.xlsx");
        write_zip(
            &path,
            &[
                ("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#),
                ("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#),
                ("xl/workbook.xml", r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Ventas" sheetId="1" r:id="rId1"/></sheets></workbook>"#),
                ("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#),
                ("xl/worksheets/sheet1.xml", r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Producto</t></is></c><c r="B1" t="inlineStr"><is><t>Precio</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Café</t></is></c><c r="B2"><v>3.5</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Té</t></is></c><c r="B3"><v>12</v></c></row></sheetData></worksheet>"#),
            ],
        );
        let sheets = sheets(&path).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].0, "Ventas");
        assert_eq!(sheets[0].1, vec![vec!["Producto", "Precio"], vec!["Café", "3.5"], vec!["Té", "12"]]);
        let doc = xlsx_doc(&path).unwrap();
        assert!(matches!(&doc[0], Block::Table(rows) if rows.len() == 3), "una sola hoja no lleva título");
    }

    #[test]
    fn un_pdf_se_lee_y_uno_roto_da_un_error_claro() {
        let path = temp_file("prueba.pdf");
        let pdf = crate::pdf_out::doc_to_pdf(&vec![Block::Para("Contenido de prueba para extraer".into())]);
        std::fs::write(&path, pdf).unwrap();
        assert!(pdf_text(&path).unwrap().contains("Contenido de prueba"));
        let broken = temp_file("roto.pdf");
        std::fs::write(&broken, b"%PDF-1.4 basura").unwrap();
        assert!(pdf_text(&broken).is_err());
        let blank = temp_file("blanco.pdf");
        std::fs::write(&blank, crate::pdf_out::doc_to_pdf(&Vec::new())).unwrap();
        assert!(pdf_text(&blank).unwrap_err().starts_with(NO_TEXT), "un PDF sin texto se reconoce para probar con el OCR");
    }

    #[test]
    fn el_xml_se_trocea_sin_romperse_con_comentarios_y_cdata() {
        let toks = tokens("<?xml?><a x=\"1\"><!-- no --><b/>hola<![CDATA[<raw>]]></a>");
        let kinds: Vec<String> = toks
            .iter()
            .map(|t| match t {
                Tok::Open { name, empty, .. } => format!("open:{name}:{empty}"),
                Tok::Close(name) => format!("close:{name}"),
                Tok::Text(text) => format!("text:{text}"),
            })
            .collect();
        assert_eq!(kinds, ["open:a:false", "open:b:true", "text:hola", "text:<raw>", "close:a"]);
        assert_eq!(attr("w:val=\"Heading2\" x=\"1\"", "w:val"), Some("Heading2"));
        assert_eq!(heading_level("Heading3"), Some(3));
        assert_eq!(heading_level("Ttulo1"), Some(1));
        assert_eq!(heading_level("Normal"), None);
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]))).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        bytes
    }

    /// Un .docx con texto e imágenes, como el que crea Word: las imágenes se enlazan desde `document.xml` por su `rId`.
    pub(crate) fn docx_with_image(path: &Path) {
        let document = r#"<?xml version="1.0"?><w:document><w:body>
            <w:p><w:r><w:t>Antes</w:t></w:r></w:p>
            <w:p><w:r><w:drawing><wp:inline><wp:extent cx="1905000" cy="952500"/><a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip r:embed="rId5"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
            <w:p><w:r><w:t>Después</w:t></w:r></w:p>
            <w:p><w:r><mc:AlternateContent><mc:Choice><w:drawing><wp:inline><a:blip r:embed="rId5"/></wp:inline></w:drawing></mc:Choice><mc:Fallback><w:pict><v:imagedata r:id="rId5"/></w:pict><w:t>texto duplicado</w:t></mc:Fallback></mc:AlternateContent></w:r></w:p>
            <w:p><w:r><w:t>Con texto </w:t></w:r><w:r><w:drawing><wp:inline><a:blip r:embed="rId9"/></wp:inline></w:drawing></w:r></w:p>
            </w:body></w:document>"#;
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="x"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="http://x" TargetMode="External"/></Relationships>"#;
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, body) in [("word/document.xml", document.as_bytes().to_vec()), ("word/_rels/document.xml.rels", rels.as_bytes().to_vec()), ("word/media/image1.png", png_bytes(80, 40))] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&body).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn lee_las_imagenes_de_un_docx_en_su_sitio() {
        let path = temp_file("conimagen.docx");
        docx_with_image(&path);
        let doc = docx_doc(&path).unwrap();
        assert_eq!(doc.len(), 5, "{doc:?}");
        assert_eq!(doc[0], Block::Para("Antes".into()));
        match &doc[1] {
            Block::Image(image) => {
                assert_eq!((image.width, image.height, image.extent), (80, 40, Some((1_905_000, 952_500))));
                assert!(image.jpeg.starts_with(&[0xFF, 0xD8]), "se guarda como JPEG");
                assert_eq!(image.size_pt(), (150.0, 75.0), "el tamaño que tenía en Word");
            }
            other => panic!("se esperaba una imagen, salió {other:?}"),
        }
        assert_eq!(doc[2], Block::Para("Después".into()));
        assert!(matches!(&doc[3], Block::Image(_)), "una imagen con versión de respaldo cuenta una sola vez, y su texto de respaldo no se duplica");
        assert_eq!(doc[4], Block::Para("Con texto".into()), "una imagen que no se encuentra se ignora sin romper nada");
    }

    #[test]
    fn un_docx_solo_con_imagenes_no_se_considera_vacio() {
        let path = temp_file("soloimagen.docx");
        let document = r#"<w:document><w:body><w:p><w:r><w:drawing><a:blip r:embed="rId1"/></w:drawing></w:r></w:p></w:body></w:document>"#;
        let rels = r#"<Relationships><Relationship Id="rId1" Type="http://x/relationships/image" Target="media/a.png"/></Relationships>"#;
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        for (name, body) in [("word/document.xml", document.as_bytes().to_vec()), ("word/_rels/document.xml.rels", rels.as_bytes().to_vec()), ("word/media/a.png", png_bytes(10, 10))] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(&body).unwrap();
        }
        zip.finish().unwrap();
        let doc = docx_doc(&path).unwrap();
        assert!(matches!(doc.as_slice(), [Block::Image(image)] if image.extent.is_none() && image.size_pt() == (7.5, 7.5)));
    }
}
