use std::fs::File;
use std::io::Read;
use std::path::Path;

use flate2::read::{DeflateDecoder, ZlibDecoder};
use memchr::memmem;

use super::bytes::{le16, le32, read_at};

const PDF_WHOLE: u64 = 64 << 20;
const PDF_EDGE: u64 = 4 << 20;
const DICT_REACH: usize = 64 << 10;
const STREAM_REACH: usize = 4 << 10;
const INFLATE_LIMIT: u64 = 16 << 20;
const ZIP_TAIL: u64 = (64 << 10) + 22;
const ZIP_DIRECTORY_LIMIT: usize = 4 << 20;
const ZIP_ENTRY_LIMIT: u64 = 8 << 20;
const END_OF_DIRECTORY: &[u8] = b"PK\x05\x06";
const DIRECTORY_ENTRY: &[u8] = b"PK\x01\x02";
const LOCAL_ENTRY: &[u8] = b"PK\x03\x04";
const STORED: u16 = 0;
const DEFLATED: u16 = 8;

pub fn pdf_pages(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let data = if len <= PDF_WHOLE {
        read_at(&mut file, 0, len as usize)?
    } else {
        let mut edges = read_at(&mut file, 0, PDF_EDGE as usize)?;
        edges.extend(read_at(&mut file, len - PDF_EDGE, PDF_EDGE as usize)?);
        edges
    };
    if !data.starts_with(b"%PDF") {
        return None;
    }
    page_count(&data).or_else(|| {
        memmem::find_iter(&data, b"/ObjStm")
            .filter_map(|at| inflate_stream(&data, at))
            .filter_map(|objects| page_count(&objects))
            .max()
    })
}

fn page_count(data: &[u8]) -> Option<u64> {
    memmem::find_iter(data, b"/Pages")
        .filter(|at| {
            let before = data[..*at].trim_ascii_end();
            before.ends_with(b"/Type")
                && !data
                    .get(at + 6)
                    .is_some_and(|next| next.is_ascii_alphanumeric())
        })
        .filter_map(|at| {
            let open = memmem::rfind(&data[at.saturating_sub(DICT_REACH)..at], b"<<")
                .map(|open| at.saturating_sub(DICT_REACH) + open)?;
            let close = at + memmem::find(data.get(at..(at + DICT_REACH).min(data.len()))?, b">>")?;
            let dict = &data[open..close];
            let count = memmem::find(dict, b"/Count")?;
            let digits: Vec<u8> = dict[count + 6..]
                .iter()
                .skip_while(|byte| byte.is_ascii_whitespace())
                .take_while(|byte| byte.is_ascii_digit())
                .copied()
                .collect();
            std::str::from_utf8(&digits).ok()?.parse().ok()
        })
        .max()
}

fn inflate_stream(data: &[u8], at: usize) -> Option<Vec<u8>> {
    let reach = data.get(at..(at + STREAM_REACH).min(data.len()))?;
    let mut start = at + memmem::find(reach, b"stream")? + 6;
    if data.get(start) == Some(&b'\r') {
        start += 1;
    }
    if data.get(start) == Some(&b'\n') {
        start += 1;
    }
    let mut out = Vec::new();
    let _ = ZlibDecoder::new(data.get(start..)?)
        .take(INFLATE_LIMIT)
        .read_to_end(&mut out);
    (!out.is_empty()).then_some(out)
}

pub fn office_pages(path: &Path, extension: &str) -> Option<u64> {
    match extension {
        "odt" => {
            let meta = zip_entry(path, "meta.xml")?;
            number_after(&meta, "page-count=\"")
        }
        _ => {
            let app = zip_entry(path, "docProps/app.xml")?;
            number_after(&app, "<Pages>")
        }
    }
}

pub fn office_slides(path: &Path, extension: &str) -> Option<u64> {
    match extension {
        "odp" => {
            let content = zip_entry(path, "content.xml")?;
            let slides = content.matches("<draw:page ").count() as u64;
            (slides > 0).then_some(slides)
        }
        _ => {
            let app = zip_entry(path, "docProps/app.xml")?;
            number_after(&app, "<Slides>")
        }
    }
}

fn number_after(text: &str, marker: &str) -> Option<u64> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|count| *count > 0)
}

fn zip_entry(path: &Path, name: &str) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(ZIP_TAIL);
    let tail = read_at(&mut file, from, (len - from) as usize)?;
    let end = memmem::rfind(&tail, END_OF_DIRECTORY)?;
    let size = le32(&tail, end + 12)? as usize;
    let offset = u64::from(le32(&tail, end + 16)?);
    if size > ZIP_DIRECTORY_LIMIT {
        return None;
    }
    let directory = read_at(&mut file, offset, size)?;
    let mut at = 0;
    while directory.get(at..at + 4) == Some(DIRECTORY_ENTRY) {
        let method = le16(&directory, at + 10)?;
        let packed = u64::from(le32(&directory, at + 20)?);
        let name_len = usize::from(le16(&directory, at + 28)?);
        let extra = usize::from(le16(&directory, at + 30)?);
        let comment = usize::from(le16(&directory, at + 32)?);
        let local = u64::from(le32(&directory, at + 42)?);
        if directory.get(at + 46..at + 46 + name_len)? == name.as_bytes() {
            if packed > ZIP_ENTRY_LIMIT {
                return None;
            }
            let head = read_at(&mut file, local, 30)?;
            if head.get(..4)? != LOCAL_ENTRY {
                return None;
            }
            let skip = 30 + u64::from(le16(&head, 26)?) + u64::from(le16(&head, 28)?);
            let body = read_at(&mut file, local + skip, packed as usize)?;
            let mut text = String::new();
            match method {
                STORED => text = String::from_utf8(body).ok()?,
                DEFLATED => {
                    DeflateDecoder::new(body.as_slice())
                        .take(ZIP_ENTRY_LIMIT)
                        .read_to_string(&mut text)
                        .ok()?;
                }
                _ => return None,
            }
            return Some(text);
        }
        at += 46 + name_len + extra + comment;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, data) in entries {
            let packed = deflate(data);
            let local = out.len() as u32;
            out.extend_from_slice(LOCAL_ENTRY);
            out.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            directory.extend_from_slice(DIRECTORY_ENTRY);
            directory.extend_from_slice(&[20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            directory.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 12]);
            directory.extend_from_slice(&local.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
        }
        let offset = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(END_OF_DIRECTORY);
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn write(name: &str, data: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, data).unwrap();
        (dir, path)
    }

    #[test]
    fn pdfs_count_the_root_page_tree_even_inside_object_streams() {
        let plain = b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Kids [3 0 R 4 0 R] /Type /Pages /Count 148 >> endobj\n3 0 obj << /Type /Pages /Parent 2 0 R /Count 100 >> endobj\n4 0 obj << /Type /Page /Parent 3 0 R >> endobj\n";
        let (_dir, path) = write("report.pdf", plain);
        assert_eq!(pdf_pages(&path), Some(148));

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(b"2 0 3 40 << /Type/Pages/Count 12/Kids[3 0 R] >> << /Type/Page >>")
            .unwrap();
        let mut packed =
            b"%PDF-1.7\n5 0 obj << /Type /ObjStm /N 2 /Filter /FlateDecode >> stream\r\n".to_vec();
        packed.extend_from_slice(&encoder.finish().unwrap());
        packed.extend_from_slice(b"\nendstream endobj\n");
        let (_dir, path) = write("lease.pdf", &packed);
        assert_eq!(pdf_pages(&path), Some(12));

        let (_dir, path) = write("fake.pdf", b"hello");
        assert_eq!(pdf_pages(&path), None);
    }

    #[test]
    fn office_files_read_their_own_statistics() {
        let (_dir, odt) = write(
            "cv.odt",
            &zip(&[
                ("mimetype", b"application/vnd.oasis.opendocument.text"),
                (
                    "meta.xml",
                    b"<meta:document-statistic meta:table-count=\"0\" meta:page-count=\"2\"/>",
                ),
            ]),
        );
        assert_eq!(office_pages(&odt, "odt"), Some(2));
        let (_dir, docx) = write(
            "cv.docx",
            &zip(&[(
                "docProps/app.xml",
                b"<Properties><Pages>7</Pages></Properties>",
            )]),
        );
        assert_eq!(office_pages(&docx, "docx"), Some(7));
        let (_dir, odp) = write(
            "deck.odp",
            &zip(&[(
                "content.xml",
                b"<office:presentation><draw:page draw:name=\"1\"/><draw:page draw:name=\"2\"/></office:presentation>",
            )]),
        );
        assert_eq!(office_slides(&odp, "odp"), Some(2));
        let (_dir, pptx) = write(
            "deck.pptx",
            &zip(&[(
                "docProps/app.xml",
                b"<Properties><Slides>42</Slides></Properties>",
            )]),
        );
        assert_eq!(office_slides(&pptx, "pptx"), Some(42));
        assert_eq!(office_pages(&pptx, "docx"), None);
    }
}
