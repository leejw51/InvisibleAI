//! Validate displays and request macOS consent only after a real capture fails.
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
}

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn has_access() -> bool {
    // No pixels are read and no dialog is opened by this preflight.
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn prepare(mode: &str, display: u32) -> Result<()> {
    ensure!(matches!(mode, "screen" | "region"), "Unknown capture mode");
    let mut count = 0;
    let status = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    ensure!(
        status == 0,
        "Could not inspect the capture Mac's displays. Sign in and unlock that Mac, then restart its capture CLI."
    );
    validate_display(mode, display, count)?;
    Ok(())
}

pub fn complete(succeeded: bool) -> Result<()> {
    capture_result(succeeded, &REQUESTED, has_access, || unsafe {
        CGRequestScreenCaptureAccess()
    })
}

fn capture_result(
    succeeded: bool,
    requested: &AtomicBool,
    preflight: impl FnOnce() -> bool,
    request: impl FnOnce() -> bool,
) -> Result<()> {
    // The CLI's preflight is advisory: /usr/sbin/screencapture is the process
    // that reads pixels. Do not reject its successful result or prompt for an
    // additional permission just because the CLI's own check returns false.
    if succeeded {
        return Ok(());
    }
    if !preflight() {
        // Prompt on the capture Mac, once per client run, never on startup or
        // repeatedly while polling. macOS may require Settings after a denial.
        if !requested.swap(true, Ordering::SeqCst) && request() {
            anyhow::bail!(
                "Screen Recording permission was granted. Press Capture again. If macOS asks you to restart the capture app, restart it first."
            );
        }
        anyhow::bail!(
            "Screen Recording access was requested on the capture Mac. Allow it in the macOS permission dialog. If no dialog appears, enable the app that started the capture client in System Settings → Privacy & Security → Screen & System Audio Recording. Then restart that app and the capture client, and press Capture again. TextEdit Automation permission is separate."
        );
    }
    anyhow::bail!(
        "Capture cancelled or unavailable on the capture Mac. Unlock the Mac and restart its capture client from the signed-in desktop. For region capture, finish the selection or press Capture to try again."
    )
}

fn validate_display(mode: &str, display: u32, count: u32) -> Result<()> {
    ensure!(
        count > 0,
        "No active display is available on the capture Mac. Sign in, unlock it and connect a display before capturing."
    );
    ensure!(
        mode == "region" || (1..=count).contains(&display),
        "Display {display} is unavailable on the capture Mac ({count} active). Choose a display number from 1 to {count} in backend settings."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_capture_never_depends_on_cli_permission_or_prompts() {
        let requested = AtomicBool::new(false);
        capture_result(
            true,
            &requested,
            || panic!("preflight must not gate capture"),
            || panic!("no permission request needed"),
        )
        .unwrap();
        assert!(!requested.load(Ordering::SeqCst));
    }

    #[test]
    fn failed_capture_requests_permission_once_and_explains_retry() {
        let requested = AtomicBool::new(false);
        let prompts = std::cell::Cell::new(0);
        let error = capture_result(
            false,
            &requested,
            || false,
            || {
                prompts.set(prompts.get() + 1);
                false
            },
        )
        .unwrap_err()
        .to_string();
        assert!(requested.load(Ordering::SeqCst));
        assert_eq!(prompts.get(), 1);
        assert!(error.contains("macOS permission dialog"));
        assert!(error.contains("System Settings"));
        capture_result(
            false,
            &requested,
            || false,
            || panic!("do not repeat permission dialogs"),
        )
        .unwrap_err();
        let granted = AtomicBool::new(false);
        assert!(
            capture_result(false, &granted, || false, || true)
                .unwrap_err()
                .to_string()
                .contains("permission was granted")
        );
        assert!(
            capture_result(false, &requested, || true, || panic!("already allowed"))
                .unwrap_err()
                .to_string()
                .contains("Capture cancelled")
        );
    }

    #[test]
    fn missing_display_is_not_reported_as_missing_permission() {
        let error = validate_display("screen", 16, 1).unwrap_err().to_string();
        assert!(error.contains("Display 16 is unavailable"));
        assert!(!error.contains("permission"));
        assert!(validate_display("screen", 0, 1).is_err());
        assert!(validate_display("screen", 1, 1).is_ok());
        assert!(validate_display("screen", 2, 2).is_ok());
    }

    #[test]
    fn region_selection_does_not_use_saved_display_but_requires_a_desktop() {
        assert!(validate_display("region", 16, 1).is_ok());
        for mode in ["screen", "region"] {
            let error = validate_display(mode, 1, 0).unwrap_err().to_string();
            assert!(error.contains("No active display"));
        }
    }
}
