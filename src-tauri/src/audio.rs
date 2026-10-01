//! Native backends normalize before constructing these frames. Never expose them over IPC.

pub const SAMPLE_RATE: u32 = 16_000;
pub const CHANNELS: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureClock {
    MacHostTime,
    WindowsQpc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMetadata {
    pub source: Source,
    pub original_format: SourceFormat,
    /// Native timestamp converted to nanoseconds; its epoch is identified by capture_clock.
    pub capture_timestamp_ns: u64,
    pub capture_clock: CaptureClock,
    /// First sample relative to the shared origin; buffered pre-origin samples may be negative.
    pub timeline_timestamp_ns: i64,
    /// Monotonically increasing per source; a gap signals missing frames.
    pub sequence: u64,
    pub discontinuity: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    InvalidSourceFormat,
    EmptyPayload,
    IncompleteSample,
}

/// Always mono 16 kHz signed PCM16 LE, independently of host endianness.
/// No serde implementation: audio stays in the native process.
#[derive(Debug)]
pub struct NormalizedFrame {
    metadata: FrameMetadata,
    pcm_le: Vec<u8>,
}

impl NormalizedFrame {
    pub fn new(metadata: FrameMetadata, pcm_le: Vec<u8>) -> Result<Self, FrameError> {
        if metadata.original_format.sample_rate == 0 || metadata.original_format.channels == 0 {
            return Err(FrameError::InvalidSourceFormat);
        }
        if pcm_le.is_empty() {
            return Err(FrameError::EmptyPayload);
        }
        if pcm_le.len() % 2 != 0 {
            return Err(FrameError::IncompleteSample);
        }
        Ok(Self { metadata, pcm_le })
    }

    pub fn metadata(&self) -> &FrameMetadata {
        &self.metadata
    }

    pub fn pcm_le(&self) -> &[u8] {
        &self.pcm_le
    }

    pub fn sample_count(&self) -> usize {
        self.pcm_le.len() / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_contract_preserves_source_and_rejects_malformed_frames() {
        let metadata = FrameMetadata {
            source: Source::Remote,
            original_format: SourceFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            capture_timestamp_ns: 9_000_000,
            capture_clock: CaptureClock::MacHostTime,
            timeline_timestamp_ns: 1_000_000,
            sequence: 7,
            discontinuity: true,
        };
        let pcm = [-32768i16, -1, 0, 32767]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        let frame = NormalizedFrame::new(metadata, pcm).unwrap();
        assert_eq!(frame.metadata(), &metadata);
        assert_eq!(frame.sample_count(), 4);
        assert_eq!(frame.pcm_le(), &[0, 128, 255, 255, 0, 0, 255, 127]);
        assert_eq!(
            NormalizedFrame::new(metadata, vec![]).unwrap_err(),
            FrameError::EmptyPayload
        );
        assert_eq!(
            NormalizedFrame::new(metadata, vec![0]).unwrap_err(),
            FrameError::IncompleteSample
        );
        for format in [
            SourceFormat {
                sample_rate: 0,
                channels: 1,
            },
            SourceFormat {
                sample_rate: 16_000,
                channels: 0,
            },
        ] {
            assert_eq!(
                NormalizedFrame::new(
                    FrameMetadata {
                        original_format: format,
                        ..metadata
                    },
                    vec![0, 0]
                )
                .unwrap_err(),
                FrameError::InvalidSourceFormat
            );
        }
    }
}
