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

// --- Encode (H.264 MP4) -----------------------------------------------------

/// Parameters for an [`encode_h264`] run: the output geometry, frame rate, and a
/// destination path. The pixel format fed on stdin is always straight-alpha
/// `rgba` (matching [`VideoFrame`] and the suite's CPU renderers); ffmpeg
/// converts it to `yuv420p` for broad-player compatibility.
#[derive(Clone, Debug, PartialEq)]
pub struct EncodeParams {
    /// Frame width in pixels (must be > 0; even for `yuv420p`).
    pub width: u32,
    /// Frame height in pixels (must be > 0; even for `yuv420p`).
    pub height: u32,
    /// Output frame rate (frames per second; must be > 0).
    pub fps: f64,
    /// libx264 Constant Rate Factor (0 = lossless, ~18 visually lossless, 23
    /// default, 51 worst). Lower is higher quality / bigger file.
    pub crf: u32,
    /// The x264 `-preset` (encode speed↔compression trade-off), e.g. `medium`.
    pub preset: String,
}

impl EncodeParams {
    /// Sensible defaults for a delivery H.264: CRF 18 (visually lossless),
    /// `medium` preset.
    pub fn new(width: u32, height: u32, fps: f64) -> Self {
        Self { width, height, fps, crf: 18, preset: "medium".to_string() }
    }
}

/// Build the **pure** ffmpeg argument vector for an H.264 encode that reads raw
/// `rgba` frames from stdin and writes an MP4 to `out`. Kept free of any I/O so
/// the exact invocation is unit-testable:
///
/// ```text
/// ffmpeg -y -f rawvideo -pix_fmt rgba -s WxH -r FPS -i -
///        -c:v libx264 -pix_fmt yuv420p -preset PRESET -crf CRF
///        -movflags +faststart out.mp4
/// ```
///
/// `-y` overwrites an existing file; the input `-` is stdin; `+faststart`
/// relocates the moov atom for web playback. The `fps` is formatted with enough
/// precision to carry NTSC fractional rates (e.g. `29.97`).
pub fn encode_h264_args(params: &EncodeParams, out: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgba".to_string(),
        "-s".to_string(),
        format!("{}x{}", params.width, params.height),
        "-r".to_string(),
        format_rate(params.fps),
        "-i".to_string(),
        "-".to_string(),
        "-c:v".to_string(),
        "libx264".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-preset".to_string(),
        params.preset.clone(),
        "-crf".to_string(),
        params.crf.to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        out.to_string_lossy().into_owned(),
    ]
}

/// Format a frame rate for ffmpeg's `-r`: an integer rate prints without a
/// decimal point, a fractional rate keeps up to two decimals (`29.97`).
fn format_rate(fps: f64) -> String {
    if (fps.fract()).abs() < 1e-6 {
        format!("{}", fps.round() as i64)
    } else {
        // Trim to 2 dp, then drop a trailing zero (e.g. 23.90 -> 23.9).
        let s = format!("{fps:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Returns `true` if the configured `ffmpeg` binary can be spawned (a `-version`
/// probe succeeds). Callers gate the actual encode on this so a missing binary
/// surfaces as a clear UI error instead of a failed encode. Never panics.
pub fn ffmpeg_available() -> bool {
    Command::new(ffmpeg_bin())
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Encode a stream of straight-alpha **`rgba`** frames (each exactly
/// `width * height * 4` bytes, top-left origin) to an H.264 MP4 at `out`.
///
/// Frames are produced lazily by `frames` (so the caller can render full-res
/// frames one at a time without holding them all in memory) and piped to
/// `ffmpeg` via stdin using the invocation from [`encode_h264_args`]. Returns
/// the number of frames written.
///
/// Errors:
/// - [`MediaError::BinaryNotFound`] when `ffmpeg` can't be spawned (gate with
///   [`ffmpeg_available`] for a clean UI message first);
/// - [`MediaError::Decode`] for a zero-size geometry, an empty stream, a frame
///   of the wrong byte length, or a non-zero ffmpeg exit (with its stderr).
///
/// This is the suite's shared, app-agnostic H.264 encoder (co-owned with Pulse);
/// the caller owns *what* to render (a Reel program frame, a Pulse comp frame).
pub fn encode_h264<I>(mut frames: I, params: &EncodeParams, out: &Path) -> Result<usize, MediaError>
where
    I: Iterator<Item = Vec<u8>>,
{
    if params.width == 0 || params.height == 0 {
        return Err(MediaError::Decode(format!(
            "zero-size encode geometry ({}x{})",
            params.width, params.height
        )));
    }
    let frame_bytes = (params.width as usize) * (params.height as usize) * 4;

    let bin = ffmpeg_bin();
    let args = encode_h264_args(params, out);
    let mut child = Command::new(&bin)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| spawn_err(&bin, e))?;

    let mut written = 0usize;
    {
        use std::io::Write;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| MediaError::Decode("failed to open ffmpeg stdin".to_string()))?;
        for frame in frames.by_ref() {
            if frame.len() != frame_bytes {
                // Drop stdin (closing the pipe) and reap before reporting, so we
                // don't leave a zombie / blocked ffmpeg.
                drop(stdin);
                let _ = child.wait();
                return Err(MediaError::Decode(format!(
                    "frame {written} has {} bytes, expected {frame_bytes} ({}x{})",
                    frame.len(),
                    params.width,
                    params.height
                )));
            }
            if let Err(e) = stdin.write_all(&frame) {
                // A broken pipe means ffmpeg exited early; fall through to wait()
                // so its stderr explains why.
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    drop(stdin);
                    let _ = child.wait();
                    return Err(MediaError::Io(e));
                }
                break;
            }
            written += 1;
        }
        // Closing stdin (end of block) signals EOF to ffmpeg.
    }

    let output = child.wait_with_output().map_err(MediaError::Io)?;
    if !output.status.success() {
        return Err(MediaError::Decode(format!(
            "ffmpeg encode exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if written == 0 {
        return Err(MediaError::Decode("no frames to encode".to_string()));
    }
    Ok(written)
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

    // --- Encode -------------------------------------------------------------

    #[test]
    fn encode_args_are_the_expected_invocation() {
        let params = EncodeParams { width: 1920, height: 1080, fps: 30.0, crf: 18, preset: "medium".to_string() };
        let args = encode_h264_args(&params, Path::new("/tmp/out.mp4"));
        let expected: Vec<String> = [
            "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", "1920x1080", "-r", "30", "-i", "-",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-preset", "medium", "-crf", "18",
            "-movflags", "+faststart", "/tmp/out.mp4",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(args, expected);
    }

    #[test]
    fn encode_args_carry_geometry_rate_crf_preset() {
        let params = EncodeParams { width: 640, height: 360, fps: 23.976, crf: 23, preset: "veryfast".to_string() };
        let args = encode_h264_args(&params, Path::new("clip.mp4"));
        assert!(args.windows(2).any(|w| w[0] == "-s" && w[1] == "640x360"));
        // 23.976 -> "23.98" trimmed to two-dp.
        assert!(args.windows(2).any(|w| w[0] == "-r" && w[1] == "23.98"), "rate arg {args:?}");
        assert!(args.windows(2).any(|w| w[0] == "-crf" && w[1] == "23"));
        assert!(args.windows(2).any(|w| w[0] == "-preset" && w[1] == "veryfast"));
        assert_eq!(args.last().unwrap(), "clip.mp4");
    }

    #[test]
    fn format_rate_integer_and_ntsc() {
        assert_eq!(format_rate(30.0), "30");
        assert_eq!(format_rate(24.0), "24");
        assert_eq!(format_rate(29.97), "29.97");
        assert_eq!(format_rate(59.94), "59.94");
    }

    #[test]
    fn encode_default_params() {
        let p = EncodeParams::new(1280, 720, 25.0);
        assert_eq!((p.width, p.height), (1280, 720));
        assert_eq!(p.crf, 18);
        assert_eq!(p.preset, "medium");
    }

    #[test]
    fn encode_rejects_zero_geometry() {
        let params = EncodeParams::new(0, 0, 30.0);
        let frames = std::iter::empty::<Vec<u8>>();
        let out = temp_path("zero.mp4");
        assert!(matches!(
            encode_h264(frames, &params, &out),
            Err(MediaError::Decode(_))
        ));
    }

    #[test]
    fn encode_real_roundtrip_then_probe() {
        // Gated like the decode tests: skip silently if ffmpeg is absent.
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available — skipping prism-media encode test");
            return;
        }
        let (w, h, fps) = (32u32, 24u32, 10.0f64);
        let params = EncodeParams::new(w, h, fps);
        let out = temp_path("encode.mp4");
        // 10 solid-magenta frames.
        let frames = (0..10).map(move |_| {
            let mut f = Vec::with_capacity((w * h * 4) as usize);
            for _ in 0..(w * h) {
                f.extend_from_slice(&[255, 0, 255, 255]);
            }
            f
        });
        let n = encode_h264(frames, &params, &out).expect("encode");
        assert_eq!(n, 10);
        let info = probe(&out).expect("probe encoded mp4");
        assert_eq!(info.width, w);
        assert_eq!(info.height, h);
        assert!(info.video_codec.is_some());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn encode_rejects_wrong_frame_size() {
        if !ffmpeg_available() {
            eprintln!("ffmpeg not available — skipping wrong-frame-size encode test");
            return;
        }
        let params = EncodeParams::new(16, 16, 10.0);
        let out = temp_path("badsize.mp4");
        // A frame too short for 16*16*4.
        let frames = std::iter::once(vec![0u8; 10]);
        assert!(matches!(
            encode_h264(frames, &params, &out),
            Err(MediaError::Decode(_))
        ));
        let _ = std::fs::remove_file(&out);
    }
}
