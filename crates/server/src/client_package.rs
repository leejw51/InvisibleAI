//! A per-session launcher, without server tokens, mnemonics or user content.
use crate::{Api, ApiError, AppState, PrivateState};
use anyhow::{Context, Result};
use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use causewaybay_shared::ClientPackage;
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};

fn binary() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    // Installed packages keep both binaries together; development uses Cargo's output.
    [
        directory.join("client"),
        directory.join("rust-cli"),
        PathBuf::from("bin/client"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

pub fn info(s: &AppState, owner_wallet: Option<String>) -> ClientPackage {
    let backend_url = s
        .addresses
        .get()
        .into_iter()
        .flatten()
        .find(|a| a.label == "Tailscale IP")
        .map(|a| a.web_url.clone());
    let reason = if owner_wallet.is_none() {
        "Sign in with your wallet to download your client."
    } else if backend_url.is_none() {
        "Connect the backend to Tailscale and restart it on a network listener."
    } else if binary().is_none() {
        "Run make package on the backend to build the capture client."
    } else {
        ""
    };
    ClientPackage {
        available: reason.is_empty(),
        reason: reason.into(),
        platform: format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH),
        backend_url,
        owner_wallet,
        token_required: s.token.is_some(),
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub fn launcher(backend: &str, owner: &str, token_required: bool) -> String {
    include_str!("../../../scripts/client-go.sh")
        .replace("@@BACKEND_URL@@", &quote(backend))
        .replace("@@OWNER_WALLET@@", &quote(owner))
        .replace(
            "@@TOKEN_REQUIRED@@",
            if token_required { "true" } else { "false" },
        )
}

pub fn archive(client: &[u8], script: &str) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);
    zip.start_file("client", options)?;
    zip.write_all(client)?;
    zip.start_file("go.sh", options)?;
    zip.write_all(script.as_bytes())?;
    Ok(zip.finish()?.into_inner())
}

pub async fn download(PrivateState(s): PrivateState, headers: HeaderMap) -> Api<Response> {
    let actor = crate::auth::principal(&s, &headers)?;
    let package = info(&s, actor.address);
    if !package.available {
        return Err(ApiError(StatusCode::CONFLICT, package.reason));
    }
    let binary = binary().context("Client binary is unavailable")?;
    let script = launcher(
        package.backend_url.as_deref().unwrap(),
        package.owner_wallet.as_deref().unwrap(),
        package.token_required,
    );
    let bytes = tokio::task::spawn_blocking(move || archive(&std::fs::read(binary)?, &script))
        .await
        .context("Could not build client package")??;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"client.zip\"",
            ),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_has_only_two_executable_files_and_quotes_parameters() -> Result<()> {
        let script = launcher("http://100.64.0.20:3000", "0x123'$(touch nope)", true);
        assert!(script.contains("owner_wallet='0x123'\"'\"'$(touch nope)'"));
        assert!(script.contains("[ true = true ]"));
        let bytes = archive(b"fixture executable", &script)?;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        assert_eq!(zip.len(), 2);
        for name in ["client", "go.sh"] {
            assert_eq!(zip.by_name(name)?.unix_mode().unwrap() & 0o777, 0o755);
        }
        Ok(())
    }
}
