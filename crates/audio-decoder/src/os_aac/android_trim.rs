//! Which decoded frames of an M4A the Android backend keeps (#764).
//!
//! An AAC stream starts with the encoder's priming (1,024 frames or more of
//! lead-in) and ends with whatever fills its last packet. macOS drops both
//! itself and `media_foundation.rs` works them out from timestamps; here they
//! come from what Android's MP4 reader says about the track, which is not the
//! same on every Android. This is arithmetic only, kept apart from `android.rs`
//! so that it is tested on every platform: the backend itself only compiles
//! for Android.
//!
//! What the reader says, read from AOSP's `MPEG4Extractor.cpp` at each release
//! tag (the reader is also part of the Media mainline module, so a phone can
//! have a newer one than its Android version):
//!
//! - `encoder-delay` and `encoder-padding`, in frames at the track's declared
//!   rate. They come from an `iTunSMPB` tag or from a one-entry edit list.
//!   Android 8 reads the edit list before it knows the rate, so for most files
//!   the keys are missing. Android 9 divides by the wrong timescale, so the
//!   delay is far too large unless the movie and the track share a timescale
//!   (1,024 becomes 45,158 for a 44.1 kHz file written by ffmpeg). Android 10
//!   and later have it right.
//! - `durationUs`. Up to Android 15 it is the media duration (`mdhd`), counted
//!   from the first decoded frame, priming included. From Android 16 it is the
//!   presentation duration (`tkhd`), counted from after the priming.
//!
//! The padding key alone is not enough. A muxer that shortens the last packet's
//! duration (ffmpeg does) leaves `encoder-padding` at zero, and the length is
//! then only in the duration. So the keys are used when they are plausible, and
//! the duration then caps the end, read as whichever of its two meanings fits
//! the frames that were decoded. Nothing is cut that the file does not say is
//! outside the audio: where a value is missing or does not fit, the frames
//! stay. None of this has run on a device yet; the cases in the tests are the
//! fixtures' own numbers put through the rules above.

use std::ops::Range;

/// Most frames of priming, or of padding after the end, taken off because a
/// key says so. As in `media_foundation.rs`: an AAC packet is 1,024 frames
/// (2,048 for HE-AAC), so a larger value is not what this assumes.
const MAX_TRIM_FRAMES: u64 = 8_192;

/// One decoded track, counted in frames at the rate the decoder gave.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Decoded {
    /// Frames the decoder returned.
    pub frames: usize,
    /// Compressed packets handed to the decoder.
    pub packets: usize,
    /// `encoder-delay`, if the track has the key.
    pub delay: Option<u64>,
    /// `encoder-padding`, if the track has the key.
    pub padding: Option<u64>,
    /// `durationUs`, if the track has the key.
    pub duration: Option<u64>,
}

/// A count of frames at the track's declared rate, at the decoder's rate. The
/// two differ for HE-AAC, whose track declares the core rate and decodes to
/// twice that. The codec framework scales the two keys the same way when it
/// trims by itself (`ACodec::sendFormatChange`).
pub(crate) fn at_output_rate(frames: i32, track_rate: i32, output_rate: u32) -> Option<u64> {
    let frames = u64::try_from(frames).ok()?;
    match u64::try_from(track_rate) {
        Ok(track_rate) if track_rate > 0 => {
            Some(frames.saturating_mul(u64::from(output_rate)) / track_rate)
        }
        _ => Some(frames),
    }
}

/// Microseconds as frames at `rate`, to the nearest frame. The reader keeps
/// whole microseconds, which is under one frame at any rate below 500 kHz, so
/// rounding gives back the frame count the file states.
pub(crate) fn micros_to_frames(micros: i64, rate: u32) -> Option<u64> {
    let micros = u64::try_from(micros).ok().filter(|&us| us > 0)?;
    let frames = (u128::from(micros) * u128::from(rate) + 500_000) / 1_000_000;
    u64::try_from(frames).ok()
}

/// The frames to keep, as a range of frame indices.
pub(crate) fn keep(decoded: &Decoded) -> Range<usize> {
    let total = decoded.frames;
    let plausible = |frames: Option<u64>| {
        frames
            .filter(|&f| f < MAX_TRIM_FRAMES)
            .and_then(|f| usize::try_from(f).ok())
            .unwrap_or(0)
    };
    let mut lead = plausible(decoded.delay);
    let mut tail = plausible(decoded.padding);
    if lead.saturating_add(tail) >= total {
        // The keys would leave nothing: they are not about this audio.
        lead = 0;
        tail = 0;
    }
    let start = lead;
    let mut end = total - tail;

    let duration = decoded.duration.and_then(|d| usize::try_from(d).ok());
    // Frames per packet. A sound file's duration ends inside its last packet,
    // so a duration is believed only when it is within one packet of the
    // frames it is compared with.
    let per_packet = total.checked_div(decoded.packets).unwrap_or(0);
    if let Some(duration) = duration.filter(|_| per_packet > 0) {
        // A media duration (priming included) ends inside the last packet. A
        // presentation duration is shorter than that by the priming, which is
        // a packet or more, so it ends before the last packet starts. A lead
        // shorter than a packet could be either, and is read as the second,
        // which never cuts more than the first would.
        let counts_priming = duration.saturating_add(per_packet) > total;
        if counts_priming && (lead == 0 || lead >= per_packet) {
            if duration > start {
                end = end.min(duration);
            }
        } else {
            let kept = end - start;
            if kept > duration && kept - duration < per_packet {
                end = start + duration;
            }
        }
    }
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    // `tone_440_mono_44100.m4a`: 23 packets of 1,024 frames, an edit list of
    // (500 ms, media time 1,024), `mdhd` duration 23,074 at 44,100 (523,219 us
    // as the reader keeps it), `tkhd` duration 500 ms. 22,050 frames of audio.
    const MONO: Decoded = Decoded {
        frames: 23 * 1_024,
        packets: 23,
        delay: None,
        padding: None,
        duration: None,
    };
    // `tones_440_880_stereo_48000.m4a`: 25 packets, `mdhd` 25,024 at 48,000
    // (521,333 us), `tkhd` 500 ms. 24,000 frames of audio.
    const STEREO: Decoded = Decoded {
        frames: 25 * 1_024,
        packets: 25,
        delay: None,
        padding: None,
        duration: None,
    };

    fn frames(range: Range<usize>) -> usize {
        range.end - range.start
    }

    #[test]
    fn android_10_to_15_gives_the_edit_lists_frames() {
        // encoder-delay 1,024, encoder-padding 0 (the last packet's duration
        // is already short in `mdhd`), durationUs from `mdhd`.
        let mono = Decoded {
            delay: Some(1_024),
            padding: Some(0),
            duration: micros_to_frames(523_219, 44_100),
            ..MONO
        };
        assert_eq!(mono.duration, Some(23_074));
        assert_eq!(keep(&mono), 1_024..23_074);
        assert_eq!(frames(keep(&mono)), 22_050);

        let stereo = Decoded {
            delay: Some(1_024),
            padding: Some(0),
            duration: micros_to_frames(521_333, 48_000),
            ..STEREO
        };
        assert_eq!(stereo.duration, Some(25_024));
        assert_eq!(frames(keep(&stereo)), 24_000);
    }

    #[test]
    fn android_16_gives_the_same_frames_from_the_presentation_duration() {
        let mono = Decoded {
            delay: Some(1_024),
            padding: Some(0),
            duration: micros_to_frames(500_000, 44_100),
            ..MONO
        };
        assert_eq!(keep(&mono), 1_024..23_074);

        let stereo = Decoded {
            delay: Some(1_024),
            padding: Some(0),
            duration: micros_to_frames(500_000, 48_000),
            ..STEREO
        };
        assert_eq!(keep(&stereo), 1_024..25_024);
    }

    #[test]
    fn android_8_keeps_the_priming_and_cuts_the_end() {
        // No keys at all: the duration still says where the audio ends.
        let mono = Decoded {
            duration: micros_to_frames(523_219, 44_100),
            ..MONO
        };
        assert_eq!(keep(&mono), 0..23_074);
    }

    #[test]
    fn android_9s_wrong_delay_is_not_believed() {
        // 1,024 * 44,100 / 1,000 (the movie's timescale, not the track's).
        let mono = Decoded {
            delay: Some(45_158),
            padding: Some(0),
            duration: micros_to_frames(523_219, 44_100),
            ..MONO
        };
        assert_eq!(keep(&mono), 0..23_074);
        // At 8 kHz the same mistake gives exactly the bound, which is refused.
        let low = Decoded {
            frames: 10 * 1_024,
            packets: 10,
            delay: Some(8_192),
            padding: Some(0),
            duration: None,
        };
        assert_eq!(keep(&low), 0..10_240);
    }

    #[test]
    fn an_itunes_tag_trims_both_ends() {
        // iTunSMPB: 2,112 frames of priming and 500 of padding, with a media
        // duration that counts every packet.
        let tagged = Decoded {
            frames: 100 * 1_024,
            packets: 100,
            delay: Some(2_112),
            padding: Some(500),
            duration: Some(100 * 1_024),
        };
        assert_eq!(keep(&tagged), 2_112..101_900);
        // The same file whose duration is the presentation's.
        let presented = Decoded {
            duration: Some(100 * 1_024 - 2_612),
            ..tagged
        };
        assert_eq!(keep(&presented), 2_112..101_900);
    }

    #[test]
    fn a_file_that_says_nothing_keeps_every_frame() {
        assert_eq!(keep(&MONO), 0..23_552);
        // A duration that counts every packet cuts nothing either.
        let full = Decoded {
            duration: Some(23_552),
            ..MONO
        };
        assert_eq!(keep(&full), 0..23_552);
        // Nor does one longer than what was decoded.
        let long = Decoded {
            duration: Some(30_000),
            ..MONO
        };
        assert_eq!(keep(&long), 0..23_552);
    }

    #[test]
    fn a_duration_far_from_the_decoded_length_is_not_believed() {
        // More than a packet short of the frames: a header that was never
        // finished, not a last packet's remainder.
        let short = Decoded {
            duration: Some(10_000),
            ..MONO
        };
        assert_eq!(keep(&short), 0..23_552);
        let short_with_delay = Decoded {
            delay: Some(1_024),
            duration: Some(10_000),
            ..MONO
        };
        assert_eq!(keep(&short_with_delay), 1_024..23_552);
    }

    #[test]
    fn a_lead_shorter_than_a_packet_never_costs_audio() {
        // An edit that starts 512 frames in. The duration could count from
        // either place, so it is read as the one that keeps more.
        let odd = Decoded {
            delay: Some(512),
            duration: Some(23_300),
            ..MONO
        };
        assert_eq!(keep(&odd), 512..23_552);
    }

    #[test]
    fn keys_that_would_leave_nothing_are_dropped() {
        let tiny = Decoded {
            frames: 1_024,
            packets: 1,
            delay: Some(1_024),
            padding: Some(0),
            duration: None,
        };
        assert_eq!(keep(&tiny), 0..1_024);
        let empty = Decoded::default();
        assert_eq!(keep(&empty), 0..0);
    }

    #[test]
    fn he_aac_keys_are_scaled_to_the_decoded_rate() {
        // The track declares the 22,050 Hz core and decodes to 44,100.
        assert_eq!(at_output_rate(1_024, 22_050, 44_100), Some(2_048));
        assert_eq!(at_output_rate(1_024, 44_100, 44_100), Some(1_024));
        // No declared rate: the value is taken as it is.
        assert_eq!(at_output_rate(1_024, 0, 44_100), Some(1_024));
        // A negative count is not a count.
        assert_eq!(at_output_rate(-1, 44_100, 44_100), None);
    }

    #[test]
    fn microseconds_round_to_the_frame_the_file_states() {
        assert_eq!(micros_to_frames(523_219, 44_100), Some(23_074));
        assert_eq!(micros_to_frames(534_058, 44_100), Some(23_552));
        assert_eq!(micros_to_frames(0, 44_100), None);
        assert_eq!(micros_to_frames(-1, 44_100), None);
    }
}
