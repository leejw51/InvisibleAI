//! Binary attachments (screenshots, uploaded media, cached thumbnails) beside
//! the SQLite database. Text is never written here: it lives in SQLite.
use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing workspace directory")?;
    private_dir(parent)?;
    let mut file = tempfile::Builder::new()
        .prefix("write-")
        .tempfile_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("Could not save {}", path.display()))?;
    Ok(())
}
pub fn image_extension(bytes: &[u8]) -> &'static str {
    match crate::capture::image_mime(bytes) {
        Some("image/png") => "png",
        Some("image/jpeg") => "jpg",
        Some("image/webp") => "webp",
        _ => "bin",
    }
}
pub fn memo_path(root: &Path, folder: &str, id: &str, extension: &str) -> Result<PathBuf> {
    ensure!(uuid::Uuid::parse_str(id).is_ok(), "Invalid memo ID");
    Ok(root.join(folder).join(format!("{id}.{extension}")))
}
pub fn file_path(root: &Path, id: &str, name: &str) -> Result<PathBuf> {
    ensure!(uuid::Uuid::parse_str(id).is_ok(), "Invalid file ID");
    let name: String = name
        .chars()
        .take(160)
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = if name.is_empty() || name == "." || name == ".." {
        "file"
    } else {
        &name
    };
    Ok(root.join("files").join(id).join(name))
}

/// Database paths are relative to the data directory, never client-supplied filesystem paths.
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    use std::path::Component;
    ensure!(
        !relative.is_empty()
            && Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "Invalid stored attachment path"
    );
    let path = root.join(relative);
    if path.exists() {
        ensure!(
            path.canonicalize()?.starts_with(root.canonicalize()?),
            "Attachment escapes the persistence folder"
        );
    }
    Ok(path)
}
