//! M4A/AAC on Android through the NDK's media library (#764).
//!
//! `AMediaExtractor` opens the file with Android's own MP4 reader and
//! `AMediaCodec` decodes its first audio track with whatever decoder the
//! device has for it (AAC-LC and HE-AAC on every Android; others by device),
//! so the decoder is the device's and the AAC patent licence is its maker's.
//! Both are plain C calls into `libmediandk`: no Java, no JNI and no activity
//! are needed, and a decode runs on whichever thread asks for it. Headers:
//! `media/NdkMediaExtractor.h`, `NdkMediaCodec.h` and `NdkMediaFormat.h`;
//! <https://developer.android.com/ndk/reference/group/media>.
//!
//! **Only API 21 symbols are linked.** The app's minSdk is 26, and a library
//! that names a newer symbol does not load on an older Android. So the format
//! keys the NDK only exports as symbols later (`AMEDIAFORMAT_KEY_PCM_ENCODING`
//! in 28, `AMEDIAFORMAT_KEY_ENCODER_DELAY` and `_PADDING` in 29) are written
//! out as the strings they hold, which are the same on every Android because
//! they are `android.media.MediaFormat`'s constants.
//!
//! **The file is opened here and handed over as a descriptor**, not as a path:
//! the reader runs behind a system service on some Android versions, and a
//! descriptor of a file in the app's own storage is what it can always read.
//!
//! **Output is whatever PCM the decoder gives**, converted to `f32`. Android's
//! AAC decoders give 16-bit unless asked otherwise, and this does not ask, so
//! an import has 16-bit resolution; float output is read if a device gives it.
//! The rate and channel count are the decoder's (HE-AAC decodes to twice the
//! rate its track declares, and parametric stereo to two channels from one),
//! never the track's.
//!
//! **Priming and padding are trimmed here, not by the codec.** Given a format
//! with `encoder-delay` and `encoder-padding`, the codec framework drops those
//! frames by itself (`ACodec` and `CCodecBufferChannel` both run the output
//! through a `SkipCutBuffer` when either is not zero). That would also act on
//! the wrong values Android 9 reports, and it cannot see the duration, which
//! is where most files keep their length. So both keys are set to zero in the
//! format the codec is configured with, which turns that off, every frame
//! comes back, and `android_trim` decides what to keep. Whether a device's
//! codec really returns every packet's frames once the keys are zero is read
//! from AOSP's source, not measured: this module has been compiled and has not
//! run on a device.
//!
//! The `ndk` crate's wrappers panic where the NDK breaks its own contract (no
//! output format, no buffer for an index it just gave, a MIME type that is not
//! UTF-8), so the decode runs behind `catch_unwind` like the Opus decoder
//! (#908; `opus.rs` refuses to build this crate with `panic = "abort"`). A
//! panic unwinds through the handles below, which close as they drop.

use std::fs::File;
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;
use ndk::media_error::MediaError;
use ndk_sys as ffi;

use super::android_trim::{self, Decoded};
use crate::{DecodeError, DecodedAudio, Result};

/// Said when the device has no decoder for the audio in this particular file
/// (`AMediaCodec_createDecoderByType` gives nothing for the track's type):
/// ALAC, AC-3 on most phones, and the like.
const NO_DECODER_FOR_FILE: &str = "Android has no decoder for the audio in this file. \
Convert the file to WAV, FLAC, MP3, Ogg Vorbis, Opus or AIFF and load that";

/// Said when the device's decoder exists and was not to be had
/// (`AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE`, `AMEDIACODEC_ERROR_RECLAIMED`):
/// a phone runs only so many codecs at once, across every app, and takes one
/// back when it runs short. The second code is also what a codec service that
/// died gives, so the text does not say which it was.
const DECODER_BUSY: &str = "Android's audio decoder was not available to decode this file. \
Close other apps that are playing or recording sound, and load the file again";

/// Most channels taken, as on macOS and Windows: a file over the bound is
/// refused instead of read as noise.
const MAX_CHANNELS: u32 = 8;

/// Tracks looked at for the first audio one. A real file has a handful.
const MAX_TRACKS: usize = 64;

/// `AMEDIAFORMAT_KEY_MIME`, `_SAMPLE_RATE`, `_CHANNEL_COUNT` and `_DURATION`
/// (API 21 symbols; written out like the three below so that every key is
/// read the same way).
const KEY_MIME: &str = "mime";
const KEY_SAMPLE_RATE: &str = "sample-rate";
const KEY_CHANNEL_COUNT: &str = "channel-count";
const KEY_DURATION: &str = "durationUs";
/// `AMEDIAFORMAT_KEY_PCM_ENCODING` (a symbol only from API 28).
const KEY_PCM_ENCODING: &str = "pcm-encoding";
/// `AMEDIAFORMAT_KEY_ENCODER_DELAY` and `_PADDING` (symbols only from API 29).
const KEY_ENCODER_DELAY: &str = "encoder-delay";
const KEY_ENCODER_PADDING: &str = "encoder-padding";

/// `android.media.AudioFormat.ENCODING_PCM_16BIT` and `ENCODING_PCM_FLOAT`,
/// the values of `pcm-encoding`. A format without the key is 16-bit.
const ENCODING_PCM_16BIT: i32 = 2;
const ENCODING_PCM_FLOAT: i32 = 4;

/// `AMEDIACODEC_BUFFER_FLAG_CODEC_CONFIG` and `_END_OF_STREAM`, and
/// `AMEDIAEXTRACTOR_SAMPLE_FLAG_ENCRYPTED`. The bindings type these as signed
/// on some ABIs and unsigned on others; the flag words they are tested against
/// are `u32` on all of them.
const BUFFER_FLAG_CODEC_CONFIG: u32 = 2;
const BUFFER_FLAG_END_OF_STREAM: u32 = 4;
const SAMPLE_FLAG_ENCRYPTED: u32 = 2;

/// How long one wait for the codec lasts. The loop below always waits in one
/// of its two calls unless it has just moved data, so it never spins.
const WAIT: Duration = Duration::from_millis(10);

/// How long the codec may go without taking or giving anything before it is
/// called stuck.
const STALL: Duration = Duration::from_secs(10);

/// The first allocation is at most this many samples (64 MB), whatever length
/// the file claims; the vector grows as it is filled.
const MAX_RESERVED_SAMPLES: usize = 16 * 1024 * 1024;

/// What the person is told for a failure while the file is opened and the
/// decoder set up: a code that says nothing else means the decoder would not
/// take this file's audio.
fn setup_error(e: MediaError) -> DecodeError {
    map_media_error(e).unwrap_or(DecodeError::UnsupportedCodec)
}

/// What the person is told for a failure once decoding has started: a code
/// that says nothing else means the data could not be decoded.
fn stream_error(e: MediaError) -> DecodeError {
    map_media_error(e).unwrap_or(DecodeError::Corrupt)
}

/// The codes that mean the same thing wherever they come up.
fn map_media_error(e: MediaError) -> Option<DecodeError> {
    match e {
        MediaError::ErrorUnsupported => Some(DecodeError::UnsupportedCodec),
        MediaError::ErrorMalformed => Some(DecodeError::Corrupt),
        MediaError::CodecErrorInsufficientResource | MediaError::CodecErrorReclaimed => {
            Some(DecodeError::OsDecoderUnavailable(DECODER_BUSY.to_string()))
        }
        _ => None,
    }
}

/// An `AMediaExtractor`, deleted when dropped.
struct Extractor(NonNull<ffi::AMediaExtractor>);

impl Drop for Extractor {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `AMediaExtractor_new` and is deleted
        // exactly once, here. The status is always `AMEDIA_OK`.
        let _ = unsafe { ffi::AMediaExtractor_delete(self.0.as_ptr()) };
    }
}

impl Extractor {
    /// An extractor reading `length` bytes of `file` from its start.
    fn open(file: &File, length: u64) -> Result<Self> {
        // SAFETY: no arguments; the result is checked for NULL.
        let raw = NonNull::new(unsafe { ffi::AMediaExtractor_new() }).ok_or_else(|| {
            DecodeError::Io(std::io::Error::other(
                "Android's media reader could not be started",
            ))
        })?;
        let extractor = Self(raw);
        let length = i64::try_from(length).map_err(|_| DecodeError::Corrupt)?;
        // SAFETY: the extractor is live and `file` is open for the call. The
        // extractor reads through a duplicate of the descriptor, and `file`
        // outlives the extractor in `decode` anyway.
        let status = unsafe {
            ffi::AMediaExtractor_setDataSourceFd(extractor.0.as_ptr(), file.as_raw_fd(), 0, length)
        };
        if status.0 != 0 {
            // The reader found no media it knows in the bytes. An I/O code
            // here is a file cut short, not a disk fault: the file opened.
            return Err(match MediaError::from(status.0) {
                MediaError::ErrorUnsupported => DecodeError::UnsupportedCodec,
                _ => DecodeError::Corrupt,
            });
        }
        Ok(extractor)
    }

    fn track_count(&self) -> usize {
        // SAFETY: a call on a live extractor with a data source set.
        unsafe { ffi::AMediaExtractor_getTrackCount(self.0.as_ptr()) }
    }

    /// The format of track `index`, which must be under [`Self::track_count`].
    fn track_format(&self, index: usize) -> Option<MediaFormat> {
        // SAFETY: a call on a live extractor with an index in range.
        let raw = unsafe { ffi::AMediaExtractor_getTrackFormat(self.0.as_ptr(), index) };
        // SAFETY: the header says the caller owns the returned format and
        // frees it, which `MediaFormat` does when dropped.
        NonNull::new(raw).map(|raw| unsafe { MediaFormat::from_ptr(raw) })
    }

    fn select_track(&self, index: usize) -> Result<()> {
        // SAFETY: a call on a live extractor with an index in range.
        let status = unsafe { ffi::AMediaExtractor_selectTrack(self.0.as_ptr(), index) };
        if status.0 != 0 {
            return Err(setup_error(MediaError::from(status.0)));
        }
        Ok(())
    }

    /// Copy the current packet into `into` and return its length, or `None`
    /// when there is none (the end of the track) or it does not fit.
    fn read_packet(&self, into: &mut [MaybeUninit<u8>]) -> Option<usize> {
        // SAFETY: the extractor is live, and `into` is `into.len()` writable
        // bytes for the call; the extractor only writes to them.
        let read = unsafe {
            ffi::AMediaExtractor_readSampleData(
                self.0.as_ptr(),
                into.as_mut_ptr().cast::<u8>(),
                into.len(),
            )
        };
        usize::try_from(read).ok().filter(|&n| n <= into.len())
    }

    /// Whether there is a current packet (false at the end of the track).
    fn has_packet(&self) -> bool {
        // SAFETY: a call on a live extractor.
        unsafe { ffi::AMediaExtractor_getSampleTrackIndex(self.0.as_ptr()) >= 0 }
    }

    /// The current packet's time in microseconds, never negative. A reader
    /// that applies the edit list (Android 16's does) gives the priming
    /// packets times before zero. The decoder needs no more of the times than
    /// their order, and what a codec does with a negative one is not
    /// something to depend on, so those are passed as zero.
    fn packet_time(&self) -> u64 {
        // SAFETY: a call on a live extractor.
        let time = unsafe { ffi::AMediaExtractor_getSampleTime(self.0.as_ptr()) };
        u64::try_from(time).unwrap_or(0)
    }

    /// Whether the current packet is encrypted (a protected purchase).
    fn packet_is_encrypted(&self) -> bool {
        // SAFETY: a call on a live extractor.
        let flags = unsafe { ffi::AMediaExtractor_getSampleFlags(self.0.as_ptr()) };
        // All ones is the call's own "no packet" answer, not a set of flags.
        flags != u32::MAX && flags & SAMPLE_FLAG_ENCRYPTED != 0
    }

    /// Move to the next packet. False when there is none.
    fn advance(&self) -> bool {
        // SAFETY: a call on a live extractor.
        unsafe { ffi::AMediaExtractor_advance(self.0.as_ptr()) }
    }
}

/// The first track whose type is `audio/...`: its index, format and type.
fn first_audio_track(extractor: &Extractor) -> Result<(usize, MediaFormat, String)> {
    for index in 0..extractor.track_count().min(MAX_TRACKS) {
        let Some(mut format) = extractor.track_format(index) else {
            continue;
        };
        let Some(mime) = format.str(KEY_MIME).map(str::to_owned) else {
            continue;
        };
        if mime.starts_with("audio/") {
            return Ok((index, format, mime));
        }
    }
    Err(DecodeError::NoAudioTrack)
}

/// The PCM the decoder is giving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pcm {
    rate: u32,
    channels: u32,
    float: bool,
}

/// The codec's current output format, checked.
fn output_pcm(codec: &MediaCodec) -> Result<Pcm> {
    let format = codec.output_format();
    let rate = format.i32(KEY_SAMPLE_RATE).ok_or(DecodeError::Corrupt)?;
    let channels = format.i32(KEY_CHANNEL_COUNT).ok_or(DecodeError::Corrupt)?;
    let float = match format.i32(KEY_PCM_ENCODING).unwrap_or(ENCODING_PCM_16BIT) {
        ENCODING_PCM_16BIT => false,
        ENCODING_PCM_FLOAT => true,
        // 8-bit, 24-bit and 32-bit integer PCM: no AAC decoder gives them.
        _ => return Err(DecodeError::UnsupportedCodec),
    };
    let rate = u32::try_from(rate).map_err(|_| DecodeError::Corrupt)?;
    let channels = u32::try_from(channels).map_err(|_| DecodeError::Corrupt)?;
    if !(1..=768_000).contains(&rate) || channels == 0 {
        return Err(DecodeError::Corrupt);
    }
    if channels > MAX_CHANNELS {
        // A real stream the app does not take, not a damaged one.
        return Err(DecodeError::UnsupportedCodec);
    }
    Ok(Pcm {
        rate,
        channels,
        float,
    })
}

/// Append one output buffer's PCM to `out` as `f32`. The bytes are read as
/// bytes: the buffer's alignment is the codec's, not ours. Android is
/// little-endian on every ABI, and the PCM is in the device's order.
fn append_pcm(bytes: &[u8], float: bool, out: &mut Vec<f32>) {
    if float {
        out.extend(
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
        );
    } else {
        // Divided by 32,768, as symphonia turns 16-bit samples into `f32`.
        out.extend(
            bytes
                .chunks_exact(2)
                .map(|b| f32::from(i16::from_ne_bytes([b[0], b[1]])) / 32_768.0),
        );
    }
}

/// What came out of the codec.
struct Pumped {
    samples: Vec<f32>,
    pcm: Pcm,
    /// Packets handed to the codec.
    packets: usize,
}

/// Feed every packet of the selected track to the started `codec` and collect
/// what it decodes. `expected_micros` is the track's stated length, used only
/// to size the first allocation.
fn pump(extractor: &Extractor, codec: &MediaCodec, expected_micros: Option<i64>) -> Result<Pumped> {
    let mut samples: Vec<f32> = Vec::new();
    let mut pcm: Option<Pcm> = None;
    let mut packets = 0_usize;
    // Whether the extractor still has a packet to give.
    let mut more_packets = true;
    // Whether the end of the stream has been queued on the codec's input.
    let mut input_ended = false;
    let mut last_progress = Instant::now();

    loop {
        let mut progressed = false;

        if !input_ended {
            match codec.dequeue_input_buffer(WAIT).map_err(stream_error)? {
                DequeuedInputBufferResult::Buffer(mut buffer) => {
                    let read = if more_packets {
                        extractor.read_packet(buffer.buffer_mut())
                    } else {
                        None
                    };
                    match read {
                        Some(size) => {
                            let time = extractor.packet_time();
                            codec
                                .queue_input_buffer(buffer, 0, size, time, 0)
                                .map_err(stream_error)?;
                            packets += 1;
                            // A failed advance ends the input: reading again
                            // would hand over the same packet for ever.
                            more_packets = extractor.advance();
                        }
                        None => {
                            if more_packets && extractor.has_packet() {
                                // A packet is there and could not be read into
                                // the codec's buffer. Ending here would pass a
                                // cut-off file as a whole one.
                                return Err(DecodeError::Corrupt);
                            }
                            codec
                                .queue_input_buffer(buffer, 0, 0, 0, BUFFER_FLAG_END_OF_STREAM)
                                .map_err(stream_error)?;
                            input_ended = true;
                        }
                    }
                    progressed = true;
                }
                DequeuedInputBufferResult::TryAgainLater => {}
            }
        }

        // While packets are still going in, output is only collected if it is
        // ready (the wait was the input's); after that, it is waited for.
        let wait = if input_ended { WAIT } else { Duration::ZERO };
        match codec.dequeue_output_buffer(wait).map_err(stream_error)? {
            DequeuedOutputBufferInfoResult::Buffer(buffer) => {
                let (offset, size, flags) = {
                    let info = buffer.info();
                    (info.offset(), info.size(), info.flags())
                };
                if size > 0 && flags & BUFFER_FLAG_CODEC_CONFIG == 0 {
                    // The format normally arrives first, as a change. A codec
                    // that sends audio without one is asked.
                    let format = match pcm {
                        Some(format) => format,
                        None => {
                            let format = output_pcm(codec)?;
                            reserve(&mut samples, format, expected_micros);
                            pcm = Some(format);
                            format
                        }
                    };
                    // The valid bytes are `size` from `offset`, as the header
                    // says. With the framework's own trim turned off (the
                    // module docs) the offset is zero on AOSP's decoders.
                    let offset = usize::try_from(offset).map_err(|_| DecodeError::Corrupt)?;
                    let size = usize::try_from(size).map_err(|_| DecodeError::Corrupt)?;
                    let end = offset.checked_add(size).ok_or(DecodeError::Corrupt)?;
                    let bytes = buffer
                        .buffer()
                        .get(offset..end)
                        .ok_or(DecodeError::Corrupt)?;
                    append_pcm(bytes, format.float, &mut samples);
                }
                codec
                    .release_output_buffer(buffer, false)
                    .map_err(stream_error)?;
                progressed = true;
                if flags & BUFFER_FLAG_END_OF_STREAM != 0 {
                    break;
                }
            }
            DequeuedOutputBufferInfoResult::OutputFormatChanged => {
                // The decoder's real format (HE-AAC announces its doubled rate
                // this way). Before any audio that is just the format; after
                // some, the samples would not be one stream.
                let now = output_pcm(codec)?;
                if pcm.is_some_and(|was| was != now) && !samples.is_empty() {
                    return Err(DecodeError::Corrupt);
                }
                if pcm.is_none() {
                    reserve(&mut samples, now, expected_micros);
                }
                pcm = Some(now);
                progressed = true;
            }
            // The buffers were replaced. They are asked for by index each
            // time, so there is nothing to refresh.
            DequeuedOutputBufferInfoResult::OutputBuffersChanged => progressed = true,
            DequeuedOutputBufferInfoResult::TryAgainLater => {}
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > STALL {
            // Neither taking packets nor giving audio nor ending.
            return Err(DecodeError::Corrupt);
        }
    }

    // No format means no audio ever came out.
    let pcm = pcm.ok_or(DecodeError::Corrupt)?;
    Ok(Pumped {
        samples,
        pcm,
        packets,
    })
}

/// Reserve room for a track of `micros` at `pcm`'s rate. Only a hint: a
/// header can claim any length, so it is capped and a failure is ignored.
fn reserve(samples: &mut Vec<f32>, pcm: Pcm, micros: Option<i64>) {
    let frames = micros
        .and_then(|us| android_trim::micros_to_frames(us, pcm.rate))
        .and_then(|frames| usize::try_from(frames).ok())
        .unwrap_or(0);
    let wanted = frames.saturating_mul(pcm.channels as usize);
    let _ = samples.try_reserve(wanted.min(MAX_RESERVED_SAMPLES));
}

/// Decode the M4A/MP4 file at `path` with Android's own reader and decoder.
pub(crate) fn decode_path(path: &Path) -> Result<DecodedAudio> {
    // A panic in the wrappers is a decoder that broke its contract on this
    // file, which is what "corrupt or unrecognised" says.
    catch_unwind(AssertUnwindSafe(|| decode(path))).unwrap_or(Err(DecodeError::Corrupt))
}

fn decode(path: &Path) -> Result<DecodedAudio> {
    // Declared first, so dropped last: after the extractor that reads it.
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let extractor = Extractor::open(&file, length)?;

    // Only the first audio track is read (a video file's picture is not).
    let (index, mut track, mime) = first_audio_track(&extractor)?;
    extractor.select_track(index)?;
    if extractor.packet_is_encrypted() {
        // A protected file: without its key the decoder would give noise.
        return Err(DecodeError::UnsupportedCodec);
    }

    // What the container says about priming, padding and length, kept for
    // `android_trim`, then the two keys set to zero in the format the codec
    // gets so that the framework does not trim as well (see the module docs).
    let track_rate = track.i32(KEY_SAMPLE_RATE).unwrap_or(0);
    let delay = track.i32(KEY_ENCODER_DELAY);
    let padding = track.i32(KEY_ENCODER_PADDING);
    let duration_micros = track.i64(KEY_DURATION);
    if delay.is_some() {
        track.set_i32(KEY_ENCODER_DELAY, 0);
    }
    if padding.is_some() {
        track.set_i32(KEY_ENCODER_PADDING, 0);
    }

    let codec = MediaCodec::from_decoder_type(&mime)
        .ok_or_else(|| DecodeError::OsDecoderUnavailable(NO_DECODER_FOR_FILE.to_string()))?;
    codec
        .configure(&track, None, MediaCodecDirection::Decoder)
        .map_err(setup_error)?;
    codec.start().map_err(setup_error)?;

    let Pumped {
        mut samples,
        pcm,
        packets,
    } = pump(&extractor, &codec, duration_micros)?;
    // Deleting the codec releases it whatever state it is in; stopping first
    // is the orderly way, and its answer changes nothing.
    let _ = codec.stop();

    // Whole frames only.
    let channels = pcm.channels as usize;
    samples.truncate(samples.len() - samples.len() % channels);

    let keep = android_trim::keep(&Decoded {
        frames: samples.len() / channels,
        packets,
        delay: delay.and_then(|d| android_trim::at_output_rate(d, track_rate, pcm.rate)),
        padding: padding.and_then(|p| android_trim::at_output_rate(p, track_rate, pcm.rate)),
        duration: duration_micros.and_then(|us| android_trim::micros_to_frames(us, pcm.rate)),
    });
    samples.truncate(keep.end * channels);
    samples.drain(..keep.start * channels);

    Ok(DecodedAudio {
        samples,
        sample_rate: pcm.rate,
        channels: pcm.channels as u16,
    })
}
