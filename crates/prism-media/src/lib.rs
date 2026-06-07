//! prism-media — the suite's app-agnostic A/V decode bridge.
//!
//! This crate probes media metadata and decodes individual video frames + whole
//! audio tracks for the suite's video apps (Reel, Pulse). It is intentionally
//! free of any app, GPU, or UI types: a [`VideoFrame`] is a flat 8-bit RGBA
//! buffer and an [`AudioBuffer`] is interleaved `f32` — the caller uploads /
//! plays them however it likes.
//!
//! **Backend: the ffmpeg / ffprobe CLI.** Rather than link against libav (the
//! `ffmpeg-sys` / `ffmpeg-next` bindings need `pkg-config` and lag new FFmpeg
//! releases), prism-media shells out to the `ffmpeg` and `ffprobe` *binaries*
//! via [`std::process::Command`]. This is version-tolerant (works with FFmpeg
//! 8.x and needs no linking) and keeps the decode path behind a small surface
//! ([`probe`], [`decode_frame_at`], [`decode_audio`]) so it can be swapped for
//! an in-process libav backend later without touching callers.
//!
//! The binary paths default to `ffmpeg` / `ffprobe` (resolved on `PATH`) and are
//! overridable via the `PRISM_FFMPEG` / `PRISM_FFPROBE` environment variables.
//!
//! **Graceful degradation.** A missing binary surfaces as
//! [`MediaError::BinaryNotFound`] (never a panic), so a caller can fall back to a
//! placeholder when FFmpeg isn't installed.

use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use thiserror::Error;

/// Errors from probing or decoding media.
#[derive(Debug, Error)]
pub enum MediaError {
    /// The `ffmpeg` / `ffprobe` binary could not be found / spawned. Callers
    /// should degrade gracefully (e.g. draw a placeholder) rather than fail hard.
    #[error("ffmpeg/ffprobe binary not found: {0}")]
    BinaryNotFound(String),
    /// `ffprobe` ran but failed (non-zero exit), with its stderr.
    #[error("probe failed: {0}")]
    Probe(String),
    /// `ffmpeg` ran but failed (non-zero exit / short read), with detail.
    #[error("decode failed: {0}")]
    Decode(String),
    /// The probe JSON could not be parsed / lacked an expected field.
    #[error("parse failed: {0}")]
    Parse(String),
    /// An underlying I/O error spawning a process or reading its output.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Audio-stream metadata from a probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    /// Samples per second per channel (e.g. 48000).
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo, …).
    pub channels: u16,
    /// The audio codec name reported by ffprobe (e.g. `aac`), if known.
    pub codec: Option<String>,
}

/// Probed media metadata: container duration plus the first video stream's
/// geometry / rate and whether the file carries audio.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    /// Total duration in seconds (container `format.duration`).
    pub duration_secs: f64,
    /// Video width in pixels (first video stream).
    pub width: u32,
    /// Video height in pixels (first video stream).
    pub height: u32,
    /// Frames per second (first video stream's `avg_frame_rate`).
    pub fps: f64,
    /// True if the file carries at least one audio stream.
    pub has_audio: bool,
    /// The video codec name (e.g. `h264`), if a video stream was found.
    pub video_codec: Option<String>,
    /// The first audio stream's metadata, if any.
    pub audio: Option<AudioInfo>,
}

/// A single decoded video frame: tightly packed **8-bit RGBA, straight
/// (non-premultiplied) alpha, sRGB**, `width * height * 4` bytes, top-left
/// origin. This matches what egui / wgpu expect for an sRGB texture upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes of straight-alpha sRGB RGBA.
    pub rgba: Vec<u8>,
}

/// A decoded audio track: interleaved 32-bit float PCM in `[-1, 1]`.
///
/// `samples.len() == frames * channels`; channel `c` of frame `f` is
/// `samples[f * channels + c]`. Whole-file decode (no streaming) for now.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved `f32` samples (`frames * channels` long).
    pub samples: Vec<f32>,
}

/// The configured `ffmpeg` binary (`$PRISM_FFMPEG`, else `ffmpeg`).
fn ffmpeg_bin() -> String {
    std::env::var("PRISM_FFMPEG").unwrap_or_else(|_| "ffmpeg".to_string())
}

/// The configured `ffprobe` binary (`$PRISM_FFPROBE`, else `ffprobe`).
fn ffprobe_bin() -> String {
    std::env::var("PRISM_FFPROBE").unwrap_or_else(|_| "ffprobe".to_string())
}

/// Map a spawn error: a not-found binary becomes [`MediaError::BinaryNotFound`]
/// (so callers can degrade), any other I/O error is passed through.
fn spawn_err(bin: &str, e: std::io::Error) -> MediaError {
    if e.kind() == std::io::ErrorKind::NotFound {
        MediaError::BinaryNotFound(bin.to_string())
    } else {
        MediaError::Io(e)
    }
}

// --- ffprobe JSON shape -----------------------------------------------------

#[derive(Deserialize)]
struct ProbeJson {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Deserialize, Default)]
struct ProbeFormat {
    /// Duration is a string in ffprobe JSON (e.g. `"1.000000"`).
    duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    /// ffprobe reports sample_rate as a string (e.g. `"48000"`).
    sample_rate: Option<String>,
    channels: Option<u16>,
}

/// Parse an ffprobe rational rate string (`"30000/1001"`, `"25/1"`, `"0/0"`)
/// into fps. A zero / malformed denominator yields `0.0`.
fn parse_rate(s: &str) -> f64 {
    let mut it = s.split('/');
    let num: f64 = it.next().and_then(|n| n.parse().ok()).unwrap_or(0.0);
    let den: f64 = it.next().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    if den.abs() < f64::EPSILON {
        0.0
    } else {
        num / den
    }
}

/// Run `ffprobe -v quiet -print_format json -show_format -show_streams <path>`
/// and parse the JSON. Shared by [`probe`] and [`probe_audio`].
fn run_ffprobe(path: &Path) -> Result<ProbeJson, MediaError> {
    let bin = ffprobe_bin();
    let output = Command::new(&bin)
        .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|e| spawn_err(&bin, e))?;

    if !output.status.success() {
        return Err(MediaError::Probe(format!(
            "ffprobe exited {} for {}",
            output.status,
            path.display()
        )));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| MediaError::Parse(format!("ffprobe json: {e}")))
}

/// The container duration (seconds) from a parsed probe, `0.0` when absent.
fn duration_of(json: &ProbeJson) -> f64 {
    json.format
        .duration
        .as_deref()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// Build an [`AudioInfo`] from the first audio stream in a parsed probe, if any.
fn first_audio_info(json: &ProbeJson) -> Option<AudioInfo> {
    json.streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .map(|a| AudioInfo {
            sample_rate: a
                .sample_rate
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            channels: a.channels.unwrap_or(0),
            codec: a.codec_name.clone(),
        })
}

/// Probe `path` for its container duration and first video/audio stream info via
/// `ffprobe -v quiet -print_format json -show_format -show_streams <path>`.
///
/// Requires a video stream (it is the video-media probe; for an **audio-only**
/// file use [`probe_audio`]).
pub fn probe(path: impl AsRef<Path>) -> Result<MediaInfo, MediaError> {
    let path = path.as_ref();
    let json = run_ffprobe(path)?;

    let video = json
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .ok_or_else(|| MediaError::Parse("no video stream".to_string()))?;

    let width = video.width.unwrap_or(0);
    let height = video.height.unwrap_or(0);
    // Prefer avg_frame_rate; fall back to r_frame_rate when it is 0/unknown.
    let fps = video
        .avg_frame_rate
        .as_deref()
        .map(parse_rate)
        .filter(|f| *f > 0.0)
        .or_else(|| video.r_frame_rate.as_deref().map(parse_rate))
        .unwrap_or(0.0);

    let audio_info = first_audio_info(&json);

    Ok(MediaInfo {
        duration_secs: duration_of(&json),
        width,
        height,
        fps,
        has_audio: audio_info.is_some(),
        video_codec: video.codec_name.clone(),
        audio: audio_info,
    })
}

/// Probe an **audio-only** file (or any file's audio) for its container duration
/// and first audio stream — the audio analogue of [`probe`], which requires a
/// video stream and so rejects a pure audio file (`.mp3`, `.wav`, …).
///
/// Returns a [`MediaInfo`] with `width`/`height`/`fps` zeroed and `video_codec`
/// `None` (there is no video), `has_audio` / `audio` set from the first audio
/// stream. Errors with [`MediaError::Parse`] when the file carries no audio
/// stream at all.
pub fn probe_audio(path: impl AsRef<Path>) -> Result<MediaInfo, MediaError> {
    let path = path.as_ref();
    let json = run_ffprobe(path)?;

    let audio_info =
        first_audio_info(&json).ok_or_else(|| MediaError::Parse("no audio stream".to_string()))?;

    Ok(MediaInfo {
        duration_secs: duration_of(&json),
        width: 0,
        height: 0,
        fps: 0.0,
        has_audio: true,
        video_codec: None,
        audio: Some(audio_info),
    })
}

/// Decode a single video frame at `t_secs` into the file as straight-alpha sRGB
/// RGBA.
///
/// Runs `ffmpeg -ss <t> -i <path> -frames:v 1 -f rawvideo -pix_fmt rgba [-vf
/// scale=w:h] -v error -` and reads exactly `width * height * 4` bytes from
/// stdout. The output geometry is `scale` when given, otherwise the file's
/// probed `width`/`height` (so the caller need not probe first for native-size
/// frames). `-ss` before `-i` is an input seek (fast, keyframe-accurate enough
/// for scrubbing).
pub fn decode_frame_at(
    path: impl AsRef<Path>,
    t_secs: f64,
    scale: Option<(u32, u32)>,
) -> Result<VideoFrame, MediaError> {
    let path = path.as_ref();

    // Determine the output geometry: explicit scale, else the probed size.
    let (w, h) = match scale {
        Some((w, h)) => (w, h),
        None => {
            let info = probe(path)?;
            (info.width, info.height)
        }
    };
    if w == 0 || h == 0 {
        return Err(MediaError::Decode(format!(
            "zero-size frame ({w}x{h}) for {}",
            path.display()
        )));
    }

    let bin = ffmpeg_bin();
    let mut cmd = Command::new(&bin);
    // Input seek before -i for a fast seek to ~t.
    cmd.arg("-ss").arg(format!("{t_secs}"));
    cmd.arg("-i").arg(path);
    cmd.args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba"]);
    if let Some((sw, sh)) = scale {
        cmd.arg("-vf").arg(format!("scale={sw}:{sh}"));
    }
    cmd.args(["-v", "error", "-"]);

    let output = cmd.output().map_err(|e| spawn_err(&bin, e))?;
    if !output.status.success() {
        return Err(MediaError::Decode(format!(
            "ffmpeg exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let expected = (w as usize) * (h as usize) * 4;
    if output.stdout.len() < expected {
        return Err(MediaError::Decode(format!(
            "short frame read: got {} bytes, expected {expected} ({w}x{h})",
            output.stdout.len()
        )));
    }

    let mut rgba = output.stdout;
    rgba.truncate(expected);
    Ok(VideoFrame { width: w, height: h, rgba })
}

/// Decode the whole audio track to interleaved `f32` PCM at `sample_rate` /
/// `channels`.
///
/// Runs `ffmpeg -i <path> -f f32le -ac <channels> -ar <sample_rate> -v error -`
/// and reinterprets the little-endian `f32` stdout as interleaved samples.
/// Whole-file decode (no streaming) — acceptable for the current pass.
pub fn decode_audio(
    path: impl AsRef<Path>,
    sample_rate: u32,
    channels: u16,
) -> Result<AudioBuffer, MediaError> {
    let path = path.as_ref();
    let bin = ffmpeg_bin();
    let output = Command::new(&bin)
        .arg("-i")
        .arg(path)
        .args([
            "-f",
            "f32le",
            "-ac",
            &channels.to_string(),
            "-ar",
            &sample_rate.to_string(),
            "-v",
            "error",
            "-",
        ])
        .output()
        .map_err(|e| spawn_err(&bin, e))?;

    if !output.status.success() {
        return Err(MediaError::Decode(format!(
            "ffmpeg audio exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // Reinterpret the LE f32 byte stream as samples (drop a trailing partial).
    let bytes = output.stdout;
    let n = bytes.len() / 4;
    let mut samples = Vec::with_capacity(n);
    for chunk in bytes.chunks_exact(4) {
        samples.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    Ok(AudioBuffer { sample_rate, channels, samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a 1s 64x48 @10fps test clip into `dir/t.mp4` via FFmpeg's lavfi
    /// `testsrc`. Returns `false` (skip) when FFmpeg isn't installed, mirroring
    /// the suite's "GPU test skips silently when no adapter" convention.
    fn make_clip(path: &Path) -> bool {
        let bin = ffmpeg_bin();
        let status = Command::new(&bin)
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=64x48:rate=10",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(path)
            .args(["-v", "error"])
            .status();
        match status {
            Ok(s) => s.success(),
            // ffmpeg missing → skip the suite of gated tests.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => false,
        }
    }

    /// A scratch path under the OS temp dir (process-id-scoped to avoid clashes).
    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("prism_media_{}_{name}", std::process::id()));
        p
    }

    #[test]
    fn probe_and_decode_generated_clip() {
        let clip = temp_path("probe.mp4");
        if !make_clip(&clip) {
            eprintln!("ffmpeg not available — skipping prism-media decode test");
            return;
        }

        // probe: ~1.0s, 64x48, ~10fps, a video codec, no audio.
        let info = match probe(&clip) {
            Ok(i) => i,
            Err(MediaError::BinaryNotFound(_)) => {
                let _ = std::fs::remove_file(&clip);
                return;
            }
            Err(e) => panic!("probe failed: {e}"),
        };
        assert!(
            (info.duration_secs - 1.0).abs() < 0.2,
            "duration ~1.0s, got {}",
            info.duration_secs
        );
        assert_eq!(info.width, 64);
        assert_eq!(info.height, 48);
        assert!((info.fps - 10.0).abs() < 0.5, "fps ~10, got {}", info.fps);
        assert!(info.video_codec.is_some());
        assert!(!info.has_audio);

        // decode native-size frame at 0.5s: 64*48*4 bytes.
        let frame = decode_frame_at(&clip, 0.5, None).expect("decode native frame");
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert_eq!(frame.rgba.len(), 64 * 48 * 4);

        // decode scaled frame: 32*24*4 bytes.
        let scaled = decode_frame_at(&clip, 0.5, Some((32, 24))).expect("decode scaled frame");
        assert_eq!(scaled.width, 32);
        assert_eq!(scaled.height, 24);
        assert_eq!(scaled.rgba.len(), 32 * 24 * 4);

        let _ = std::fs::remove_file(&clip);
    }

    #[test]
    fn decode_audio_of_generated_clip() {
        // A clip WITH an audio track (sine) so decode_audio has something to read.
        let clip = temp_path("audio.mp4");
        let bin = ffmpeg_bin();
        let made = Command::new(&bin)
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=64x48:rate=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-pix_fmt",
                "yuv420p",
                "-shortest",
                "-y",
            ])
            .arg(&clip)
            .args(["-v", "error"])
            .status();
        match made {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("ffmpeg not available — skipping prism-media audio test");
                return;
            }
        }

        let info = probe(&clip).expect("probe audio clip");
        assert!(info.has_audio);

        let buf = match decode_audio(&clip, 48000, 2) {
            Ok(b) => b,
            Err(MediaError::BinaryNotFound(_)) => {
                let _ = std::fs::remove_file(&clip);
                return;
            }
            Err(e) => panic!("decode_audio failed: {e}"),
        };
        assert_eq!(buf.sample_rate, 48000);
        assert_eq!(buf.channels, 2);
        // ~1s of stereo @48k ≈ 96000 interleaved samples; just assert non-empty
        // and an even (stereo-interleaved) length.
        assert!(!buf.samples.is_empty());
        assert_eq!(buf.samples.len() % 2, 0);

        let _ = std::fs::remove_file(&clip);
    }

    #[test]
    fn probe_audio_of_audio_only_file() {
        // An audio-only file (no video stream) that `probe` rejects but
        // `probe_audio` accepts, reporting duration + audio stream info.
        let clip = temp_path("audio_only.wav");
        let bin = ffmpeg_bin();
        let made = Command::new(&bin)
            .args([
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-ar",
                "44100",
                "-ac",
                "2",
                "-y",
            ])
            .arg(&clip)
            .args(["-v", "error"])
            .status();
        match made {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("ffmpeg not available — skipping probe_audio test");
                return;
            }
        }

        // `probe` rejects an audio-only file (no video stream).
        assert!(matches!(probe(&clip), Err(MediaError::Parse(_))));

        // `probe_audio` accepts it: ~1s, no video geometry, an audio stream.
        let info = match probe_audio(&clip) {
            Ok(i) => i,
            Err(MediaError::BinaryNotFound(_)) => {
                let _ = std::fs::remove_file(&clip);
                return;
            }
            Err(e) => panic!("probe_audio failed: {e}"),
        };
        assert!((info.duration_secs - 1.0).abs() < 0.2, "duration {}", info.duration_secs);
        assert_eq!(info.width, 0);
        assert_eq!(info.height, 0);
        assert!(info.video_codec.is_none());
        assert!(info.has_audio);
        let audio = info.audio.expect("audio stream");
        assert_eq!(audio.sample_rate, 44100);
        assert_eq!(audio.channels, 2);

        let _ = std::fs::remove_file(&clip);
    }

    #[test]
    fn missing_binary_is_binary_not_found() {
        // An override pointing at a nonexistent binary must surface as
        // BinaryNotFound (never a panic / generic Io), so callers degrade.
        std::env::set_var("PRISM_FFPROBE", "prism_media_definitely_missing_binary");
        let res = probe("whatever.mp4");
        std::env::remove_var("PRISM_FFPROBE");
        assert!(matches!(res, Err(MediaError::BinaryNotFound(_))), "got {res:?}");
    }

    #[test]
    fn parse_rate_handles_rationals_and_zero() {
        assert!((parse_rate("30/1") - 30.0).abs() < 1e-9);
        assert!((parse_rate("30000/1001") - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("0/0"), 0.0);
        assert_eq!(parse_rate("garbage"), 0.0);
    }
}
