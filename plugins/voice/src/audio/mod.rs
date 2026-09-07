#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioEncoding {
    PcmS16Le,
}

impl AudioEncoding {
    pub fn protocol_name(self) -> &'static str {
        match self {
            Self::PcmS16Le => "PCM_S16LE",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::PcmS16Le => "s16le",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub encoding: AudioEncoding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioFrame {
    pub observed_at_ms: u64,
    pub pcm: Vec<u8>,
}
