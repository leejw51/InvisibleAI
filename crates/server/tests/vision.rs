#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "Requires macOS Vision framework access; run make test-vision"]
async fn apple_vision_reads_real_image() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("ocr.png");
    apple_vision::recognize_text::_test_helper_render_text_png(
        "CAUSEWAYBAY RUST 123",
        900,
        180,
        &path,
    )?;
    let text = causewaybay_server::capture::ocr_bytes(std::fs::read(path)?).await?;
    assert!(text.contains("CAUSEWAYBAY"), "OCR result: {text}");
    assert!(text.contains("123"), "OCR result: {text}");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-fixtures/capture.png");
    std::fs::create_dir_all(fixture.parent().unwrap())?;
    apple_vision::recognize_text::_test_helper_render_text_png(
        "What is 7 + 5?",
        900,
        180,
        &fixture,
    )?;
    Ok(())
}
