//! Un documento sencillo en memoria: títulos, párrafos, viñetas, código y tablas. Todo lo que Ryan sabe leer
//! (texto, Markdown, HTML, Word, PDF, Excel…) se convierte a esto, y de esto salen los formatos de salida
//! (texto, Markdown, HTML y PDF). Así cada formato nuevo solo necesita un lector o un escritor, no una pareja
//! por cada combinación.

use base64::Engine as _;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Título de nivel 1 (el mayor) a 6.
    Heading(u8, String),
    /// Párrafo; los saltos de línea (`\n`) dentro de él se conservan.
    Para(String),
    Bullet(String),
    /// Texto que debe verse tal cual (código, JSON, registros).
    Pre(String),
    Table(Vec<Vec<String>>),
    /// Una imagen incrustada (de un Word, por ejemplo).
    Image(DocImage),
}

/// Una imagen de un documento, ya como JPEG (con fondo blanco si era transparente) y reducida si era enorme.
#[derive(Clone, PartialEq, Eq)]
pub struct DocImage {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Tamaño con el que el documento original la mostraba, en EMU (914 400 por pulgada), si lo decía.
    pub extent: Option<(u32, u32)>,
}

impl std::fmt::Debug for DocImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DocImage({}x{}, {} bytes)", self.width, self.height, self.jpeg.len())
    }
}

impl DocImage {
    /// Tamaño en puntos (1/72 de pulgada): el del documento original o, si no lo decía, el de sus píxeles a 96 ppp.
    pub fn size_pt(&self) -> (f32, f32) {
        match self.extent {
            Some((cx, cy)) if cx > 0 && cy > 0 => (cx as f32 / 12_700.0, cy as f32 / 12_700.0),
            _ => (self.width as f32 * 0.75, self.height as f32 * 0.75),
        }
    }
}

pub type Doc = Vec<Block>;

// ---------------------------------------------------------------------------------------------
// Lectores
// ---------------------------------------------------------------------------------------------

/// Texto plano: los párrafos se separan por líneas en blanco y las líneas de dentro se conservan.
pub fn from_plain_text(text: &str) -> Doc {
    let mut doc = Doc::new();
    let mut lines: Vec<&str> = Vec::new();
    let flush = |lines: &mut Vec<&str>, doc: &mut Doc| {
        if !lines.is_empty() {
            doc.push(Block::Para(lines.join("\n")));
            lines.clear();
        }
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut lines, &mut doc);
        } else {
            lines.push(line.trim_end());
        }
    }
    flush(&mut lines, &mut doc);
    doc
}

/// Código, JSON o cualquier texto que debe verse tal cual.
pub fn from_preformatted(text: &str) -> Doc {
    vec![Block::Pre(text.trim_end().to_string())]
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Markdown: títulos, párrafos, listas, bloques de código y tablas. El énfasis y los enlaces se quedan en su texto.
pub fn from_markdown(text: &str) -> Doc {
    let mut doc = Doc::new();
    let mut buf = String::new();
    let mut heading: Option<u8> = None;
    let mut in_code = false;
    let mut items = 0usize; // anidamiento de listas
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_cell = false;

    for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => heading = Some(heading_number(level)),
            Event::End(TagEnd::Heading(_)) => {
                doc.push(Block::Heading(heading.take().unwrap_or(1), buf.trim().to_string()));
                buf.clear();
            }
            Event::Start(Tag::Item) => {
                items += 1;
                buf.clear();
            }
            Event::End(TagEnd::Item) => {
                items = items.saturating_sub(1);
                if !buf.trim().is_empty() {
                    doc.push(Block::Bullet(buf.trim().to_string()));
                }
                buf.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                // En una lista, el texto del párrafo pertenece a la viñeta y se cierra con ella.
                if items == 0 && !in_cell {
                    if !buf.trim().is_empty() {
                        doc.push(Block::Para(buf.trim().to_string()));
                    }
                    buf.clear();
                } else if items > 0 {
                    buf.push(' ');
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code = true;
                buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                doc.push(Block::Pre(buf.trim_end().to_string()));
                buf.clear();
            }
            Event::Start(Tag::Table(_)) => table = Some(Vec::new()),
            Event::End(TagEnd::Table) => {
                if let Some(rows) = table.take() {
                    doc.push(Block::Table(rows));
                }
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => row = Vec::new(),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(rows) = table.as_mut() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            Event::Start(Tag::TableCell) => {
                in_cell = true;
                cell.clear();
            }
            Event::End(TagEnd::TableCell) => {
                in_cell = false;
                row.push(cell.trim().to_string());
            }
            Event::Text(t) | Event::Code(t) => {
                if in_cell {
                    cell.push_str(&t);
                } else {
                    buf.push_str(&t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code {
                    buf.push('\n');
                } else if in_cell {
                    cell.push(' ');
                } else {
                    buf.push(' ');
                }
            }
            _ => {}
        }
    }
    if !buf.trim().is_empty() {
        doc.push(Block::Para(buf.trim().to_string()));
    }
    doc
}

pub(crate) fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity.strip_prefix('#').and_then(|number| {
                let code = match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => number.parse().ok()?,
                };
                char::from_u32(code)
            }),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML: lo visible, con títulos, párrafos, listas y tablas. Los scripts y estilos se descartan.
pub fn from_html(html: &str) -> Doc {
    let mut doc = Doc::new();
    let mut buf = String::new();
    let mut heading: Option<u8> = None;
    let mut in_li = false;
    let mut in_pre = false;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_cell = false;

    fn flush(doc: &mut Doc, buf: &mut String, heading: &mut Option<u8>, in_li: bool, in_pre: bool) {
        let text = if in_pre { buf.trim_end().to_string() } else { buf.split_whitespace().collect::<Vec<_>>().join(" ") };
        if !text.trim().is_empty() {
            doc.push(match (*heading, in_li, in_pre) {
                (Some(level), _, _) => Block::Heading(level, text),
                (None, true, _) => Block::Bullet(text),
                (None, false, true) => Block::Pre(text),
                _ => Block::Para(text),
            });
        }
        buf.clear();
        *heading = None;
    }

    let mut rest = html;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            let text = decode_entities(rest);
            if in_cell { cell.push_str(&text) } else { buf.push_str(&text) }
            break;
        };
        let text = decode_entities(&rest[..open]);
        if in_cell { cell.push_str(&text) } else { buf.push_str(&text) }
        rest = &rest[open..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        match name.as_str() {
            "script" | "style" | "head" | "noscript" if !closing => {
                let end_tag = format!("</{name}");
                let lower = rest.to_ascii_lowercase();
                rest = lower.find(&end_tag).and_then(|at| rest[at..].find('>').map(|gt| &rest[at + gt + 1..])).unwrap_or("");
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
                heading = if closing { None } else { Some(name.as_bytes()[1] - b'0') };
            }
            "li" => {
                flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
                in_li = !closing;
            }
            "pre" => {
                flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
                in_pre = !closing;
            }
            "table" => {
                flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
                if closing {
                    if let Some(rows) = table.take() {
                        doc.push(Block::Table(rows));
                    }
                } else {
                    table = Some(Vec::new());
                }
            }
            "tr" => {
                if closing {
                    if let Some(rows) = table.as_mut() {
                        rows.push(std::mem::take(&mut row));
                    }
                } else {
                    row = Vec::new();
                }
            }
            "td" | "th" => {
                if closing {
                    in_cell = false;
                    row.push(cell.split_whitespace().collect::<Vec<_>>().join(" "));
                } else if table.is_some() {
                    in_cell = true;
                    cell.clear();
                }
            }
            "p" | "div" | "section" | "article" | "br" | "ul" | "ol" | "header" | "footer" | "blockquote" | "tbody" | "thead" => {
                if in_pre && name == "br" {
                    buf.push('\n');
                } else if !in_cell {
                    flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
                }
            }
            _ => {}
        }
    }
    flush(&mut doc, &mut buf, &mut heading, in_li, in_pre);
    doc
}

// ---------------------------------------------------------------------------------------------
// Escritores
// ---------------------------------------------------------------------------------------------

fn push_gap(out: &mut String) {
    if !out.is_empty() && !out.ends_with("\n\n") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
}

/// Texto plano.
pub fn to_text(doc: &Doc) -> String {
    let mut out = String::new();
    for (i, block) in doc.iter().enumerate() {
        match block {
            Block::Heading(_, text) => {
                push_gap(&mut out);
                out.push_str(text);
                out.push_str("\n\n");
            }
            Block::Para(text) => {
                out.push_str(text);
                out.push_str("\n\n");
            }
            Block::Bullet(text) => {
                out.push_str("- ");
                out.push_str(text);
                out.push('\n');
                if !matches!(doc.get(i + 1), Some(Block::Bullet(_))) {
                    out.push('\n');
                }
            }
            Block::Pre(text) => {
                push_gap(&mut out);
                out.push_str(text);
                out.push_str("\n\n");
            }
            Block::Image(_) => out.push_str("[imagen]\n\n"),
            Block::Table(rows) => {
                push_gap(&mut out);
                for row in rows {
                    out.push_str(&row.join("\t"));
                    out.push('\n');
                }
                out.push('\n');
            }
        }
    }
    out.trim_end().to_string() + "\n"
}

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// Markdown.
pub fn to_markdown(doc: &Doc) -> String {
    let mut out = String::new();
    for (i, block) in doc.iter().enumerate() {
        match block {
            Block::Heading(level, text) => {
                push_gap(&mut out);
                out.push_str(&"#".repeat((*level).clamp(1, 6) as usize));
                out.push(' ');
                out.push_str(text);
                out.push_str("\n\n");
            }
            Block::Para(text) => {
                out.push_str(&text.replace('\n', "  \n"));
                out.push_str("\n\n");
            }
            Block::Bullet(text) => {
                out.push_str("- ");
                out.push_str(&text.replace('\n', " "));
                out.push('\n');
                if !matches!(doc.get(i + 1), Some(Block::Bullet(_))) {
                    out.push('\n');
                }
            }
            Block::Pre(text) => {
                push_gap(&mut out);
                out.push_str("```\n");
                out.push_str(text);
                out.push_str("\n```\n\n");
            }
            Block::Image(_) => out.push_str("[imagen]\n\n"),
            Block::Table(rows) => {
                push_gap(&mut out);
                let width = rows.iter().map(Vec::len).max().unwrap_or(0);
                if width == 0 {
                    continue;
                }
                for (r, row) in rows.iter().enumerate() {
                    let cells: Vec<String> = (0..width).map(|c| md_cell(row.get(c).map_or("", String::as_str))).collect();
                    out.push_str(&format!("| {} |\n", cells.join(" | ")));
                    if r == 0 {
                        out.push_str(&format!("|{}\n", " --- |".repeat(width)));
                    }
                }
                out.push('\n');
            }
        }
    }
    out.trim_end().to_string() + "\n"
}

pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Una página HTML sencilla y legible.
pub fn to_html(doc: &Doc, title: &str) -> String {
    let mut body = String::new();
    let mut in_list = false;
    for block in doc {
        if !matches!(block, Block::Bullet(_)) && in_list {
            body.push_str("</ul>\n");
            in_list = false;
        }
        match block {
            Block::Heading(level, text) => body.push_str(&format!("<h{0}>{1}</h{0}>\n", (*level).clamp(1, 6), html_escape(text))),
            Block::Para(text) => body.push_str(&format!("<p>{}</p>\n", html_escape(text).replace('\n', "<br>\n"))),
            Block::Bullet(text) => {
                if !in_list {
                    body.push_str("<ul>\n");
                    in_list = true;
                }
                body.push_str(&format!("<li>{}</li>\n", html_escape(text)));
            }
            Block::Pre(text) => body.push_str(&format!("<pre>{}</pre>\n", html_escape(text))),
            Block::Image(image) => body.push_str(&format!(
                "<p><img src=\"data:image/jpeg;base64,{}\" alt=\"\" style=\"max-width: 100%; height: auto;\"></p>\n",
                base64::engine::general_purpose::STANDARD.encode(&image.jpeg)
            )),
            Block::Table(rows) => {
                body.push_str("<table>\n");
                for (r, row) in rows.iter().enumerate() {
                    let tag = if r == 0 { "th" } else { "td" };
                    let cells: String = row.iter().map(|c| format!("<{tag}>{}</{tag}>", html_escape(c))).collect();
                    body.push_str(&format!("<tr>{cells}</tr>\n"));
                }
                body.push_str("</table>\n");
            }
        }
    }
    if in_list {
        body.push_str("</ul>\n");
    }
    format!(
        "<!doctype html>\n<html lang=\"es\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\nbody {{ font-family: Segoe UI, Arial, sans-serif; max-width: 820px; margin: 2rem auto; padding: 0 1rem; line-height: 1.55; color: #222; }}\npre {{ background: #f4f4f6; padding: 0.8rem; overflow: auto; }}\ntable {{ border-collapse: collapse; }}\nth, td {{ border: 1px solid #ccc; padding: 0.3rem 0.6rem; text-align: left; }}\nth {{ background: #f0f0f4; }}\n</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        html_escape(title),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_texto_plano_agrupa_parrafos_y_conserva_las_lineas() {
        let doc = from_plain_text("Hola\nmundo\n\n\n\nSegundo párrafo  \n");
        assert_eq!(doc, vec![Block::Para("Hola\nmundo".into()), Block::Para("Segundo párrafo".into())]);
        assert!(from_plain_text("  \n\n").is_empty());
    }

    #[test]
    fn el_markdown_se_lee_con_titulos_listas_codigo_y_tablas() {
        let md = "# Título\n\nUn *párrafo* con **énfasis** y [un enlace](http://x).\n\n- uno\n- dos\n\n```\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let doc = from_markdown(md);
        assert_eq!(doc[0], Block::Heading(1, "Título".into()));
        assert_eq!(doc[1], Block::Para("Un párrafo con énfasis y un enlace.".into()));
        assert_eq!(doc[2], Block::Bullet("uno".into()));
        assert_eq!(doc[3], Block::Bullet("dos".into()));
        assert_eq!(doc[4], Block::Pre("let x = 1;".into()));
        assert_eq!(doc[5], Block::Table(vec![vec!["a".into(), "b".into()], vec!["1".into(), "2".into()]]));
    }

    #[test]
    fn el_html_se_lee_sin_scripts_ni_estilos_y_con_entidades() {
        let html = "<html><head><title>x</title><style>p{color:red}</style></head><body><h2>Hola &amp; adiós</h2><script>alert(1)</script><p>Uno <b>dos</b>&nbsp;tres</p><ul><li>a</li><li>b</li></ul><table><tr><th>k</th><th>v</th></tr><tr><td>1</td><td>2&lt;3</td></tr></table><!-- nada --></body></html>";
        let doc = from_html(html);
        assert_eq!(doc[0], Block::Heading(2, "Hola & adiós".into()));
        assert_eq!(doc[1], Block::Para("Uno dos tres".into()));
        assert_eq!(doc[2], Block::Bullet("a".into()));
        assert_eq!(doc[3], Block::Bullet("b".into()));
        assert_eq!(doc[4], Block::Table(vec![vec!["k".into(), "v".into()], vec!["1".into(), "2<3".into()]]));
        assert_eq!(doc.len(), 5, "sin restos del script, del estilo ni del título");
        assert_eq!(decode_entities("&#233; &#x41; &desconocido; & fin"), "é A &desconocido; & fin");
    }

    #[test]
    fn los_escritores_producen_texto_markdown_y_html() {
        let doc = vec![
            Block::Heading(1, "Informe".into()),
            Block::Para("Línea 1\nLínea 2".into()),
            Block::Bullet("uno".into()),
            Block::Bullet("dos".into()),
            Block::Pre("a = 1".into()),
            Block::Table(vec![vec!["x".into(), "y|z".into()], vec!["1".into(), "2".into()]]),
        ];
        let text = to_text(&doc);
        assert!(text.starts_with("Informe\n\nLínea 1\nLínea 2\n\n- uno\n- dos\n\na = 1\n\n"), "{text:?}");
        assert!(text.contains("x\ty|z"));
        let md = to_markdown(&doc);
        assert!(md.starts_with("# Informe\n\nLínea 1  \nLínea 2\n\n- uno\n- dos\n\n```\na = 1\n```\n"), "{md:?}");
        assert!(md.contains("| x | y\\|z |\n| --- | --- |\n| 1 | 2 |"), "{md}");
        let html = to_html(&doc, "A <b>");
        assert!(html.contains("<title>A &lt;b&gt;</title>") && html.contains("<h1>Informe</h1>") && html.contains("<li>dos</li>\n</ul>"));
        assert!(html.contains("<p>Línea 1<br>\nLínea 2</p>") && html.contains("<th>x</th><th>y|z</th>"));
    }

    #[test]
    fn ida_y_vuelta_de_markdown_conserva_la_estructura() {
        let doc = vec![Block::Heading(2, "Sección".into()), Block::Para("Texto".into()), Block::Bullet("a".into()), Block::Pre("x".into())];
        assert_eq!(from_markdown(&to_markdown(&doc)), doc);
    }
}
