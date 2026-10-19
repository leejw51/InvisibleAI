//! Audio Capture on the capture Mac: the microphone is opened only when the
//! backend asks, its signal is resampled to 16 kHz mono and cut into
//! utterances at pauses, and each utterance leaves as a small PCM WAV on the
//! client WebSocket. Nothing is transcribed here; the backend's local speech
//! engine does that. A `--fixture-audio` file replays as if it were spoken.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const SAMPLE_RATE: usize = 16_000;
/// Longest utterance sent in one piece (the speech engines' 30-second window).
const MAX_UTTERANCE: usize = 30 * SAMPLE_RATE;
/// RMS above this counts as speech.
const VAD_THRESHOLD: f32 = 0.003;
/// A pause this long ends an utterance.
const SILENCE_MS: usize = 900;
/// Utterances with less speech than this are noise.
const MIN_SPEECH_MS: usize = 300;
/// Audio kept from just before speech starts, so the first syllable is not clipped.
const PREROLL_MS: usize = 300;

// ---- resampling ----

/// Streaming converter from the device format to 16 kHz mono; keeps state across chunks.
pub struct Resampler {
    channels: usize,
    rate: usize,
    pending: Vec<f32>,
    /// Fractional read position into `pending` (non-integer ratios only).
    pos: f64,
}
impl Resampler {
    pub fn new(channels: usize, rate: usize) -> Self {
        Self {
            channels: channels.max(1),
            rate: rate.max(1),
            pending: Vec::new(),
            pos: 0.0,
        }
    }
    pub fn push(&mut self, interleaved: &[f32]) -> Vec<f32> {
        self.pending.extend(
            interleaved
                .chunks(self.channels)
                .map(|c| c.iter().sum::<f32>() / c.len() as f32),
        );
        if self.rate == SAMPLE_RATE {
            return std::mem::take(&mut self.pending);
        }
        let mut out = Vec::new();
        if self.rate.is_multiple_of(SAMPLE_RATE) {
            // Integer ratio (48 kHz is the usual mic rate): box-average every `k` samples.
            let k = self.rate / SAMPLE_RATE;
            let n = self.pending.len() / k * k;
            out.extend(
                self.pending[..n]
                    .chunks_exact(k)
                    .map(|c| c.iter().sum::<f32>() / k as f32),
            );
            self.pending.drain(..n);
        } else {
            let step = self.rate as f64 / SAMPLE_RATE as f64;
            while (self.pos as usize) + 1 < self.pending.len() {
                let j = self.pos as usize;
                let frac = (self.pos - j as f64) as f32;
                out.push(self.pending[j] + (self.pending[j + 1] - self.pending[j]) * frac);
                self.pos += step;
            }
            let consumed = (self.pos as usize).min(self.pending.len());
            self.pending.drain(..consumed);
            self.pos -= consumed as f64;
        }
        out
    }
}

// ---- voice activity detection ----

/// Energy-based detector that cuts the stream into utterances.
pub struct Vad {
    /// 20 ms analysis frames.
    pub frame: usize,
    preroll: Vec<f32>,
    utterance: Vec<f32>,
    in_speech: bool,
    speech_samples: usize,
    trailing_silence: usize,
}
impl Default for Vad {
    fn default() -> Self {
        Self::new()
    }
}
impl Vad {
    pub fn new() -> Self {
        Self {
            frame: 20 * SAMPLE_RATE / 1000,
            preroll: Vec::new(),
            utterance: Vec::new(),
            in_speech: false,
            speech_samples: 0,
            trailing_silence: 0,
        }
    }
    pub fn in_speech(&self) -> bool {
        self.in_speech
    }
    /// Feed one 20 ms frame; returns a finished utterance when a pause (or the cap) ends it.
    pub fn push(&mut self, frame: &[f32]) -> Option<Vec<f32>> {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt();
        let voiced = rms > VAD_THRESHOLD;
        if !self.in_speech {
            self.preroll.extend_from_slice(frame);
            let keep = PREROLL_MS * SAMPLE_RATE / 1000;
            if self.preroll.len() > keep {
                self.preroll.drain(..self.preroll.len() - keep);
            }
            if !voiced {
                return None;
            }
            self.in_speech = true;
            self.utterance = std::mem::take(&mut self.preroll);
            self.speech_samples = 0;
            self.trailing_silence = 0;
        } else {
            self.utterance.extend_from_slice(frame);
        }
        if voiced {
            self.speech_samples += frame.len();
            self.trailing_silence = 0;
        } else {
            self.trailing_silence += frame.len();
        }
        let done = self.trailing_silence >= SILENCE_MS * SAMPLE_RATE / 1000
            || self.utterance.len() >= MAX_UTTERANCE;
        if !done {
            return None;
        }
        self.take()
    }
    /// End the utterance being collected (when listening stops).
    pub fn take(&mut self) -> Option<Vec<f32>> {
        self.in_speech = false;
        let audio = std::mem::take(&mut self.utterance);
        (self.speech_samples >= MIN_SPEECH_MS * SAMPLE_RATE / 1000).then_some(audio)
    }
}

// ---- WAV ----

/// 16 kHz mono 16-bit PCM WAV bytes.
pub fn wav_bytes(audio: &[f32]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &v in audio {
            writer.write_sample((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        writer.finalize()?;
    }
    Ok(cursor.into_inner())
}

/// A WAV file as 16 kHz mono samples, whatever it was recorded at.
pub fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1u32 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    Ok(Resampler::new(spec.channels as usize, spec.sample_rate as usize).push(&samples))
}

/// The chunk message for one utterance.
pub fn chunk_message(generation: &str, seq: u64, audio: &[f32]) -> Result<Value> {
    // Lift quiet speech without clipping; never amplify silence into speech.
    let peak = audio.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
    let gain = if peak > VAD_THRESHOLD {
        (0.7 / peak).clamp(1.0, 8.0)
    } else {
        1.0
    };
    let audio: Vec<f32> = audio.iter().map(|v| v * gain).collect();
    Ok(json!({
        "type":"audio_chunk",
        "generation":generation,
        "seq":seq,
        "seconds":audio.len() as f64 / SAMPLE_RATE as f64,
        "wav_base64":STANDARD.encode(wav_bytes(&audio)?),
    }))
}

/// Meter updates and rolling speech snapshots never block microphone capture.
/// Like Jarvis's separate capture/transcription paths, a busy recognizer may
/// skip previews while completed utterances still use reliable delivery.
#[derive(Default)]
struct Live {
    frames: usize,
    peak: f32,
}
impl Live {
    fn frame(
        &mut self,
        frame: &[f32],
        vad: &Vad,
        generation: &str,
        seq: u64,
        sender: &tokio::sync::mpsc::Sender<Value>,
    ) {
        let rms = (frame.iter().map(|v| v * v).sum::<f32>() / frame.len().max(1) as f32).sqrt();
        self.peak = self.peak.max(rms);
        self.frames += 1;
        if self.frames.is_multiple_of(10) {
            let _ = sender.try_send(json!({"type":"audio_activity","generation":generation,
                "listening":true,"level":(self.peak / 0.12).sqrt().clamp(0.0,1.0)}));
            self.peak = 0.0;
        }
        if self.frames.is_multiple_of(50)
            && vad.in_speech
            && vad.speech_samples >= MIN_SPEECH_MS * SAMPLE_RATE / 1000
            && let Ok(mut preview) = chunk_message(generation, seq + 1, &vad.utterance)
        {
            preview["type"] = json!("audio_preview");
            let _ = sender.try_send(preview);
        }
    }
}

// ---- the recorder ----

/// A listening run: a thread that owns the microphone (or replays a fixture)
/// until `stop` is set, sending every utterance through `sender`.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    /// Joined only by tests; the connection task never waits on the thread.
    #[cfg_attr(not(test), allow(dead_code))]
    thread: Option<std::thread::JoinHandle<()>>,
    pub generation: String,
}
impl Recorder {
    pub fn start(
        generation: String,
        fixture: Option<PathBuf>,
        sender: tokio::sync::mpsc::Sender<Value>,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let g = generation.clone();
        // The microphone is opened on the thread so a missing or denied device
        // is reported before the backend is told listening started.
        let (opened_tx, opened_rx) = std::sync::mpsc::channel::<Result<()>>();
        let thread = std::thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                let outcome = match fixture {
                    Some(path) => replay(&path, &g, &flag, &sender, &opened_tx),
                    None => listen(&g, &flag, &sender, &opened_tx),
                };
                if let Err(e) = outcome {
                    eprintln!("Audio capture ended: {e:#}");
                    let _ = sender.try_send(json!({"type":"audio_activity","generation":g,
                        "listening":false,"error":e.to_string()}));
                    let _ = opened_tx.send(Err(e));
                } else {
                    let _ = sender.try_send(
                        json!({"type":"audio_activity","generation":g,"listening":false}),
                    );
                }
            })?;
        match opened_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
                generation,
            }),
            Ok(Err(e)) => {
                stop.store(true, Ordering::SeqCst);
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                anyhow::bail!("The microphone did not open within 15 seconds")
            }
        }
    }
    /// Ask the thread to stop; it closes the microphone and exits on its own.
    /// Never joined from the connection task: the thread may be waiting to hand
    /// over an utterance that only that task can take.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    #[cfg(test)]
    pub fn wait(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Hand one utterance to the connection task. The channel is small and drained
/// by that task alone, so this waits by polling and gives up when told to stop,
/// never blocking in a way the task would have to unblock.
fn send(
    sender: &tokio::sync::mpsc::Sender<Value>,
    stop: &AtomicBool,
    generation: &str,
    seq: &mut u64,
    audio: &[f32],
) -> Result<()> {
    *seq += 1;
    println!(
        "Heard {:.1} s of speech; sending utterance {}.",
        audio.len() as f64 / SAMPLE_RATE as f64,
        *seq
    );
    let mut message = chunk_message(generation, *seq, audio)?;
    loop {
        match sender.try_send(message) {
            Ok(()) => return Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                anyhow::bail!("Connection closed")
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(m)) => {
                ensure!(!stop.load(Ordering::SeqCst), "Stopped while sending");
                message = m;
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Replay a WAV as if it were spoken: 10 ms device chunks through the resampler
/// and the VAD, one utterance at a time, then wait quietly until stopped.
fn replay(
    path: &Path,
    generation: &str,
    stop: &AtomicBool,
    sender: &tokio::sync::mpsc::Sender<Value>,
    opened: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    let samples = read_wav(path)?;
    let _ = opened.send(Ok(()));
    println!(
        "TEST FIXTURE: replaying {} ({:.1} s) instead of the microphone.",
        path.display(),
        samples.len() as f64 / SAMPLE_RATE as f64
    );
    let mut vad = Vad::new();
    let mut live = Live::default();
    let mut seq = 0;
    // Trailing silence so the last utterance is closed like a real pause would.
    let tail = vec![0f32; SAMPLE_RATE];
    for frame in samples.chunks(vad.frame).chain(tail.chunks(vad.frame)) {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if frame.len() < vad.frame {
            continue;
        }
        if let Some(utterance) = vad.push(frame) {
            send(sender, stop, generation, &mut seq, &utterance)?;
        }
        live.frame(frame, &vad, generation, seq, sender);
        // Real time, so a backend test sees chunks arrive while it listens.
        std::thread::sleep(Duration::from_millis(2));
    }
    if vad.in_speech()
        && let Some(utterance) = vad.take()
        && let Ok(message) = chunk_message(generation, seq + 1, &utterance)
    {
        let _ = sender.try_send(message);
    }
    while !stop.load(Ordering::SeqCst) {
        let _ = sender.try_send(
            json!({"type":"audio_activity","generation":generation,"listening":true,"level":0.0}),
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn listen(
    generation: &str,
    stop: &AtomicBool,
    sender: &tokio::sync::mpsc::Sender<Value>,
    opened: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    use cpal::SampleFormat;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::Mutex;

    let device = cpal::default_host().default_input_device().context(
        "No microphone on the capture Mac. Connect one (AirPods, a USB mic, an iPhone) and press Capture Audio again.",
    )?;
    let config = device.default_input_config().context(
        "The microphone could not be configured. Allow microphone access for the app that started the capture client in System Settings → Privacy & Security → Microphone.",
    )?;
    let name = device
        .description()
        .map(|d| d.to_string())
        .unwrap_or_else(|_| "microphone".into());
    let sink: Arc<Mutex<Vec<f32>>> = Arc::default();
    let gone = Arc::new(AtomicBool::new(false));
    let err = {
        let gone = gone.clone();
        move |e: cpal::Error| {
            let text = e.to_string();
            if text.contains("underrun") || text.contains("overrun") {
                return;
            }
            eprintln!("Microphone stream error: {text}");
            gone.store(true, Ordering::Relaxed);
        }
    };
    let stream_config = config.config();
    let mut resampler = Resampler::new(config.channels() as usize, config.sample_rate() as usize);
    macro_rules! stream {
        ($t:ty, $conv:expr) => {{
            let sink = sink.clone();
            let mut scratch: Vec<f32> = Vec::new();
            device.build_input_stream::<$t, _, _>(
                stream_config.clone(),
                move |data: &[$t], _| {
                    scratch.clear();
                    scratch.extend(data.iter().map($conv));
                    let mono = resampler.push(&scratch);
                    sink.lock().unwrap().extend(mono);
                },
                err,
                None,
            )?
        }};
    }
    let stream = match config.sample_format() {
        SampleFormat::F32 => stream!(f32, |&s| s),
        SampleFormat::I16 => stream!(i16, |&s| s as f32 / 32768.0),
        SampleFormat::I32 => stream!(i32, |&s| s as f32 / 2147483648.0),
        SampleFormat::U16 => stream!(u16, |&s| (s as f32 - 32768.0) / 32768.0),
        f => anyhow::bail!("Unsupported microphone sample format {f:?}"),
    };
    stream.play().context("The microphone could not start")?;
    let _ = opened.send(Ok(()));
    println!(
        "Listening on {name} ({} ch, {} Hz). Audio stays on your backend Mac.",
        config.channels(),
        config.sample_rate()
    );
    let mut vad = Vad::new();
    let mut live = Live::default();
    let mut pending: Vec<f32> = Vec::new();
    let mut seq = 0;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
        ensure!(
            !gone.load(Ordering::Relaxed),
            "The microphone disconnected. Press Capture Audio to listen again."
        );
        pending.append(&mut sink.lock().unwrap());
        let n = pending.len() / vad.frame * vad.frame;
        for frame in pending[..n].chunks_exact(vad.frame) {
            if let Some(utterance) = vad.push(frame) {
                send(sender, stop, generation, &mut seq, &utterance)?;
            }
            live.frame(frame, &vad, generation, seq, sender);
        }
        pending.drain(..n);
    }
    // Whatever was being said when listening stopped still counts; the stop
    // flag is already set, so this only goes out if the channel has room.
    if vad.in_speech()
        && let Some(utterance) = vad.take()
        && let Ok(message) = chunk_message(generation, seq + 1, &utterance)
    {
        let _ = sender.try_send(message);
    }
    drop(stream);
    println!("Stopped listening.");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn listen(
    _generation: &str,
    _stop: &AtomicBool,
    _sender: &tokio::sync::mpsc::Sender<Value>,
    _opened: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    anyhow::bail!(
        "Microphone capture currently requires macOS. --fixture-audio is available for transport testing."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `secs` of a 440 Hz tone at `amp`, at `rate` Hz, `channels` interleaved.
    fn tone(secs: f32, amp: f32, rate: usize, channels: usize) -> Vec<f32> {
        let n = (secs * rate as f32) as usize;
        (0..n)
            .flat_map(|i| {
                let v = amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin();
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    #[test]
    fn resampler_handles_integer_and_fractional_ratios() {
        let mut rs = Resampler::new(2, 48000);
        let out = rs.push(&tone(1.0, 0.5, 48000, 2));
        assert_eq!(out.len(), SAMPLE_RATE);
        assert!(out.iter().all(|v| v.abs() <= 0.5));
        let mut rs = Resampler::new(1, 44100);
        let input = tone(1.0, 0.5, 44100, 1);
        let expected = Resampler::new(1, 44100).push(&input);
        let mut out = Vec::new();
        for chunk in input.chunks(512) {
            out.extend(rs.push(chunk));
        }
        assert_eq!(out.len(), expected.len());
        assert!(
            out.iter()
                .zip(&expected)
                .all(|(a, b)| (a - b).abs() < 0.0001)
        );
        assert!(
            (out.len() as i64 - SAMPLE_RATE as i64).abs() <= 2,
            "{}",
            out.len()
        );
    }

    #[test]
    fn vad_cuts_speech_at_a_pause_drops_blips_and_caps_length() {
        let mut vad = Vad::new();
        let frame = vad.frame;
        let mut audio = vec![0.0f32; SAMPLE_RATE / 2];
        audio.extend(tone(1.0, 0.3, SAMPLE_RATE, 1));
        audio.extend(vec![0.0f32; SAMPLE_RATE]);
        audio.extend(tone(0.04, 0.3, SAMPLE_RATE, 1)); // 40 ms blip: noise
        audio.extend(vec![0.0f32; SAMPLE_RATE]);
        let utterances: Vec<Vec<f32>> = audio
            .chunks_exact(frame)
            .filter_map(|f| vad.push(f))
            .collect();
        assert_eq!(utterances.len(), 1);
        let secs = utterances[0].len() as f32 / SAMPLE_RATE as f32;
        assert!((1.5..2.5).contains(&secs), "utterance is {secs} s");
        let mut vad = Vad::new();
        let long: Vec<Vec<f32>> = tone(31.0, 0.3, SAMPLE_RATE, 1)
            .chunks_exact(frame)
            .filter_map(|f| vad.push(f))
            .collect();
        assert!(!long.is_empty());
        assert!(long[0].len() <= MAX_UTTERANCE + frame);
        // Stopping mid-sentence still yields what was said.
        let mut vad = Vad::new();
        for f in tone(0.5, 0.3, SAMPLE_RATE, 1).chunks_exact(frame) {
            assert!(vad.push(f).is_none());
        }
        assert!(vad.in_speech());
        assert!(vad.take().is_some());
        assert!(!vad.in_speech());
    }

    #[test]
    fn quiet_speech_is_detected_and_previewed_without_waiting_for_a_pause() -> Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut vad = Vad::new();
        let mut live = Live::default();
        for frame in tone(1.2, 0.006, SAMPLE_RATE, 1).chunks_exact(vad.frame) {
            assert!(vad.push(frame).is_none());
            live.frame(frame, &vad, "quiet", 0, &tx);
        }
        assert!(vad.in_speech());
        let mut preview = None;
        while let Ok(event) = rx.try_recv() {
            if event["type"] == "audio_preview" {
                preview = Some(event);
            }
        }
        let preview = preview.context("quiet speech preview before the pause")?;
        assert_eq!(preview["seq"], 1);
        let bytes = STANDARD.decode(preview["wav_base64"].as_str().unwrap())?;
        let mut wav = hound::WavReader::new(std::io::Cursor::new(bytes))?;
        let peak = wav
            .samples::<i16>()
            .map(|v| v.unwrap().unsigned_abs())
            .max()
            .unwrap();
        assert!(peak > 1000, "quiet voice receives bounded gain");
        let mut silence = Vad::new();
        for frame in tone(2.0, 0.001, SAMPLE_RATE, 1).chunks_exact(silence.frame) {
            assert!(silence.push(frame).is_none());
        }
        assert!(
            !silence.in_speech(),
            "low-level noise does not start speech"
        );
        Ok(())
    }

    #[test]
    fn wav_round_trips_and_chunk_message_is_bounded() -> Result<()> {
        let audio = tone(0.5, 0.5, SAMPLE_RATE, 1);
        let bytes = wav_bytes(&audio)?;
        assert!(bytes.starts_with(b"RIFF"));
        assert_eq!(bytes.len(), 44 + audio.len() * 2);
        let path = std::env::temp_dir().join(format!("audio-test-{}.wav", std::process::id()));
        std::fs::write(&path, &bytes)?;
        let back = read_wav(&path)?;
        let _ = std::fs::remove_file(&path);
        assert_eq!(back.len(), audio.len());
        let peak = back.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.45..=0.5).contains(&peak), "peak {peak}");
        let message = chunk_message("gen", 3, &audio)?;
        assert_eq!(message["type"], "audio_chunk");
        assert_eq!(message["seq"], 3);
        assert!((message["seconds"].as_f64().unwrap() - 0.5).abs() < 1e-6);
        let longest = chunk_message("gen", 1, &vec![0.1f32; MAX_UTTERANCE])?;
        assert!(
            longest["wav_base64"].as_str().unwrap().len()
                <= causewaybay_shared::MAX_AUDIO_CHUNK_BYTES / 3 * 4
        );
        Ok(())
    }

    #[tokio::test]
    async fn fixture_replay_sends_utterances_then_waits_for_stop() -> Result<()> {
        let path = std::env::temp_dir().join(format!("audio-fixture-{}.wav", std::process::id()));
        let mut audio = tone(1.0, 0.3, SAMPLE_RATE, 1);
        audio.extend(vec![0.0f32; SAMPLE_RATE * 2]);
        audio.extend(tone(1.0, 0.3, SAMPLE_RATE, 1));
        std::fs::write(&path, wav_bytes(&audio)?)?;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut recorder = Recorder::start("gen".into(), Some(path.clone()), tx)?;
        let mut chunks = vec![];
        let mut previews = 0;
        let mut levels = 0;
        tokio::time::timeout(Duration::from_secs(10), async {
            while chunks.len() < 2 {
                let event = rx.recv().await.context("recorder event")?;
                assert_eq!(event["generation"], "gen");
                match event["type"].as_str() {
                    Some("audio_chunk") => chunks.push(event),
                    Some("audio_preview") => {
                        previews += 1;
                        assert!(event["wav_base64"].as_str().is_some());
                    }
                    Some("audio_activity") if event["level"].as_f64().unwrap_or(0.0) > 0.0 => {
                        levels += 1;
                    }
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        assert_eq!(chunks[0]["seq"], 1);
        assert_eq!(chunks[1]["seq"], 2);
        assert!(
            previews > 0,
            "partial audio arrives before utterances finish"
        );
        assert!(levels > 0, "actual sound levels are reported");
        assert!(!recorder.finished(), "replay keeps listening until stopped");
        recorder.wait();
        assert!(recorder.finished());
        let _ = std::fs::remove_file(&path);
        assert!(
            Recorder::start("gen".into(), Some(PathBuf::from("/nonexistent.wav")), {
                let (tx, _rx) = tokio::sync::mpsc::channel(1);
                tx
            })
            .is_err()
        );
        Ok(())
    }
}
