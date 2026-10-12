use anyhow::{Result, ensure};
use std::path::Path;

pub async fn ocr_bytes(bytes: Vec<u8>) -> Result<String> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut file = tempfile::Builder::new().suffix(".png").tempfile()?;
        file.write_all(&bytes)?;
        ocr_path(file.path())
    })
    .await?
}

pub fn ocr_path(path: &Path) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        use apple_vision::recognize_text::{RecognitionLevel, TextRecognizer};
        let recognizer = TextRecognizer::new()
            .with_recognition_level(RecognitionLevel::Accurate)
            .with_language_correction(false);
        let observations = recognizer
            .recognize_in_path(path)
            .map_err(|e| anyhow::anyhow!("Apple Vision could not read this image: {e}"))?;
        let text = observations
            .into_iter()
            .map(|o| o.text)
            .collect::<Vec<_>>()
            .join("\n");
        ensure!(
            !text.trim().is_empty(),
            "No readable text was found. Try a clearer capture or a different screen region."
        );
        Ok(text)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        anyhow::bail!("Image text recognition requires macOS and Apple Vision")
    }
}

/// What Apple Vision sees in a picture, as hashtags: its image classifier's
/// confident labels (1000+ generic concepts), one spelling each. Empty when
/// nothing is confident, or off macOS.
pub async fn labels_bytes(bytes: Vec<u8>) -> Result<Vec<String>> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut file = tempfile::Builder::new().suffix(".png").tempfile()?;
        file.write_all(&bytes)?;
        labels_path(file.path())
    })
    .await?
}
pub fn labels_path(path: &Path) -> Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        let found = apple_vision::classify::classify_image_in_path(path)
            .map_err(|e| anyhow::anyhow!("Apple Vision could not classify this image: {e}"))?;
        Ok(label_tags(
            found.into_iter().map(|c| (c.identifier, c.confidence)),
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(vec![])
    }
}
/// Confident labels (0.3 and up, best first, six at most) as normalized tags.
pub fn label_tags(found: impl IntoIterator<Item = (String, f32)>) -> Vec<String> {
    let mut labels: Vec<(String, f32)> = found
        .into_iter()
        .filter(|(_, confidence)| *confidence >= 0.3)
        .collect();
    labels.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out: Vec<String> = vec![];
    for (name, _) in labels {
        if let Some(tag) = crate::wiki::normalize_tag(&name)
            && !out.contains(&tag)
        {
            out.push(tag);
        }
        if out.len() == 6 {
            break;
        }
    }
    out
}

pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn decode_image(bytes: Vec<u8>) -> Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(16000);
    limits.max_image_height = Some(16000);
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// Compare decoded pixels, so metadata/encoding changes do not create new frames.
/// History itself is the durable baseline, including after backend restarts.
pub async fn discard_unchanged(
    database: &crate::db::Database,
    room: &str,
    memo: &str,
    bytes: Vec<u8>,
) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let previous: Option<String> = database.0.lock().unwrap().query_row(
        "SELECT image_path FROM memos WHERE roomid=?1 AND id!=?2 AND owner_wallet=?3 AND image_path IS NOT NULL ORDER BY created_at DESC,rowid DESC LIMIT 1",
        rusqlite::params![room, memo, database.owner()], |r| r.get(0)
    ).optional()?;
    let Some(previous) = previous else {
        return Ok(false);
    };
    let old = std::fs::read(crate::artifacts::resolve(&database.1, &previous)?)?;
    let same = tokio::task::spawn_blocking(move || -> Result<bool> {
        let old = decode_image(old)?.to_rgba8();
        let new = decode_image(bytes)?.to_rgba8();
        Ok(old == new)
    })
    .await??;
    if same {
        let conn = database.0.lock().unwrap();
        let path: Option<String> = conn
            .query_row(
                "SELECT image_path FROM memos WHERE id=?1 AND owner_wallet=?2",
                rusqlite::params![memo, database.owner()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(path) = path {
            std::fs::remove_file(crate::artifacts::resolve(&database.1, &path)?)?;
            conn.execute(
                "UPDATE memos SET image_path=NULL,image=NULL WHERE id=?1",
                [memo],
            )?;
            conn.execute("INSERT INTO notifications(client_id,kind,data) SELECT client_id,'capture.remove',json_object('request_id',id) FROM capture_requests WHERE memo_id=?1", [memo])?;
        }
    }
    Ok(same)
}

/// True when the previous capture in this room produced the same recognized text.
pub fn unchanged_text(
    database: &crate::db::Database,
    room: &str,
    memo: &str,
    text: &str,
) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let previous: Option<String> = database
        .0
        .lock()
        .unwrap()
        .query_row(
            "SELECT e.text FROM events e JOIN memos m ON m.id=e.memo_id
        WHERE m.roomid=?1 AND m.id!=?2 AND m.owner_wallet=?3 AND e.kind='ocr_text'
        ORDER BY e.id DESC LIMIT 1",
            rusqlite::params![room, memo, database.owner()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(previous.is_some_and(|p| p.trim() == text.trim() && !text.trim().is_empty()))
}

/// Browser previews use a 960-pixel, standard RGB JPEG for fast response display.
/// The original capture is never replaced, and no derivative files survive
/// deletion of the original.
pub async fn thumbnail(bytes: Vec<u8>) -> Result<Vec<u8>> {
    static WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let permit = WORKERS.acquire().await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let image = decode_image(bytes)?;
        let image = if image.width() > 960 || image.height() > 960 {
            image.resize(960, 960, image::imageops::FilterType::Triangle)
        } else {
            image
        };
        // Flatten transparent uploads onto white instead of turning them black.
        let rgb = image::RgbImage::from_fn(image.width(), image.height(), |x, y| {
            use image::GenericImageView;
            let pixel = image.get_pixel(x, y);
            let alpha = u16::from(pixel[3]);
            image::Rgb(std::array::from_fn(|i| {
                ((u16::from(pixel[i]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
            }))
        });
        let mut jpeg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 78).encode_image(&rgb)?;
        Ok(jpeg)
    })
    .await?
}

/// Bound decoded allocation and resize a cloud copy. Original bytes stay in the backend persistence folder.
pub async fn cloud_image(bytes: Vec<u8>) -> Result<(String, String)> {
    tokio::task::spawn_blocking(move || {
        use base64::Engine;
        let image = decode_image(bytes)?;
        let image = if image.width() > 2400 || image.height() > 2400 {
            image.resize(2400, 2400, image::imageops::FilterType::Lanczos3)
        } else {
            image
        }
        .to_rgb8();
        let mut jpeg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88).encode_image(&image)?;
        ensure!(
            jpeg.len() < 7 * 1024 * 1024,
            "Image is too large for the selected provider"
        );
        Ok((
            "image/jpeg".into(),
            base64::engine::general_purpose::STANDARD.encode(jpeg),
        ))
    })
    .await?
}
