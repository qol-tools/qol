use std::fs::File;

use memchr::memmem;

use super::super::bytes::{be32, le16, le32, le64, read_at, syncsafe};
use super::Media;

const TAG_HEAD: usize = 10;
const WAV_SCAN: usize = 64;
const OGG_TAIL: u64 = 64 << 10;
const OPUS_RATE: f64 = 48_000.0;
const MP3_HEAD: usize = 64 << 10;
const MP3_RATES: [u32; 3] = [44_100, 48_000, 32_000];
const MPEG1_KBPS: [u32; 15] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_KBPS: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

pub fn flac(file: &mut File) -> Option<Media> {
    let start = tag_end(file)?;
    let head = read_at(file, start, 42)?;
    if head.get(..4)? != b"fLaC" || head.get(4)? & 0x7f != 0 {
        return None;
    }
    let packed = u64::from_be_bytes(head.get(18..26)?.try_into().ok()?);
    let rate = packed >> 44;
    let bits = ((packed >> 36) & 0x1f) + 1;
    let samples = packed & 0xF_FFFF_FFFF;
    Some(Media {
        seconds: (rate > 0 && samples > 0).then(|| samples as f64 / rate as f64),
        bits: Some(bits),
        ..Media::default()
    })
}

pub fn wav(file: &mut File, len: u64) -> Option<Media> {
    let head = read_at(file, 0, 12)?;
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut format = None;
    for _ in 0..WAV_SCAN {
        let chunk = read_at(file, at, 24)?;
        let size = u64::from(le32(&chunk, 4)?);
        match chunk.get(..4)? {
            b"fmt " => format = Some((le32(&chunk, 16)?, le16(&chunk, 22)?)),
            b"data" => {
                let (rate, bits) = format?;
                let size = if size == u64::from(u32::MAX) {
                    len - at - 8
                } else {
                    size
                };
                return Some(Media {
                    seconds: (rate > 0).then(|| size as f64 / f64::from(rate)),
                    bits: Some(u64::from(bits)),
                    ..Media::default()
                });
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    None
}

pub fn ogg(file: &mut File, len: u64) -> Option<Media> {
    let head = read_at(file, 0, 512)?;
    if head.get(..4)? != b"OggS" {
        return None;
    }
    let body = 27 + usize::from(*head.get(26)?);
    let packet = head.get(body..)?;
    let (rate, skip) = if packet.starts_with(b"\x01vorbis") {
        (f64::from(le32(packet, 12)?), 0)
    } else if packet.starts_with(b"OpusHead") {
        (OPUS_RATE, u64::from(le16(packet, 10)?))
    } else {
        return None;
    };
    let from = len.saturating_sub(OGG_TAIL);
    let tail = read_at(file, from, OGG_TAIL as usize)?;
    let last = memmem::rfind_iter(&tail, b"OggS").find(|at| tail.get(at + 4) == Some(&0))?;
    let granule = le64(&tail, last + 6)?;
    Some(Media {
        seconds: (rate > 0.0 && granule != u64::MAX)
            .then(|| granule.saturating_sub(skip) as f64 / rate),
        ..Media::default()
    })
}

pub fn mp3(file: &mut File, len: u64) -> Option<Media> {
    let start = tag_end(file)?;
    let data = read_at(file, start, MP3_HEAD)?;
    let (at, frame) = (0..data.len().saturating_sub(4))
        .find_map(|at| Frame::parse(data.get(at..at + 4)?).map(|frame| (at, frame)))?;
    let body = data.get(at..)?;
    let side = match (frame.mpeg1, frame.mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let xing = 4 + side;
    let frames = if matches!(body.get(xing..xing + 4), Some(b"Xing" | b"Info"))
        && be32(body, xing + 4)? & 1 == 1
    {
        be32(body, xing + 8)
    } else if body.get(36..40) == Some(b"VBRI") {
        be32(body, 50)
    } else {
        None
    };
    let seconds = match frames {
        Some(frames) => f64::from(frames) * f64::from(frame.samples) / f64::from(frame.rate),
        None => {
            let audio = len.saturating_sub(start + at as u64);
            audio as f64 * 8.0 / (f64::from(frame.kbps) * 1000.0)
        }
    };
    Some(Media {
        seconds: Some(seconds),
        ..Media::default()
    })
}

struct Frame {
    mpeg1: bool,
    mono: bool,
    rate: u32,
    kbps: u32,
    samples: u32,
}

impl Frame {
    fn parse(head: &[u8]) -> Option<Self> {
        let &[sync, flags, rates, mode] = head else {
            return None;
        };
        if sync != 0xFF || flags & 0xE0 != 0xE0 {
            return None;
        }
        let version = (flags >> 3) & 3;
        let layer = (flags >> 1) & 3;
        let kbps_index = usize::from(rates >> 4);
        let rate_index = usize::from((rates >> 2) & 3);
        if version == 1 || layer != 1 || kbps_index == 0 || kbps_index == 15 || rate_index == 3 {
            return None;
        }
        let mpeg1 = version == 3;
        let divisor = match version {
            3 => 1,
            2 => 2,
            _ => 4,
        };
        Some(Self {
            mpeg1,
            mono: mode >> 6 == 3,
            rate: MP3_RATES[rate_index] / divisor,
            kbps: if mpeg1 { MPEG1_KBPS } else { MPEG2_KBPS }[kbps_index],
            samples: if mpeg1 { 1152 } else { 576 },
        })
    }
}

fn tag_end(file: &mut File) -> Option<u64> {
    let head = read_at(file, 0, TAG_HEAD)?;
    if head.get(..3) != Some(b"ID3") {
        return Some(0);
    }
    let footer = if head.get(5)? & 0x10 != 0 { 10 } else { 0 };
    Some(TAG_HEAD as u64 + syncsafe(head.get(6..)?)? + footer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(name: &str, data: &[u8], read: fn(&mut File, u64) -> Option<Media>) -> Option<Media> {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, data).unwrap();
        read(&mut File::open(&path).unwrap(), data.len() as u64)
    }

    #[test]
    fn flac_reads_stream_info_after_a_tag() {
        let mut data = b"ID3\x04\0\0\0\0\0\x02\0\0".to_vec();
        data.extend_from_slice(b"fLaC\x80\0\0\x22");
        data.extend_from_slice(&[0; 10]);
        let packed: u64 = (48_000 << 44) | (1 << 41) | (23 << 36) | (48_000 * 252);
        data.extend_from_slice(&packed.to_be_bytes());
        data.extend_from_slice(&[0; 16]);
        let media = probe("mix.flac", &data, |file, _| flac(file)).expect("flac");
        assert_eq!(media.seconds, Some(252.0));
        assert_eq!(media.bits, Some(24));
    }

    #[test]
    fn wav_divides_the_data_by_the_byte_rate() {
        let mut data = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x02\0".to_vec();
        data.extend_from_slice(&44_100u32.to_le_bytes());
        data.extend_from_slice(&176_400u32.to_le_bytes());
        data.extend_from_slice(&[4, 0, 16, 0]);
        data.extend_from_slice(b"LIST\x03\0\0\0abc\0data");
        data.extend_from_slice(&(176_400u32 * 3).to_le_bytes());
        let media = probe("take.wav", &data, wav).expect("wav");
        assert_eq!(media.seconds, Some(3.0));
        assert_eq!(media.bits, Some(16));
    }

    #[test]
    fn ogg_reads_the_last_granule() {
        let page = |granule: u64, packet: &[u8]| {
            let mut page = b"OggS\0\x02".to_vec();
            page.extend_from_slice(&granule.to_le_bytes());
            page.extend_from_slice(&[0; 12]);
            page.push(1);
            page.push(packet.len() as u8);
            page.extend_from_slice(packet);
            page
        };
        let mut opus = b"OpusHead\x01\x02".to_vec();
        opus.extend_from_slice(&312u16.to_le_bytes());
        opus.extend_from_slice(&48_000u32.to_le_bytes());
        let data = [page(0, &opus), page(48_000 * 90 + 312, b"audio")].concat();
        let media = probe("voice.opus", &data, ogg).expect("opus");
        assert_eq!(media.seconds, Some(90.0));
    }

    #[test]
    fn mp3_counts_xing_frames_or_falls_back_to_the_bitrate() {
        let mut vbr = vec![0xFF, 0xFB, 0x90, 0x00];
        vbr.extend_from_slice(&[0; 32]);
        vbr.extend_from_slice(b"Xing\0\0\0\x01");
        vbr.extend_from_slice(&1000u32.to_be_bytes());
        vbr.extend_from_slice(&[0; 400]);
        let media = probe("song.mp3", &vbr, mp3).expect("vbr");
        assert!((media.seconds.unwrap() - 1000.0 * 1152.0 / 44_100.0).abs() < 1e-6);
        let mut cbr = vec![0xFF, 0xFB, 0x90, 0x00];
        cbr.resize(16_000, 0);
        let media = probe("talk.mp3", &cbr, mp3).expect("cbr");
        assert!((media.seconds.unwrap() - 1.0).abs() < 1e-6);
    }
}
