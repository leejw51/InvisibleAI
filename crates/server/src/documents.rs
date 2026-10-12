//! Bounded, local document text extraction. Never executes macros or extracts ZIPs to disk.
use anyhow::{Context, Result, bail, ensure};
use quick_xml::{Reader, events::Event};
use std::{
    collections::HashMap,
    io::{Cursor, Read, Write},
};

const TEXT_LIMIT: usize = 100_000;
const PART_LIMIT: u64 = 8 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 32 * 1024 * 1024;
pub const PDF: &str = "application/pdf";
pub const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
pub const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
pub const ODT: &str = "application/vnd.oasis.opendocument.text";
pub const ODS: &str = "application/vnd.oasis.opendocument.spreadsheet";
pub const ODP: &str = "application/vnd.oasis.opendocument.presentation";
pub const ZIP: &str = "application/zip";

pub fn mime(name: &str, bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"%PDF-") {
        return Some(PDF);
    }
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Some(PDF),
        "docx" => Some(DOCX),
        "pptx" => Some(PPTX),
        "xlsx" => Some(XLSX),
        "odt" => Some(ODT),
        "ods" => Some(ODS),
        "odp" => Some(ODP),
        "zip" => Some(ZIP),
        _ if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") => Some(ZIP),
        _ => None,
    }
}

#[derive(Default)]
struct Text {
    value: String,
    truncated: bool,
}
impl Text {
    fn push(&mut self, text: &str) {
        let mut remaining = TEXT_LIMIT.saturating_sub(self.value.len()).min(text.len());
        while !text.is_char_boundary(remaining) {
            remaining -= 1;
        }
        self.value.push_str(&text[..remaining]);
        self.truncated |= remaining < text.len();
    }
}
impl Write for Text {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.push(std::str::from_utf8(bytes).map_err(std::io::Error::other)?);
        if self.truncated {
            return Err(std::io::Error::other("Document text limit reached"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Package<'a> {
    zip: zip::ZipArchive<Cursor<&'a [u8]>>,
    read: u64,
}
impl<'a> Package<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let zip = zip::ZipArchive::new(Cursor::new(bytes)).context("Invalid document archive")?;
        ensure!(
            zip.len() <= 4096,
            "Document archive contains too many entries"
        );
        Ok(Self { zip, read: 0 })
    }
    fn part(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut file = self
            .zip
            .by_name(name)
            .with_context(|| format!("Missing document part: {name}"))?;
        ensure!(
            file.size() <= PART_LIMIT,
            "Document part exceeds the 8 MB extraction limit"
        );
        let mut bytes = Vec::new();
        (&mut file).take(PART_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= PART_LIMIT,
            "Document part exceeds the extraction limit"
        );
        self.read += bytes.len() as u64;
        ensure!(
            self.read <= TOTAL_LIMIT,
            "Document exceeds the 32 MB extraction limit"
        );
        Ok(bytes)
    }
}

fn entity(reference: &quick_xml::events::BytesRef<'_>) -> Result<String> {
    if let Some(c) = reference.resolve_char_ref()? {
        return Ok(c.to_string());
    }
    Ok(match reference.decode()?.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => bail!("Unsupported XML entity"),
    }
    .into())
}

fn xml_text(bytes: &[u8], out: &mut Text) -> Result<()> {
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Text(v) => out.push(&v.xml_content()?),
            Event::CData(v) => out.push(&v.decode()?),
            Event::GeneralRef(v) => out.push(&entity(&v)?),
            Event::End(v) => match v.local_name().as_ref() {
                b"p" | b"h" | b"tr" | b"table-row" => out.push("\n"),
                b"tc" | b"table-cell" => out.push("\t"),
                _ => {}
            },
            Event::Empty(v) => match v.local_name().as_ref() {
                b"tab" => out.push("\t"),
                b"br" | b"line-break" => out.push("\n"),
                b"s" => out.push(" "),
                _ => {}
            },
            Event::DocType(_) => bail!("Document XML cannot contain a DTD"),
            Event::Eof => break,
            _ => {}
        }
        if out.truncated {
            break;
        }
        buffer.clear();
    }
    Ok(())
}

fn attributes(
    v: &quick_xml::events::BytesStart<'_>,
    reader: &Reader<&[u8]>,
) -> Result<HashMap<String, String>> {
    v.attributes()
        .map(|a| {
            let a = a?;
            Ok((
                std::str::from_utf8(a.key.local_name().as_ref())?.to_owned(),
                a.decode_and_unescape_value(reader.decoder())?.into_owned(),
            ))
        })
        .collect()
}

/// Preserve workbook/slide order from the manifest, never from numeric file names.
fn ordered_parts(
    package: &mut Package<'_>,
    root: &str,
    manifest: &str,
    item: &[u8],
) -> Result<Vec<(String, String)>> {
    let rels = package.part(&format!("{root}/_rels/{manifest}.rels"))?;
    let mut reader = Reader::from_reader(rels.as_slice());
    let mut links = HashMap::new();
    loop {
        match reader.read_event()? {
            Event::Empty(v) | Event::Start(v) if v.local_name().as_ref() == b"Relationship" => {
                let a = attributes(&v, &reader)?;
                if a.get("TargetMode").is_some_and(|v| v == "External") {
                    continue;
                }
                if let (Some(id), Some(target)) = (a.get("Id"), a.get("Target")) {
                    // Relationships only identify local package entries; URLs and traversal are never fetched.
                    ensure!(
                        !target.contains([':', '\\']) && !target.split('/').any(|p| p == ".."),
                        "Invalid document relationship"
                    );
                    let target = if target.starts_with('/') {
                        target.trim_start_matches('/').to_owned()
                    } else {
                        format!("{root}/{}", target.trim_start_matches("./"))
                    };
                    links.insert(id.clone(), target);
                }
            }
            Event::DocType(_) => bail!("Document XML cannot contain a DTD"),
            Event::Eof => break,
            _ => {}
        }
    }
    let bytes = package.part(&format!("{root}/{manifest}"))?;
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut parts = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Empty(v) | Event::Start(v) if v.local_name().as_ref() == item => {
                // p:sldId has both id and r:id. Only the namespaced relationship ID is a link.
                let mut relation = None;
                let mut name = None;
                for a in v.attributes() {
                    let a = a?;
                    let value = a.decode_and_unescape_value(reader.decoder())?.into_owned();
                    if a.key.as_ref().contains(&b':') && a.key.local_name().as_ref() == b"id" {
                        relation = Some(value);
                    } else if a.key.as_ref() == b"name" {
                        name = Some(value);
                    }
                }
                let path = relation
                    .and_then(|r| links.get(&r).cloned())
                    .context("Missing document relationship")?;
                parts.push((
                    name.unwrap_or_else(|| format!("Slide {}", parts.len() + 1)),
                    path,
                ));
                ensure!(
                    parts.len() <= 200,
                    "Document has more than 200 sheets or slides"
                );
            }
            Event::DocType(_) => bail!("Document XML cannot contain a DTD"),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(parts)
}

fn shared_strings(bytes: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(bytes);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            Event::Start(v) if v.local_name().as_ref() == b"si" => current.clear(),
            Event::Start(v) if v.local_name().as_ref() == b"t" => in_text = true,
            Event::Text(v) if in_text => current.push_str(&v.xml_content()?),
            Event::GeneralRef(v) if in_text => current.push_str(&entity(&v)?),
            Event::End(v) if v.local_name().as_ref() == b"t" => in_text = false,
            Event::End(v) if v.local_name().as_ref() == b"si" => strings.push(current.clone()),
            Event::DocType(_) => bail!("Document XML cannot contain a DTD"),
            Event::Eof => break,
            _ => {}
        }
        ensure!(
            strings.len() <= 100_000,
            "Spreadsheet has too many shared strings"
        );
    }
    Ok(strings)
}

fn sheet_text(bytes: &[u8], strings: &[String], out: &mut Text) -> Result<()> {
    let mut reader = Reader::from_reader(bytes);
    let (mut reference, mut kind, mut value) = (String::new(), String::new(), String::new());
    let mut in_value = false;
    loop {
        match reader.read_event()? {
            Event::Start(v) if v.local_name().as_ref() == b"c" => {
                let attrs = attributes(&v, &reader)?;
                reference = attrs.get("r").cloned().unwrap_or_default();
                kind = attrs.get("t").cloned().unwrap_or_default();
                value.clear();
            }
            Event::Start(v) if matches!(v.local_name().as_ref(), b"v" | b"t") => in_value = true,
            Event::Text(v) if in_value => value.push_str(&v.xml_content()?),
            Event::GeneralRef(v) if in_value => value.push_str(&entity(&v)?),
            Event::End(v) if matches!(v.local_name().as_ref(), b"v" | b"t") => in_value = false,
            Event::End(v) if v.local_name().as_ref() == b"c" => {
                if !value.is_empty() {
                    let decoded = if kind == "s" {
                        strings
                            .get(value.trim().parse::<usize>()?)
                            .context("Invalid shared-string reference")?
                            .as_str()
                    } else {
                        &value
                    };
                    out.push(&format!(
                        "{reference}: {}\t",
                        serde_json::to_string(decoded)?
                    ));
                }
            }
            Event::End(v) if v.local_name().as_ref() == b"row" => out.push("\n"),
            Event::DocType(_) => bail!("Document XML cannot contain a DTD"),
            Event::Eof => break,
            _ => {}
        }
        if out.truncated {
            break;
        }
    }
    Ok(())
}

pub fn extract(mime: &str, bytes: &[u8]) -> Result<Option<(String, String, bool)>> {
    if !matches!(mime, PDF | DOCX | PPTX | XLSX | ODT | ODS | ODP | ZIP) {
        return Ok(None);
    }
    let mut out = Text::default();
    if mime == PDF {
        let document = pdf_extract::Document::load_mem(bytes)
            .context("Cannot read PDF; it may be encrypted or damaged")?;
        ensure!(
            !document.is_encrypted(),
            "Password-protected PDFs need an unlocked copy"
        );
        let pages = document.get_pages();
        let truncated_pages = pages.len() > 200;
        for page in pages.keys().take(200) {
            let result = {
                let writer: &mut dyn Write = &mut out;
                pdf_extract::output_doc_page(
                    &document,
                    &mut pdf_extract::PlainTextOutput::new(writer),
                    *page,
                )
            };
            if out.truncated {
                break;
            }
            result.context("Cannot extract PDF text")?;
            out.push("\n");
        }
        out.truncated |= truncated_pages;
        ensure!(
            !out.value.trim().is_empty(),
            "PDF has no readable text; upload a text-enabled PDF or images of its scanned pages"
        );
    } else {
        let mut package = Package::new(bytes)?;
        match mime {
            DOCX => {
                xml_text(&package.part("word/document.xml")?, &mut out)?;
                for part in ["word/footnotes.xml", "word/endnotes.xml"] {
                    if package.zip.file_names().any(|n| n == part) {
                        out.push("\n");
                        xml_text(&package.part(part)?, &mut out)?;
                    }
                }
            }
            ODT | ODS | ODP => xml_text(&package.part("content.xml")?, &mut out)?,
            PPTX => {
                for (title, part) in
                    ordered_parts(&mut package, "ppt", "presentation.xml", b"sldId")?
                {
                    out.push(&format!("\n{title}\n"));
                    xml_text(&package.part(&part)?, &mut out)?;
                    if out.truncated {
                        break;
                    }
                }
            }
            XLSX => {
                let strings = if package
                    .zip
                    .file_names()
                    .any(|n| n == "xl/sharedStrings.xml")
                {
                    shared_strings(&package.part("xl/sharedStrings.xml")?)?
                } else {
                    vec![]
                };
                out.push("Spreadsheet cell values; formulas use saved results and dates may be serial numbers.\n");
                for (title, part) in ordered_parts(&mut package, "xl", "workbook.xml", b"sheet")? {
                    out.push(&format!("\nSheet: {title}\n"));
                    sheet_text(&package.part(&part)?, &strings, &mut out)?;
                    if out.truncated {
                        break;
                    }
                }
            }
            ZIP => {
                out.push(
                    "Archive contents listing only; nested file contents have not been read.\n",
                );
                for index in 0..package.zip.len() {
                    let file = package.zip.by_index(index)?;
                    out.push(&format!("{} — {} bytes\n", file.name(), file.size()));
                    if out.truncated {
                        break;
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(Some((
        if mime == ZIP { "archive" } else { "document" }.into(),
        out.value,
        out.truncated,
    )))
}
