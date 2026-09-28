use std::fs::File;

use super::super::bytes::{be32, be64, read_at};
use super::Media;

const MOOV_LIMIT: u64 = 32 << 20;

pub fn read(file: &mut File, len: u64) -> Option<Media> {
    let mut at = 0;
    while at + 8 <= len {
        let head = read_at(file, at, 16)?;
        let (header, size) = match be32(&head, 0)? {
            0 => (8, len - at),
            1 => (16, be64(&head, 8)?),
            size => (8, u64::from(size)),
        };
        if size < header {
            return None;
        }
        if head.get(4..8)? == b"moov" {
            if size - header > MOOV_LIMIT {
                return None;
            }
            let moov = read_at(file, at + header, (size - header) as usize)?;
            return Some(movie(&moov));
        }
        at += size;
    }
    None
}

fn movie(moov: &[u8]) -> Media {
    let mut media = Media {
        seconds: child(moov, b"mvhd").and_then(header_seconds),
        ..Media::default()
    };
    for track in children(moov)
        .into_iter()
        .filter(|(kind, _)| kind == b"trak")
        .map(|(_, body)| body)
    {
        let Some(mdia) = child(track, b"mdia") else {
            continue;
        };
        if media.size.is_some()
            || child(mdia, b"hdlr").and_then(|hdlr| hdlr.get(8..12)) != Some(b"vide")
        {
            continue;
        }
        media.size = child(track, b"tkhd").and_then(track_size);
        let seconds = child(mdia, b"mdhd").and_then(header_seconds);
        let samples = child(mdia, b"minf")
            .and_then(|minf| child(minf, b"stbl"))
            .and_then(|stbl| child(stbl, b"stts"))
            .and_then(sample_count);
        media.fps = samples
            .zip(seconds)
            .map(|(samples, seconds)| samples as f64 / seconds);
    }
    media
}

fn children(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 8 <= data.len() {
        let Some(declared) = be32(data, at) else {
            break;
        };
        let (header, size) = match declared {
            0 => (8, data.len() - at),
            1 => match be64(data, at + 8) {
                Some(size) => (16, size as usize),
                None => break,
            },
            size => (8, size as usize),
        };
        let Some(end) = at
            .checked_add(size)
            .filter(|end| size >= header && *end <= data.len())
        else {
            break;
        };
        let mut kind = [0; 4];
        kind.copy_from_slice(&data[at + 4..at + 8]);
        found.push((kind, &data[at + header..end]));
        at = end;
    }
    found
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, body)| body)
}

fn header_seconds(header: &[u8]) -> Option<f64> {
    let (scale, duration) = match *header.first()? {
        1 => (be32(header, 20)?, be64(header, 24)?),
        _ => (be32(header, 12)?, u64::from(be32(header, 16)?)),
    };
    (scale > 0 && duration > 0 && duration != u64::from(u32::MAX) && duration != u64::MAX)
        .then(|| duration as f64 / f64::from(scale))
}

fn track_size(tkhd: &[u8]) -> Option<(u64, u64)> {
    let end = tkhd.len();
    let width = u64::from(be32(tkhd, end.checked_sub(8)?)? >> 16);
    let height = u64::from(be32(tkhd, end - 4)? >> 16);
    let matrix = end.checked_sub(44)?;
    let turned = be32(tkhd, matrix)? == 0 && be32(tkhd, matrix + 16)? == 0;
    (width > 0 && height > 0).then_some(if turned {
        (height, width)
    } else {
        (width, height)
    })
}

fn sample_count(stts: &[u8]) -> Option<u64> {
    let entries = be32(stts, 4)? as usize;
    Some(
        (0..entries)
            .map_while(|entry| be32(stts, 8 + entry * 8))
            .map(u64::from)
            .sum(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn header(scale: u32, duration: u32) -> Vec<u8> {
        let mut body = vec![0; 12];
        body.extend_from_slice(&scale.to_be_bytes());
        body.extend_from_slice(&duration.to_be_bytes());
        body.extend_from_slice(&[0; 80]);
        body
    }

    fn tkhd(width: u32, height: u32, turned: bool) -> Vec<u8> {
        let mut body = vec![0; 40];
        let (a, b, c, d) = if turned {
            (0i32, 0x10000i32, -0x10000i32, 0i32)
        } else {
            (0x10000, 0, 0, 0x10000)
        };
        for value in [a, b, 0, c, d, 0, 0, 0, 0x4000_0000] {
            body.extend_from_slice(&value.to_be_bytes());
        }
        body.extend_from_slice(&(width << 16).to_be_bytes());
        body.extend_from_slice(&(height << 16).to_be_bytes());
        body
    }

    fn video(turned: bool) -> Vec<u8> {
        let mut stts = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stts.extend_from_slice(&600u32.to_be_bytes());
        stts.extend_from_slice(&1000u32.to_be_bytes());
        let mut hdlr = vec![0; 8];
        hdlr.extend_from_slice(b"vide");
        let mdia = [
            atom(b"mdhd", &header(60_000, 600_000)),
            atom(b"hdlr", &hdlr),
            atom(b"minf", &atom(b"stbl", &atom(b"stts", &stts))),
        ]
        .concat();
        let trak = [
            atom(b"tkhd", &tkhd(1920, 1080, turned)),
            atom(b"mdia", &mdia),
        ]
        .concat();
        let moov = [atom(b"mvhd", &header(1000, 10_000)), atom(b"trak", &trak)].concat();
        [
            atom(b"ftyp", b"isom"),
            atom(b"mdat", &[0; 64]),
            atom(b"moov", &moov),
        ]
        .concat()
    }

    fn probe(data: &[u8]) -> Option<Media> {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clip.mp4");
        std::fs::write(&path, data).unwrap();
        let mut file = File::open(&path).unwrap();
        read(&mut file, data.len() as u64)
    }

    #[test]
    fn movies_read_length_size_and_rate_from_the_movie_box_at_the_end() {
        let media = probe(&video(false)).expect("moov");
        assert_eq!(media.seconds, Some(10.0));
        assert_eq!(media.size, Some((1920, 1080)));
        assert_eq!(media.fps, Some(60.0));
        assert_eq!(
            probe(&video(true)).and_then(|media| media.size),
            Some((1080, 1920))
        );
        assert!(probe(b"not a movie at all").is_none());
    }
}
