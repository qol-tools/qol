use anyhow::{anyhow, Context, Result};
use std::os::fd::AsRawFd;
use x11rb::connection::Connection;
use x11rb::protocol::shm::{self, ConnectionExt as ShmExt};
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
use x11rb::rust_connection::RustConnection;

use crate::capture::frozen_frame::FrozenFrame;
use crate::Rect;

pub fn capture_frozen_frame() -> Result<Option<FrozenFrame>> {
    let (conn, root) = connection_and_root()?;
    let geometry = conn
        .get_geometry(root)
        .context("failed to request X11 root geometry")?
        .reply()
        .context("failed to read X11 root geometry")?;
    let bounds = Rect {
        x: 0,
        y: 0,
        w: i32::from(geometry.width),
        h: i32::from(geometry.height),
    };
    let bgra = grab_bgra(&conn, root, &bounds)?;
    frozen_frame_from_bgra(bounds, bgra)
        .map(Some)
        .context("X11 frozen frame dimensions did not match its pixel buffer")
}

fn frozen_frame_from_bgra(bounds: Rect, pixels: Vec<u8>) -> Option<FrozenFrame> {
    let width = u32::try_from(bounds.w).ok()?;
    let height = u32::try_from(bounds.h).ok()?;
    FrozenFrame::from_bgra_segments(vec![(bounds, pixels, width, height)])
}

pub fn grab_preview_rgba(rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    let (conn, root) = connection_and_root().ok()?;
    let mut data = grab_bgra(&conn, root, rect).ok()?;
    bgra_to_rgba(&mut data);
    Some((data, rect.w as u32, rect.h as u32))
}

fn connection_and_root() -> Result<(RustConnection, u32)> {
    let (conn, screen_num) = x11rb::connect(None).context("failed to connect to X11")?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .context("X11 connection did not expose the selected screen")?
        .root;
    Ok((conn, root))
}

fn grab_bgra(conn: &RustConnection, root: u32, rect: &Rect) -> Result<Vec<u8>> {
    let started = std::time::Instant::now();
    let x = i16::try_from(rect.x).context("X11 capture x coordinate is out of range")?;
    let y = i16::try_from(rect.y).context("X11 capture y coordinate is out of range")?;
    let width = u16::try_from(rect.w).context("X11 capture width is out of range")?;
    let height = u16::try_from(rect.h).context("X11 capture height is out of range")?;
    let expected = usize::from(width) * usize::from(height) * 4;
    let (mut data, path) = match grab_bgra_shm(conn, root, x, y, width, height, expected) {
        Ok(data) => (data, "shm"),
        Err(_) => (
            grab_bgra_wire(conn, root, x, y, width, height, expected)?,
            "wire",
        ),
    };
    let alpha_started = std::time::Instant::now();
    force_opaque_bgra(&mut data);
    qol_runtime::probe!(
        "SHOT_X11_GRAB",
        "path={path} ms={} alpha_ms={} dims={width}x{height}",
        started.elapsed().as_millis(),
        alpha_started.elapsed().as_millis()
    );
    Ok(data)
}

fn force_opaque_bgra(data: &mut [u8]) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        unsafe { force_opaque_bgra_avx2(data) };
        return;
    }
    force_opaque_bgra_scalar(data);
}

fn force_opaque_bgra_scalar(data: &mut [u8]) {
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel[3] = 255;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn force_opaque_bgra_avx2(data: &mut [u8]) {
    use std::arch::x86_64::_mm256_storeu_si256;
    use std::arch::x86_64::{__m256i, _mm256_loadu_si256, _mm256_or_si256, _mm256_set1_epi32};

    let mask = _mm256_set1_epi32(0xff00_0000_u32 as i32);
    let mut offset = 0;
    while offset + 256 <= data.len() {
        unsafe {
            opaque_avx2_block(data.as_mut_ptr().add(offset), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 32), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 64), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 96), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 128), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 160), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 192), mask);
            opaque_avx2_block(data.as_mut_ptr().add(offset + 224), mask);
        }
        offset += 256;
    }
    while offset + 32 <= data.len() {
        unsafe { opaque_avx2_block(data.as_mut_ptr().add(offset), mask) };
        offset += 32;
    }
    force_opaque_bgra_scalar(&mut data[offset..]);

    #[target_feature(enable = "avx2")]
    unsafe fn opaque_avx2_block(data: *mut u8, mask: __m256i) {
        let pixels = unsafe { _mm256_loadu_si256(data.cast()) };
        unsafe { _mm256_storeu_si256(data.cast(), _mm256_or_si256(pixels, mask)) };
    }
}

fn grab_bgra_shm(
    conn: &RustConnection,
    root: u32,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    expected: usize,
) -> Result<Vec<u8>> {
    let version_started = std::time::Instant::now();
    let version = conn
        .shm_query_version()
        .context("failed to query X11 SHM")?
        .reply()
        .context("failed to read X11 SHM version")?;
    if !shm_supports_fd_segments(version.major_version, version.minor_version) {
        return Err(anyhow!("X11 SHM fd segments are unavailable"));
    }
    let version_ms = version_started.elapsed().as_millis();
    let segment_started = std::time::Instant::now();
    let segment = MappedSegment::new(conn, expected)?;
    let segment_ms = segment_started.elapsed().as_millis();
    let image_started = std::time::Instant::now();
    let reply = conn
        .shm_get_image(
            root,
            x,
            y,
            width,
            height,
            u32::MAX,
            u8::from(ImageFormat::Z_PIXMAP),
            segment.id,
            0,
        )
        .context("failed to request X11 SHM image")?
        .reply()
        .context("failed to read X11 SHM image")?;
    let image_ms = image_started.elapsed().as_millis();
    if usize::try_from(reply.size).ok() != Some(expected) {
        return Err(anyhow!(
            "X11 SHM image returned {} bytes, expected {expected}",
            reply.size
        ));
    }
    let copy_started = std::time::Instant::now();
    let data = segment.bytes().to_vec();
    qol_runtime::probe!(
        "SHOT_X11_SHM",
        "version_ms={version_ms} segment_ms={segment_ms} image_ms={image_ms} copy_ms={}",
        copy_started.elapsed().as_millis()
    );
    Ok(data)
}

fn grab_bgra_wire(
    conn: &RustConnection,
    root: u32,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    expected: usize,
) -> Result<Vec<u8>> {
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, x, y, width, height, u32::MAX)
        .context("failed to request X11 image")?
        .reply()
        .context("failed to read X11 image")?;

    if reply.data.len() != expected {
        return Err(anyhow!(
            "X11 image returned {} bytes for a {width}x{height} BGRA frame",
            reply.data.len()
        ));
    }
    Ok(reply.data)
}

fn shm_supports_fd_segments(major: u16, minor: u16) -> bool {
    major > 1 || major == 1 && minor >= 2
}

struct MappedSegment<'a> {
    conn: &'a RustConnection,
    id: shm::Seg,
    ptr: *mut libc::c_void,
    len: usize,
}

impl<'a> MappedSegment<'a> {
    fn new(conn: &'a RustConnection, len: usize) -> Result<Self> {
        let id = conn
            .generate_id()
            .context("failed to allocate X11 SHM id")?;
        let size = u32::try_from(len).context("X11 SHM frame is too large")?;
        let reply = conn
            .shm_create_segment(id, size, false)
            .context("failed to create X11 SHM segment")?
            .reply()
            .context("failed to receive X11 SHM segment")?;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                reply.shm_fd.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            let _ = conn.shm_detach(id);
            return Err(anyhow!("failed to map X11 SHM segment"));
        }
        Ok(Self { conn, id, ptr, len })
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.cast(), self.len) }
    }
}

impl Drop for MappedSegment<'_> {
    fn drop(&mut self) {
        let _ = self.conn.shm_detach(self.id);
        let _ = self.conn.flush();
        unsafe {
            libc::munmap(self.ptr, self.len);
        }
    }
}

fn bgra_to_rgba(data: &mut [u8]) {
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::{bgra_to_rgba, capture_frozen_frame, force_opaque_bgra, shm_supports_fd_segments};

    #[test]
    fn x11_bgra_alpha_is_normalized_without_changing_color_channels() {
        let cases = [
            (vec![], vec![]),
            (vec![1, 2, 3, 0], vec![1, 2, 3, 255]),
            (
                vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
                vec![1, 2, 3, 255, 5, 6, 7, 255, 9, 10, 11, 255],
            ),
            (
                (0..65).flat_map(|value| [value, 2, 3, 4]).collect(),
                (0..65).flat_map(|value| [value, 2, 3, 255]).collect(),
            ),
        ];
        for (mut pixels, expected) in cases {
            force_opaque_bgra(&mut pixels);
            assert_eq!(pixels, expected);
        }
    }

    #[test]
    fn x11_bgra_is_normalized_to_rgba() {
        let mut pixels = [1, 2, 3, 255, 4, 5, 6, 255];
        bgra_to_rgba(&mut pixels);
        assert_eq!(pixels, [3, 2, 1, 255, 6, 5, 4, 255]);
    }

    #[test]
    fn shm_fd_capture_requires_version_1_2() {
        let cases = [
            (0, 0, false),
            (1, 1, false),
            (1, 2, true),
            (1, 3, true),
            (2, 0, true),
        ];
        for (major, minor, expected) in cases {
            assert_eq!(
                shm_supports_fd_segments(major, minor),
                expected,
                "version: {major}.{minor}"
            );
        }
    }

    #[test]
    #[ignore = "requires a live X11 desktop"]
    fn live_x11_capture_returns_the_root_frame() {
        let frame = capture_frozen_frame().unwrap().unwrap();
        let bounds = frame.bounds();
        assert!(bounds.w > 0);
        assert!(bounds.h > 0);
    }
}
