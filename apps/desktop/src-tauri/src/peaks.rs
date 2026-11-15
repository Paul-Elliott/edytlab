//! A file's waveform as a short list of peaks, for the timeline to draw.
//!
//! On a desktop each lane hands its file's address to WaveSurfer, which
//! fetches the whole file and decodes it in the webview: five minutes of
//! stereo at 48 kHz is about 115 MB of samples per track, held for as long as
//! the lane is on screen. A desktop has that to spare. A phone's WebView does
//! not, least of all beside a stem separation that needs 3 GB of its own, and
//! a project of one song and its four stems is five such tracks.
//!
//! So the Android window asks here instead ([`waveform_peaks`]). The file is
//! read in Rust, a chunk at a time where the format allows, and reduced to
//! the loudest sample of each short stretch: at most [`PEAKS_PER_SEC`] values
//! a second and [`MAX_PEAKS`] in all, a few hundred kilobytes for a song.
//! WaveSurfer draws from those and decodes nothing. The desktop window does
//! not call this, and what it draws is unchanged.
//!
//! The answer for a file is kept while the file's size and modification time
//! stay the same, so switching panes or reopening the timeline does not read
//! the audio again.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

/// Peaks kept per second of audio. At the timeline's fitted zoom a pane is a
/// few hundred pixels for the whole file, and zoomed in to the point where
/// one second fills a phone's width, 200 values are still one per two
/// pixels.
pub const PEAKS_PER_SEC: f64 = 200.0;

/// The most peaks one file is given: an hour and a half of audio at 75 a
/// second. Past it, each peak covers more time.
pub const MAX_PEAKS: usize = 400_000;

/// Frames read from a WAV per step: about a third of a second, a megabyte
/// or so of samples at most.
const CHUNK_FRAMES: usize = 16_384;

/// How many files' peaks are kept. A project is rarely more than a dozen
/// tracks.
const CACHE_ENTRIES: usize = 32;

/// What [`waveform_peaks`] answers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peaks {
    /// The length of the audio, in seconds.
    pub duration_sec: f64,
    /// One value per stretch of the file, in order: the loudest sample in
    /// it across every channel, as 0 (silence) to 32767 (full scale). Whole
    /// numbers, because a list of them is a third the size of the same
    /// list of floats on the way to the window.
    pub peaks: Vec<u16>,
}

/// How many peaks a file of `frames` frames at `sample_rate` gets.
pub fn peak_count(frames: u64, sample_rate: u32) -> usize {
    if frames == 0 || sample_rate == 0 {
        return 0;
    }
    let seconds = frames as f64 / f64::from(sample_rate);
    let wanted = (seconds * PEAKS_PER_SEC).ceil() as u64;
    // Never more peaks than frames: a ten-sample file has ten.
    wanted.clamp(1, (MAX_PEAKS as u64).min(frames)) as usize
}

/// Collects peaks from interleaved samples as they are read.
struct PeakBuilder {
    channels: usize,
    total_frames: u64,
    seen_frames: u64,
    peaks: Vec<f32>,
}

impl PeakBuilder {
    fn new(total_frames: u64, channels: u16, sample_rate: u32) -> Self {
        Self {
            channels: usize::from(channels.max(1)),
            total_frames,
            seen_frames: 0,
            peaks: vec![0.0; peak_count(total_frames, sample_rate)],
        }
    }

    /// Add the next samples, interleaved by channel. Frames past the length
    /// the builder was told of are counted into the last peak.
    fn push(&mut self, interleaved: &[f32]) {
        if self.peaks.is_empty() {
            return;
        }
        let buckets = self.peaks.len() as u128;
        let last = self.peaks.len() - 1;
        for frame in interleaved.chunks_exact(self.channels) {
            let loudest = frame.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let bucket = if self.total_frames == 0 {
                last
            } else {
                // 128 bits: frames times buckets overflows 64 for a long file.
                ((u128::from(self.seen_frames) * buckets) / u128::from(self.total_frames)) as usize
            };
            let slot = &mut self.peaks[bucket.min(last)];
            // A NaN in the file is not a level; `max` drops it.
            *slot = slot.max(loudest);
            self.seen_frames += 1;
        }
    }

    fn finish(self, sample_rate: u32) -> Peaks {
        let frames = self.seen_frames.max(self.total_frames);
        Peaks {
            duration_sec: if sample_rate == 0 {
                0.0
            } else {
                frames as f64 / f64::from(sample_rate)
            },
            peaks: self
                .peaks
                .into_iter()
                .map(|p| (p.clamp(0.0, 1.0) * 32767.0).round() as u16)
                .collect(),
        }
    }
}

/// The peaks of already decoded audio.
pub fn peaks_of_samples(interleaved: &[f32], channels: u16, sample_rate: u32) -> Peaks {
    let channel_count = usize::from(channels.max(1));
    let frames = (interleaved.len() / channel_count) as u64;
    let mut builder = PeakBuilder::new(frames, channels, sample_rate);
    builder.push(interleaved);
    builder.finish(sample_rate)
}

/// Read `path` and reduce it to peaks.
///
/// A WAV is read a chunk at a time, so its samples are never all in memory:
/// that is every file the session made (renders, stems, recordings, and the
/// WAV copy it keeps of a compressed source). Anything else is decoded
/// whole, as loading it was.
pub fn peaks_of_file(path: &Path) -> Result<Peaks, String> {
    if let Ok(mut reader) = audio_decoder::WavStreamReader::open(path) {
        let (rate, channels) = (reader.sample_rate(), reader.channels());
        let mut builder = PeakBuilder::new(reader.total_frames(), channels, rate);
        let mut chunk = vec![0.0f32; CHUNK_FRAMES * usize::from(channels.max(1))];
        loop {
            let frames = reader
                .read_frames(&mut chunk)
                .map_err(|e| format!("the audio could not be read: {e}"))?;
            if frames == 0 {
                break;
            }
            builder.push(&chunk[..frames * usize::from(channels.max(1))]);
        }
        return Ok(builder.finish(rate));
    }
    let decoded = audio_decoder::decode_file(path)
        .map_err(|e| format!("the audio could not be read: {e}"))?;
    Ok(peaks_of_samples(
        &decoded.samples,
        decoded.channels,
        decoded.sample_rate,
    ))
}

/// One remembered answer: the file as it was when it was read.
struct Cached {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
    peaks: Arc<Peaks>,
}

static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());

/// [`peaks_of_file`], remembered while the file is unchanged.
pub fn cached_peaks(path: &Path) -> Result<Arc<Peaks>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("the audio could not be read: {e}"))?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    if let Ok(cache) = CACHE.lock() {
        if let Some(hit) = cache
            .iter()
            .find(|c| c.path == path && c.len == len && c.modified == modified)
        {
            return Ok(Arc::clone(&hit.peaks));
        }
    }
    let peaks = Arc::new(peaks_of_file(path)?);
    if let Ok(mut cache) = CACHE.lock() {
        // An older answer for this path is for a file that has changed.
        cache.retain(|c| c.path != path);
        if cache.len() >= CACHE_ENTRIES {
            cache.remove(0);
        }
        cache.push(Cached {
            path: path.to_path_buf(),
            len,
            modified,
            peaks: Arc::clone(&peaks),
        });
    }
    Ok(peaks)
}

/// The waveform of one of the session's audio files, as peaks.
///
/// The window may ask only about a file it could already read: one inside
/// the asset protocol's scope (the open project and the app's own data, see
/// `commands::allow_assets_in_dir`). A path outside it is refused, so this
/// reads nothing the webview could not fetch itself.
///
/// File work, with no session lock taken: it runs on the blocking pool.
#[tauri::command]
pub async fn waveform_peaks<R: Runtime>(app: AppHandle<R>, path: String) -> Result<Peaks, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() || !app.asset_protocol_scope().is_allowed(&path) {
        return Err("that file is not part of the open project".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || cached_peaks(&path).map(|p| (*p).clone()))
        .await
        .map_err(|e| format!("the task did not finish: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_gets_about_two_hundred_peaks_a_second_up_to_the_cap() {
        assert_eq!(peak_count(0, 48_000), 0);
        assert_eq!(peak_count(48_000, 0), 0);
        assert_eq!(peak_count(48_000, 48_000), 200);
        assert_eq!(peak_count(24_000, 48_000), 100);
        // Three hours: capped, so each peak covers more time.
        assert_eq!(peak_count(3 * 3600 * 44_100, 44_100), MAX_PEAKS);
        // Never more peaks than frames.
        assert_eq!(peak_count(10, 48_000), 1);
        assert_eq!(peak_count(1, 8_000), 1);
    }

    #[test]
    fn each_peak_is_the_loudest_sample_of_its_stretch_on_any_channel() {
        // One second of stereo at 1 kHz: 200 peaks of five frames each.
        let rate = 1_000u32;
        let mut samples = vec![0.0f32; 2 * rate as usize];
        samples[2 * 2] = 0.25; // frame 2, left: the first peak
        samples[2 * 7 + 1] = -0.5; // frame 7, right: the second, and negative
        samples[2 * 999] = 1.5; // the last frame, over full scale
        let peaks = peaks_of_samples(&samples, 2, rate);

        assert_eq!(peaks.duration_sec, 1.0);
        assert_eq!(peaks.peaks.len(), 200);
        assert_eq!(peaks.peaks[0], (0.25f32 * 32767.0).round() as u16);
        assert_eq!(peaks.peaks[1], (0.5f32 * 32767.0).round() as u16);
        assert_eq!(peaks.peaks[2], 0);
        // Clipped to full scale, not wrapped.
        assert_eq!(peaks.peaks[199], 32767);
    }

    #[test]
    fn feeding_in_chunks_gives_what_feeding_at_once_does() {
        let rate = 8_000u32;
        let samples: Vec<f32> = (0..rate * 3)
            .map(|i| ((i as f32) * 0.013).sin() * 0.8)
            .collect();
        let whole = peaks_of_samples(&samples, 1, rate);

        let mut builder = PeakBuilder::new(u64::from(rate) * 3, 1, rate);
        for chunk in samples.chunks(777) {
            builder.push(chunk);
        }
        assert_eq!(builder.finish(rate), whole);
        assert_eq!(whole.peaks.len(), 600);
    }

    #[test]
    fn a_wav_on_disk_is_read_and_its_answer_is_reused_until_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        let write = |amplitude: f32| {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 8_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..8_000 {
                let s = ((i as f32) * 0.2).sin() * amplitude;
                writer.write_sample((s * 32767.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        };

        write(0.5);
        let first = cached_peaks(&path).unwrap();
        assert_eq!(first.duration_sec, 1.0);
        assert_eq!(first.peaks.len(), 200);
        let loudest = f32::from(*first.peaks.iter().max().unwrap()) / 32767.0;
        assert!((loudest - 0.5).abs() < 0.01, "{loudest}");
        // The same file again is the same answer, not a second read.
        assert!(Arc::ptr_eq(&first, &cached_peaks(&path).unwrap()));

        // A file that has changed is read again. A different length makes
        // the change visible whatever the clock's resolution.
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..4_000 {
            writer.write_sample(i16::MAX / 4).unwrap();
        }
        writer.finalize().unwrap();
        let second = cached_peaks(&path).unwrap();
        assert_eq!(second.duration_sec, 0.5);
        assert!(!Arc::ptr_eq(&first, &second));

        assert!(cached_peaks(&dir.path().join("missing.wav")).is_err());
    }
}
