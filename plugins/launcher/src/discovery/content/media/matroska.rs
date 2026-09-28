use std::fs::File;

use super::super::bytes::read_at;
use super::Media;

const HEAD_LIMIT: usize = 1 << 20;
const EBML: u64 = 0x1A45_DFA3;
const SEGMENT: u64 = 0x1853_8067;
const INFO: u64 = 0x1549_A966;
const TIMESTAMP_SCALE: u64 = 0x2A_D7B1;
const DURATION: u64 = 0x4489;
const TRACKS: u64 = 0x1654_AE6B;
const TRACK: u64 = 0xAE;
const TRACK_TYPE: u64 = 0x83;
const FRAME_TIME: u64 = 0x23_E383;
const VIDEO: u64 = 0xE0;
const PIXEL_WIDTH: u64 = 0xB0;
const PIXEL_HEIGHT: u64 = 0xBA;
const AUDIO: u64 = 0xE1;
const BIT_DEPTH: u64 = 0x6264;
const CLUSTER: u64 = 0x1F43_B675;
const VIDEO_TRACK: u64 = 1;
const NANOS: f64 = 1e9;

struct Element {
    id: u64,
    start: usize,
    end: usize,
}

pub fn read(file: &mut File) -> Option<Media> {
    let data = read_at(file, 0, HEAD_LIMIT)?;
    let top = elements(&data, 0, data.len());
    if top.first()?.id != EBML {
        return None;
    }
    let segment = top.iter().find(|element| element.id == SEGMENT)?;
    let mut scale = 1_000_000.0;
    let mut length = None;
    let mut media = Media::default();
    for element in elements(&data, segment.start, segment.end) {
        let body = &data[element.start..element.end];
        match element.id {
            INFO => {
                for field in elements(body, 0, body.len()) {
                    let value = &body[field.start..field.end];
                    match field.id {
                        TIMESTAMP_SCALE => scale = uint(value)? as f64,
                        DURATION => length = float(value),
                        _ => {}
                    }
                }
            }
            TRACKS => {
                for track in elements(body, 0, body.len())
                    .iter()
                    .filter(|track| track.id == TRACK)
                {
                    read_track(&body[track.start..track.end], &mut media);
                }
            }
            _ => {}
        }
    }
    media.seconds = length.map(|length| length * scale / NANOS);
    Some(media)
}

fn read_track(track: &[u8], media: &mut Media) {
    let fields = elements(track, 0, track.len());
    let value = |id| {
        fields
            .iter()
            .find(|field| field.id == id)
            .map(|field| &track[field.start..field.end])
    };
    if value(TRACK_TYPE).and_then(uint) == Some(VIDEO_TRACK) {
        if media.size.is_some() {
            return;
        }
        let video = value(VIDEO).unwrap_or_default();
        let pixels = elements(video, 0, video.len());
        let pixel = |id| {
            pixels
                .iter()
                .find(|field| field.id == id)
                .and_then(|field| uint(&video[field.start..field.end]))
        };
        media.size = pixel(PIXEL_WIDTH).zip(pixel(PIXEL_HEIGHT));
        media.fps = value(FRAME_TIME)
            .and_then(uint)
            .filter(|nanos| *nanos > 0)
            .map(|nanos| NANOS / nanos as f64);
    } else if let Some(audio) = value(AUDIO) {
        let fields = elements(audio, 0, audio.len());
        media.bits = media.bits.or_else(|| {
            fields
                .iter()
                .find(|field| field.id == BIT_DEPTH)
                .and_then(|field| uint(&audio[field.start..field.end]))
        });
    }
}

fn elements(data: &[u8], from: usize, to: usize) -> Vec<Element> {
    let mut found = Vec::new();
    let mut at = from;
    while at < to {
        let Some((id, id_len)) = vint(data, at, true) else {
            break;
        };
        let Some((size, size_len)) = vint(data, at + id_len, false) else {
            break;
        };
        let start = at + id_len + size_len;
        let unknown = size == (1u64 << (7 * size_len)) - 1;
        let end = if unknown {
            to
        } else {
            start.saturating_add(size as usize).min(to)
        };
        if id == CLUSTER || start > to {
            break;
        }
        found.push(Element { id, start, end });
        at = end;
    }
    found
}

fn vint(data: &[u8], at: usize, marked: bool) -> Option<(u64, usize)> {
    let first = *data.get(at)?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 {
        return None;
    }
    let lead = if marked {
        u64::from(first)
    } else {
        u64::from(first) & ((1u64 << (8 - len)) - 1)
    };
    let value = data
        .get(at + 1..at + len)?
        .iter()
        .fold(lead, |value, byte| (value << 8) | u64::from(*byte));
    Some((value, len))
}

fn uint(data: &[u8]) -> Option<u64> {
    (!data.is_empty() && data.len() <= 8).then(|| {
        data.iter()
            .fold(0, |value, byte| (value << 8) | u64::from(*byte))
    })
}

fn float(data: &[u8]) -> Option<f64> {
    match data.len() {
        4 => Some(f64::from(f32::from_be_bytes(data.try_into().ok()?))),
        8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(id: u64, body: &[u8]) -> Vec<u8> {
        let id_bytes = id.to_be_bytes();
        let skip = id_bytes.iter().take_while(|byte| **byte == 0).count();
        let mut out = id_bytes[skip..].to_vec();
        out.push(0x01);
        out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    fn movie() -> Vec<u8> {
        let info = [
            element(TIMESTAMP_SCALE, &1_000_000u32.to_be_bytes()),
            element(DURATION, &6_128_000.0f64.to_be_bytes()),
        ]
        .concat();
        let video = [
            element(PIXEL_WIDTH, &3840u16.to_be_bytes()),
            element(PIXEL_HEIGHT, &2160u16.to_be_bytes()),
        ]
        .concat();
        let picture = [
            element(TRACK_TYPE, &[1]),
            element(FRAME_TIME, &16_666_667u32.to_be_bytes()),
            element(VIDEO, &video),
        ]
        .concat();
        let sound = [
            element(TRACK_TYPE, &[2]),
            element(AUDIO, &element(BIT_DEPTH, &[24])),
        ]
        .concat();
        let tracks = [element(TRACK, &picture), element(TRACK, &sound)].concat();
        let segment = [
            element(INFO, &info),
            element(TRACKS, &tracks),
            element(CLUSTER, &[0; 32]),
        ]
        .concat();
        [
            element(EBML, &[0x42, 0x82, 0x84, b'w', b'e', b'b', b'm']),
            element(SEGMENT, &segment),
        ]
        .concat()
    }

    #[test]
    fn matroska_reads_info_and_tracks_before_the_first_cluster() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("film.mkv");
        std::fs::write(&path, movie()).unwrap();
        let media = read(&mut File::open(&path).unwrap()).expect("matroska");
        assert_eq!(media.seconds, Some(6128.0));
        assert_eq!(media.size, Some((3840, 2160)));
        assert!((media.fps.unwrap() - 60.0).abs() < 0.01);
        assert_eq!(media.bits, Some(24));
        assert_eq!(vint(&[0x1A, 0x45, 0xDF, 0xA3], 0, true), Some((EBML, 4)));
        assert_eq!(vint(&[0x81], 0, false), Some((1, 1)));
        assert_eq!(vint(&[0x40, 0x02], 0, false), Some((2, 2)));
    }
}
