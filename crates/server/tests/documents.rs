use causewaybay_server::{documents, media};
use std::io::{Cursor, Write};

fn package(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in entries {
        zip.start_file(
            *name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn pdf(text: &str) -> Vec<u8> {
    let text = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    let content = format!("BT /F1 12 Tf 40 760 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()),
    ];
    let mut output = "%PDF-1.4\n".to_owned();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = output.len();
    output.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        output.push_str(&format!("{offset:010} 00000 n \n"));
    }
    output.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    output.into_bytes()
}

#[test]
fn pdf_and_word_extract_actual_text_including_unicode_and_entities() -> anyhow::Result<()> {
    let pdf = pdf("PDF reference ORCHID-48");
    assert_eq!(media::mime("no-extension", &pdf), documents::PDF);
    let (_, text, truncated) = media::text_or_raw("brief.pdf", documents::PDF, &pdf)?;
    assert!(text.contains("PDF reference ORCHID-48"));
    assert!(!truncated);
    let word = package(&[(
        "word/document.xml",
        "<w:document xmlns:w=\"urn:word\"><w:body><w:p><w:r><w:t>회의 &amp; Notes</w:t></w:r></w:p><w:p><w:r><w:t>Ready &#49;2</w:t></w:r></w:p></w:body></w:document>",
    )]);
    let (_, text, _) = media::text_or_raw("brief.docx", "application/octet-stream", &word)?;
    assert!(text.contains("회의 & Notes\n"));
    assert!(text.contains("Ready 12"));
    Ok(())
}

#[test]
fn spreadsheets_resolve_shared_strings_inline_text_and_cached_values() -> anyhow::Result<()> {
    let bytes = package(&[
        (
            "xl/workbook.xml",
            "<workbook xmlns:r=\"urn:rels\"><sheets><sheet name=\"Sales\" sheetId=\"1\" r:id=\"rId7\"/></sheets></workbook>",
        ),
        (
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"rId7\" Target=\"worksheets/sheet3.xml\"/></Relationships>",
        ),
        (
            "xl/sharedStrings.xml",
            "<sst><si><r><t>Blue </t></r><r><t>&amp; Green</t></r></si></sst>",
        ),
        (
            "xl/worksheets/sheet3.xml",
            "<worksheet><sheetData><row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\"><f>20+22</f><v>42</v></c><c r=\"C1\" t=\"inlineStr\"><is><t>Cost &#x20AC;</t></is></c></row></sheetData></worksheet>",
        ),
    ]);
    let (_, text, _) = media::text_or_raw("sales.xlsx", documents::XLSX, &bytes)?;
    assert!(text.contains("Sheet: Sales"));
    assert!(text.contains("A1: \"Blue & Green\""));
    assert!(text.contains("B1: \"42\""));
    assert!(text.contains("C1: \"Cost €\""));
    assert!(!text.contains("20+22"));
    Ok(())
}

#[test]
fn presentation_manifest_controls_slide_order() -> anyhow::Result<()> {
    let bytes = package(&[
        (
            "ppt/presentation.xml",
            "<p:presentation xmlns:p=\"urn:ppt\" xmlns:r=\"urn:rels\"><p:sldIdLst><p:sldId id=\"257\" r:id=\"rId2\"/><p:sldId id=\"256\" r:id=\"rId1\"/></p:sldIdLst></p:presentation>",
        ),
        (
            "ppt/_rels/presentation.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Target=\"slides/slide1.xml\"/><Relationship Id=\"rId2\" Target=\"slides/slide2.xml\"/></Relationships>",
        ),
        ("ppt/slides/slide1.xml", "<p><t>SECOND</t></p>"),
        ("ppt/slides/slide2.xml", "<p><t>FIRST</t></p>"),
    ]);
    let (_, text, _) = media::text_or_raw("slides.pptx", documents::PPTX, &bytes)?;
    assert!(text.find("FIRST").unwrap() < text.find("SECOND").unwrap());
    Ok(())
}

#[test]
fn open_documents_and_archives_keep_data_local() -> anyhow::Result<()> {
    let bytes = package(&[(
        "content.xml",
        "<document><p>Open document reference</p><table-row><table-cell><p>42</p></table-cell></table-row></document>",
    )]);
    for extension in ["odt", "ods", "odp"] {
        let name = format!("notes.{extension}");
        let (_, text, _) = media::text_or_raw(&name, media::mime(&name, &bytes), &bytes)?;
        assert!(text.contains("Open document reference"));
        assert!(text.contains("42"));
    }
    let bytes = package(&[
        ("folder/hello.txt", "Nested contents stay unread"),
        ("../outside.txt", "not extracted"),
    ]);
    let (kind, text, _) = media::text_or_raw("archive.zip", documents::ZIP, &bytes)?;
    assert_eq!(kind, "archive");
    assert!(text.contains("folder/hello.txt") && text.contains("../outside.txt"));
    assert!(!text.contains("Nested contents stay unread"));
    Ok(())
}

#[tokio::test]
async fn malformed_oversized_and_external_documents_are_rejected() {
    for (name, bytes) in [
        ("broken.pdf", b"%PDF-1.4\nbroken".to_vec()),
        ("broken.docx", b"not a zip".to_vec()),
        (
            "entities.docx",
            package(&[(
                "word/document.xml",
                "<!DOCTYPE doc [<!ENTITY private SYSTEM 'file:///etc/passwd'>]><doc>&private;</doc>",
            )]),
        ),
        (
            "large.docx",
            package(&[("word/document.xml", &"x".repeat(8 * 1024 * 1024 + 1))]),
        ),
        (
            "external.xlsx",
            package(&[
                (
                    "xl/workbook.xml",
                    "<workbook xmlns:r=\"urn:rels\"><sheet name=\"Secret\" r:id=\"rId1\"/></workbook>",
                ),
                (
                    "xl/_rels/workbook.xml.rels",
                    "<Relationships><Relationship Id=\"rId1\" TargetMode=\"External\" Target=\"https://example.test/private\"/></Relationships>",
                ),
            ]),
        ),
    ] {
        let mime = media::mime(name, &bytes);
        assert!(
            media::context_text(name, mime, bytes).await.is_err(),
            "{name}"
        );
    }
}

#[test]
fn long_unicode_documents_and_context_excerpts_are_bounded() -> anyhow::Result<()> {
    let body = format!("<document><p>{}</p></document>", "한글".repeat(30_000));
    let bytes = package(&[("word/document.xml", &body)]);
    let (_, text, truncated) = media::text_or_raw("long.docx", documents::DOCX, &bytes)?;
    assert!(truncated);
    assert!(text.len() <= 100_000);
    assert!(text.ends_with(['한', '글']));
    assert!(media::excerpt("한글 abc", 4).starts_with("한\n[Context excerpt truncated"));
    Ok(())
}

#[test]
fn uploads_allow_dotfiles_unicode_and_extensionless_names_but_never_paths() {
    for name in [".gitignore", ".env.example", "회의 자료.docx", "Makefile"] {
        assert_eq!(media::upload_name(name).unwrap(), name);
    }
    for name in [
        "",
        ".",
        "..",
        "../secret",
        "/absolute",
        "C:\\file",
        "bad\nname",
        "bad\0name",
    ] {
        assert!(media::upload_name(name).is_err(), "{name:?}");
    }
}
