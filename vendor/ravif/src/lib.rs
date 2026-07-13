use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

pub use imgref::Img;
pub use rgb::{RGB8, RGBA8};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ColorModel {
    YCbCr,
    RGB,
}

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
pub enum BitDepth {
    Eight,
    Ten,
    #[default]
    Auto,
}

#[non_exhaustive]
#[derive(Clone)]
pub struct EncodedImage {
    pub avif_file: Vec<u8>,
    pub color_byte_size: usize,
    pub alpha_byte_size: usize,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(msg) => write!(f, "Not supported: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

const STUB_MESSAGE: &str =
    "AVIF encoding is stubbed out in this workspace; vendor/ravif replaces the rav1e stack";

#[derive(Debug, Clone)]
pub struct Encoder<'exif_slice> {
    exif: PhantomData<&'exif_slice [u8]>,
}

impl<'exif_slice> Encoder<'exif_slice> {
    pub fn new() -> Self {
        Self { exif: PhantomData }
    }

    pub fn with_quality(self, _quality: f32) -> Self {
        self
    }

    pub fn with_alpha_quality(self, _quality: f32) -> Self {
        self
    }

    pub fn with_speed(self, _speed: u8) -> Self {
        self
    }

    pub fn with_bit_depth(self, _depth: BitDepth) -> Self {
        self
    }

    pub fn with_internal_color_model(self, _color_model: ColorModel) -> Self {
        self
    }

    pub fn with_num_threads(self, _num_threads: Option<usize>) -> Self {
        self
    }

    pub fn with_exif(self, _exif_data: impl Into<Cow<'exif_slice, [u8]>>) -> Self {
        self
    }

    pub fn encode_rgba(&self, _in_buffer: Img<&[RGBA8]>) -> Result<EncodedImage, Error> {
        Err(Error::Unsupported(STUB_MESSAGE))
    }

    pub fn encode_rgb(&self, _buffer: Img<&[RGB8]>) -> Result<EncodedImage, Error> {
        Err(Error::Unsupported(STUB_MESSAGE))
    }
}

impl Default for Encoder<'_> {
    fn default() -> Self {
        Self::new()
    }
}
