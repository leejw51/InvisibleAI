//! Driving the resumable video upload protocol (`PROTOCOL.md`), ported from
//! PocketSkynet's client.
//!
//! # The file is never in memory
//!
//! A `web_sys::File` is a handle, not bytes: the browser keeps the data on
//! disk. `Blob::slice` makes another handle to part of it without reading
//! anything, and only `array_buffer()` on a slice pulls bytes in. So the
//! checksum pass holds one slice at a time, and the upload pass holds nothing:
//! each chunk's body is the `Blob` slice itself, which the browser's network
//! process reads directly.
//!
//! # The chunk body must be a `Blob`, not an `ArrayBuffer`
//!
//! A measured 4x. A `fetch` whose body is an `ArrayBuffer` makes the renderer
//! copy the buffer to the network process on the main thread (~250 ms per
//! 8 MB chunk, UI frozen, uploads capped near 30 MB/s). A `Blob` body is a
//! handle the network process reads by itself, measured at over 100 MB/s with
//! the main thread idle.
//!
//! # Two passes, on purpose
//!
//! The SHA-256 is computed before uploading because the backend wants it
//! declared up front, which lets `finish` catch a mismatch against something
//! the browser committed to before it knew what would arrive. Reading a local
//! file twice is cheap next to sending it once. Progress is reported for both
//! passes, so a minute of hashing does not read as a hang.
use causewaybay_shared::FileInfo;
use futures_util::future::{Either, select};
use gloo_net::http::{Request, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use yew::Callback;

/// Which pass a progress report belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Reading the file to compute its checksum. Local, no network.
    Checksum,
    /// Sending it.
    Upload,
}

/// What the caller is told after every slice of either pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub phase: Phase,
    pub done: f64,
    pub total: f64,
}
impl Progress {
    /// 0.0–1.0. A zero-length file is complete rather than a division by zero.
    pub fn fraction(&self) -> f64 {
        if self.total <= 0.0 {
            1.0
        } else {
            (self.done / self.total).clamp(0.0, 1.0)
        }
    }
}

/// Where an uploaded video belongs; every field is optional.
#[derive(Clone, Default)]
pub struct VideoTarget {
    pub room: Option<String>,
    pub clear_epoch: Option<i64>,
    pub data_epoch: Option<i64>,
    /// Also keep it as a Wiki video (the Videos page).
    pub wiki: bool,
}

/// The ceiling, mirrored from the backend's `uploads::MAX_VIDEO`. Checked on
/// the device so a file that was never going to work fails before hashing.
pub const MAX_VIDEO_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

/// Fallback when the backend does not name a chunk size (it always does).
/// Matches the backend's `SUGGESTED_CHUNK_BYTES`.
const FALLBACK_CHUNK: f64 = 500.0 * 1024.0;

/// How many times the backend's position is re-read after a failed chunk
/// before the upload gives up.
const CHUNK_ATTEMPTS: u32 = 4;

/// How long `begin` and status requests may run before they count as failed.
/// A dead mobile connection often stops answering rather than erroring, and a
/// `fetch` in that state never settles; without a bound the retry loop never
/// engages and the transfer sits at one byte count forever.
const REQUEST_TIMEOUT_MS: u32 = 45_000;

/// The floor transfer rate an append is budgeted against, in bytes per
/// millisecond (128 KB/s): far below any usable link, because it only
/// separates "slow" from "never". Every timed-out chunk halves the next one,
/// so a link genuinely under it still converges.
const FLOOR_BYTES_PER_MS: f64 = 131.0;

/// Where halving stops. Must stay below the chunk size the backend suggests,
/// or the `.max()` floor would raise a chunk that timed out instead of
/// shrinking it.
const MIN_CHUNK: f64 = 128.0 * 1024.0;
const _: () = assert!(MIN_CHUNK < FALLBACK_CHUNK);

/// Where a transfer starts before the link has proven anything (TCP slow
/// start, a layer up): each landed chunk doubles the next, up to the backend's
/// suggestion. Inert while the suggestion is 500 KB, which is below it; kept so
/// a backend that raises the suggestion gets the slow start back.
const INITIAL_CHUNK: f64 = 2.0 * 1024.0 * 1024.0;

/// The message every timed-out request carries, and the marker that tells
/// "the link is slow" (shrink the next chunk) from an ordinary refusal.
const TIMED_OUT: &str = "The request timed out";

/// While an append is in flight, how often the backend is asked where it is.
/// For a chunk that arrived in full whose answer never reached the phone: the
/// moment the backend's offset shows it landed, the hung request is abandoned
/// and the loop moves on. Safe because the backend advances the offset only
/// once the whole chunk body has arrived.
const PROBE_INTERVAL_MS: u32 = 15_000;

/// The floor rate the backend's re-hash at `finish` is budgeted at
/// (10 MB/s): separates "still hashing" from "never answering".
const HASH_FLOOR_BYTES_PER_MS: f64 = 10_485.0;

/// How many times `finish` is attempted before the upload reports failure.
const FINISH_ATTEMPTS: u32 = 3;

/// Read size for the checksum pass. Larger than a network chunk: this pass
/// never leaves the device, so the only cost is one allocation at a time.
const CHECKSUM_READ: f64 = 16.0 * 1024.0 * 1024.0;

fn append_timeout_ms(bytes: f64) -> u32 {
    REQUEST_TIMEOUT_MS.saturating_add((bytes / FLOOR_BYTES_PER_MS) as u32)
}
fn finish_timeout_ms(size: f64) -> u32 {
    REQUEST_TIMEOUT_MS.saturating_add((size / HASH_FLOOR_BYTES_PER_MS) as u32)
}

/// Whether a picked file goes through the chunked video upload.
pub fn is_video(file: &web_sys::File) -> bool {
    let name = file.name().to_ascii_lowercase();
    file.type_().starts_with("video/")
        || [".mp4", ".mov", ".m4v", ".webm"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

/// A failed request: no answer at all, or an HTTP status with the backend's
/// message.
#[derive(Debug)]
enum Failure {
    Network(String),
    Status(u16, String),
}
impl Failure {
    fn message(self) -> String {
        match self {
            Failure::Network(m) => m,
            Failure::Status(401, _) => "AUTH_REQUIRED".into(),
            Failure::Status(_, m) => m,
        }
    }
}
type Outcome<T> = Result<T, Failure>;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionView {
    id: String,
    offset: f64,
    size: f64,
    chunk_size: f64,
}

/// Upload a video in chunks, reporting progress, and return the stored file.
pub async fn upload_video(
    blob: web_sys::Blob,
    name: String,
    target: VideoTarget,
    progress: Option<Callback<Progress>>,
) -> Result<FileInfo, String> {
    let report = |p: Progress| {
        if let Some(progress) = &progress {
            progress.emit(p);
        }
    };
    let size = blob.size();
    if size <= 0.0 {
        return Err("This video is empty".into());
    }
    if size > MAX_VIDEO_BYTES {
        return Err("Videos must be 1 GB or smaller".into());
    }
    // Pass one: the checksum, over slices.
    let digest = checksum(&blob, size, &report)
        .await
        .map_err(Failure::message)?;
    // Pass two: the transfer, resuming the last attempt at this exact file if
    // one is still open on the backend.
    let key = resume_key(&target, &digest, size);
    let session = match resume(&key, size).await {
        Some(existing) => existing,
        None => {
            let started = begin(&name, size, &digest, &target)
                .await
                .map_err(Failure::message)?;
            remember_session(&key, &started.id);
            started
        }
    };
    let ceiling = if session.chunk_size > 0.0 {
        session.chunk_size
    } else {
        FALLBACK_CHUNK
    };
    let mut chunk = ceiling.min(INITIAL_CHUNK);
    let mut offset = session.offset;
    report(Progress {
        phase: Phase::Upload,
        done: offset,
        total: size,
    });
    while offset < size {
        let end = (offset + chunk).min(size);
        // A handle, not bytes: see the module docs.
        let part = blob
            .slice_with_f64_and_f64(offset, end)
            .map_err(|_| "Could not read this video".to_string())?;
        let controller = abort_controller().map_err(Failure::message)?;
        let append = append(&session.id, offset, &part, &controller);
        let probe = async {
            loop {
                gloo_timers::future::TimeoutFuture::new(PROBE_INTERVAL_MS).await;
                // A failed probe proves nothing about the append; the append's
                // own clock covers the case where everything is dead.
                if let Ok(s) = upload_status(&session.id).await
                    && s.offset >= end
                {
                    return s.offset;
                }
            }
        };
        match select(Box::pin(append), Box::pin(probe)).await {
            Either::Left((Ok(next), _)) => {
                offset = next;
                chunk = (chunk * 2.0).min(ceiling);
            }
            Either::Left((Err(e), _)) => {
                // A timeout means the link could not move this much inside its
                // budget, so the next attempt asks less of it.
                if matches!(&e, Failure::Network(m) if m == TIMED_OUT) {
                    chunk = (chunk / 2.0).max(MIN_CHUNK).min(ceiling);
                }
                // Ask the backend where it really is and carry on from there.
                offset = recover(&session.id, e).await.map_err(Failure::message)?;
            }
            Either::Right((landed, _)) => {
                // The chunk is on the backend; only this answer went missing.
                controller.abort();
                offset = landed;
            }
        }
        report(Progress {
            phase: Phase::Upload,
            done: offset,
            total: size,
        });
    }
    let done = finish(&session.id, size, &name).await;
    // Only a transfer that never reached `finish` is worth resuming.
    if done.is_ok() {
        forget_session(&key);
    }
    done.map_err(Failure::message)
}

/// Pick up an upload of this exact file that an earlier attempt left open, or
/// `None` for anything unclear (no stored id, a forgotten session, a size
/// mismatch) rather than appending this file to the wrong session.
async fn resume(key: &str, size: f64) -> Option<SessionView> {
    let id = stored_session(key)?;
    match upload_status(&id).await {
        Ok(s) if s.size == size => Some(s),
        _ => {
            forget_session(key);
            None
        }
    }
}

async fn begin(name: &str, size: f64, digest: &str, target: &VideoTarget) -> Outcome<SessionView> {
    // A file size goes on the wire as an integer: an `f64` serialises as
    // `125829120.0`, which the backend's `u64` refuses.
    let body = json!({"name":name,"size":size as u64,"sha256":digest,"roomId":target.room,"clearEpoch":target.clear_epoch,"dataEpoch":target.data_epoch,"wiki":target.wiki});
    let controller = abort_controller()?;
    let request = Request::post("/api/uploads")
        .abort_signal(Some(&controller.signal()))
        .json(&body)
        .map_err(|e| Failure::Network(e.to_string()))?;
    decode(send_with_timeout(request, controller, REQUEST_TIMEOUT_MS).await?).await
}

/// Send one chunk as a `Blob` slice and return the backend's new offset.
async fn append(
    id: &str,
    offset: f64,
    part: &web_sys::Blob,
    controller: &web_sys::AbortController,
) -> Outcome<f64> {
    let request = Request::patch(&format!("/api/uploads/{id}?offset={}", offset as u64))
        .header("content-type", "application/octet-stream")
        .abort_signal(Some(&controller.signal()))
        .body(part.clone())
        .map_err(|e| Failure::Network(e.to_string()))?;
    let response =
        send_with_timeout(request, controller.clone(), append_timeout_ms(part.size())).await?;
    let ack: Value = decode(response).await?;
    ack["offset"]
        .as_f64()
        .ok_or_else(|| Failure::Network("Unexpected response from the backend".into()))
}

/// Where to resume after a failed chunk, or the original error when the
/// session is gone or the backend stays unreachable.
async fn recover(id: &str, err: Failure) -> Outcome<f64> {
    for _ in 0..CHUNK_ATTEMPTS {
        match upload_status(id).await {
            Ok(s) => return Ok(s.offset),
            Err(Failure::Status(404, _)) => break,
            Err(_) => continue,
        }
    }
    Err(err)
}

async fn upload_status(id: &str) -> Outcome<SessionView> {
    let controller = abort_controller()?;
    let request = Request::get(&format!("/api/uploads/{id}"))
        .abort_signal(Some(&controller.signal()))
        .build()
        .map_err(|e| Failure::Network(e.to_string()))?;
    decode(send_with_timeout(request, controller, REQUEST_TIMEOUT_MS).await?).await
}

/// Commit the upload: bounded, probed, and able to adopt a commit whose
/// answer never arrived. The backend deletes the session when the commit goes
/// through, so a status probe that starts answering 404 means success.
async fn finish(id: &str, size: f64, name: &str) -> Outcome<FileInfo> {
    let mut answer_lost = false;
    for _ in 0..FINISH_ATTEMPTS {
        let controller = abort_controller()?;
        let request = Request::post(&format!("/api/uploads/{id}/finish"))
            .abort_signal(Some(&controller.signal()))
            .json(&json!({}))
            .map_err(|e| Failure::Network(e.to_string()))?;
        let race = controller.clone();
        let attempt = async move {
            let response = send_with_timeout(request, race, finish_timeout_ms(size)).await?;
            decode::<FileInfo>(response).await
        };
        let probe = async {
            loop {
                gloo_timers::future::TimeoutFuture::new(PROBE_INTERVAL_MS).await;
                if let Err(Failure::Status(404, _)) = upload_status(id).await {
                    return;
                }
            }
        };
        match select(Box::pin(attempt), Box::pin(probe)).await {
            Either::Left((Ok(file), _)) => return Ok(file),
            Either::Left((Err(e), _)) => match e {
                // No answer: the commit may or may not have happened. Try
                // again; a 404 from here on means it had.
                Failure::Network(_) => answer_lost = true,
                Failure::Status(404, _) if answer_lost => return adopt_committed(name, size).await,
                other => return Err(other),
            },
            Either::Right(_) => {
                controller.abort();
                return adopt_committed(name, size).await;
            }
        }
    }
    Err(Failure::Network(TIMED_OUT.into()))
}

/// Find the file a lost `finish` answer was carrying: the newest stored file
/// with this upload's name and size.
async fn adopt_committed(name: &str, size: f64) -> Outcome<FileInfo> {
    let files: Vec<FileInfo> = crate::api::api("GET", "/api/files", None)
        .await
        .map_err(Failure::Network)?;
    files
        .into_iter()
        .filter(|f| f.name == name && f.size as f64 == size)
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .ok_or_else(|| {
            Failure::Network("The upload finished, but its record could not be found".into())
        })
}

/// Abandon an upload so the backend reclaims its disk now rather than at the
/// next sweep.
#[allow(dead_code)]
pub async fn abort_upload(id: &str) {
    if let Ok(request) = Request::delete(&format!("/api/uploads/{id}")).build() {
        let _ = request.send().await;
    }
}

/// Race a request carrying `controller`'s signal against `timeout_ms`,
/// aborting it on the clock rather than leaving it to hang.
async fn send_with_timeout(
    request: Request,
    controller: web_sys::AbortController,
    timeout_ms: u32,
) -> Outcome<Response> {
    match select(
        Box::pin(request.send()),
        Box::pin(gloo_timers::future::TimeoutFuture::new(timeout_ms)),
    )
    .await
    {
        Either::Left((result, _)) => result.map_err(|_| {
            Failure::Network("The connection dropped while sending this video".into())
        }),
        Either::Right(_) => {
            // Also lets the next attempt at this offset land cleanly instead of
            // racing a chunk this attempt is still sending.
            controller.abort();
            Err(Failure::Network(TIMED_OUT.into()))
        }
    }
}

async fn decode<T: serde::de::DeserializeOwned>(response: Response) -> Outcome<T> {
    if !response.ok() {
        let status = response.status();
        let value: Value = response.json().await.unwrap_or_default();
        return Err(Failure::Status(
            status,
            value["error"]
                .as_str()
                .unwrap_or("The backend could not complete this upload")
                .into(),
        ));
    }
    response
        .json()
        .await
        .map_err(|_| Failure::Network("Unexpected response from the backend".into()))
}

fn abort_controller() -> Outcome<web_sys::AbortController> {
    web_sys::AbortController::new()
        .map_err(|_| Failure::Network("Could not prepare the request".into()))
}

// An upload that fails halfway should cost the half that failed. The backend
// keeps the partial file and says where it got to, but only if the next attempt
// knows which session to ask about, so the id outlives the attempt.
const RESUME_PREFIX: &str = "causewaybay.upload.resume.";

/// The identity of an upload for resuming: the content digest, not the name
/// (renaming between attempts must not orphan a session), plus destination
/// and size.
fn resume_key(target: &VideoTarget, digest: &str, size: f64) -> String {
    let dest = match &target.room {
        Some(room) => format!("room:{room}"),
        None if target.wiki => "videos".into(),
        None => "files".into(),
    };
    format!("{RESUME_PREFIX}{dest}:{digest}:{size}")
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}
fn remember_session(key: &str, id: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, id);
    }
}
fn stored_session(key: &str) -> Option<String> {
    storage().and_then(|s| s.get_item(key).ok().flatten())
}
fn forget_session(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}

/// SHA-256 of a blob, read in slices.
async fn checksum(blob: &web_sys::Blob, size: f64, report: &impl Fn(Progress)) -> Outcome<String> {
    let mut hasher = Sha256::new();
    let mut at = 0.0f64;
    while at < size {
        let end = (at + CHECKSUM_READ).min(size);
        hasher.update(read_slice(blob, at, end).await?.to_vec());
        at = end;
        report(Progress {
            phase: Phase::Checksum,
            done: at,
            total: size,
        });
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// `[start, end)` of a blob as bytes, for the checksum pass only. The
/// `f64` slice overload, because an `i32` offset overflows at 2 GB.
async fn read_slice(blob: &web_sys::Blob, start: f64, end: f64) -> Outcome<js_sys::Uint8Array> {
    let unreadable = || Failure::Network("Could not read this video".into());
    let part = blob
        .slice_with_f64_and_f64(start, end)
        .map_err(|_| unreadable())?;
    let buffer = JsFuture::from(part.array_buffer())
        .await
        .map_err(|_| unreadable())?
        .dyn_into::<js_sys::ArrayBuffer>()
        .map_err(|_| unreadable())?;
    Ok(js_sys::Uint8Array::new(&buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_is_bounded_and_survives_an_empty_file() {
        let p = |done, total| Progress {
            phase: Phase::Upload,
            done,
            total,
        };
        assert_eq!(p(50.0, 100.0).fraction(), 0.5);
        assert_eq!(p(0.0, 0.0).fraction(), 1.0);
        assert_eq!(p(150.0, 100.0).fraction(), 1.0);
    }

    #[test]
    fn a_resume_key_identifies_content_and_destination() {
        let room = |id: &str| VideoTarget {
            room: Some(id.into()),
            ..Default::default()
        };
        let a = resume_key(&room("r1"), &"aa".repeat(32), 100.0);
        assert_ne!(a, resume_key(&room("r1"), &"bb".repeat(32), 100.0));
        assert_ne!(a, resume_key(&room("r1"), &"aa".repeat(32), 101.0));
        assert_ne!(a, resume_key(&room("r2"), &"aa".repeat(32), 100.0));
        let videos = VideoTarget {
            wiki: true,
            ..Default::default()
        };
        assert_ne!(a, resume_key(&videos, &"aa".repeat(32), 100.0));
    }

    #[test]
    fn a_chunks_clock_scales_with_its_size_and_halving_shrinks() {
        assert_eq!(append_timeout_ms(0.0), REQUEST_TIMEOUT_MS);
        assert!(append_timeout_ms(8.0 * 1024.0 * 1024.0) > REQUEST_TIMEOUT_MS + 60_000);
        let ceiling = FALLBACK_CHUNK;
        let chunk = ceiling.min(INITIAL_CHUNK);
        assert_eq!(chunk, FALLBACK_CHUNK);
        assert_eq!((chunk * 2.0).min(ceiling), FALLBACK_CHUNK);
        let after_timeout = (chunk / 2.0).max(MIN_CHUNK).min(ceiling);
        assert!(after_timeout < chunk && after_timeout <= ceiling);
    }

    #[test]
    fn the_device_ceiling_matches_the_backend() {
        assert_eq!(MAX_VIDEO_BYTES, 1_073_741_824.0);
    }
}
