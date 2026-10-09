use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, WAVEFORMATEX, WAVE_FORMAT_PCM,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::capture::{Pcm16Format, Source};
use crate::devices::Direction;
use crate::AudioError;

use super::com::{self, Com};

const POLL: Duration = Duration::from_millis(10);
const BUFFER_HNS: i64 = 2_000_000;
const BYTES_PER_SAMPLE: u16 = 2;

type Chunk = io::Result<Vec<u8>>;

pub(crate) struct Capture {
    device: String,
    chunks: mpsc::Receiver<Chunk>,
    pending: Vec<u8>,
    offset: usize,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Capture {
    pub(crate) fn open(
        source: Source,
        device: Option<&str>,
        format: Pcm16Format,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, AudioError> {
        let (opened_tx, opened_rx) = mpsc::channel();
        let (chunk_tx, chunks) = mpsc::channel();
        let requested = device.map(str::to_owned);
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("qol-audio-capture".to_owned())
            .spawn(move || {
                record(
                    source,
                    requested.as_deref(),
                    format,
                    &opened_tx,
                    &chunk_tx,
                    &worker_stop,
                )
            })
            .map_err(|error| {
                AudioError::Operation(format!(
                    "cannot start {} capture: {error}",
                    source_noun(source)
                ))
            })?;
        let opened = opened_rx.recv().unwrap_or_else(|_| {
            Err(AudioError::Operation(format!(
                "{} capture stopped before it opened",
                source_noun(source)
            )))
        });
        match opened {
            Ok(device) => Ok(Self {
                device,
                chunks,
                pending: Vec::new(),
                offset: 0,
                stop,
                worker: Some(worker),
            }),
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }

    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    pub(crate) fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while self.offset == self.pending.len() {
            match self.chunks.recv() {
                Ok(Ok(chunk)) => {
                    self.pending = chunk;
                    self.offset = 0;
                }
                Ok(Err(error)) => return Err(error),
                Err(mpsc::RecvError) => return Ok(0),
            }
        }
        let available = &self.pending[self.offset..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.offset += count;
        Ok(count)
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Stream {
    device: String,
    client: IAudioClient,
    reader: IAudioCaptureClient,
    block_align: usize,
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

fn record(
    source: Source,
    device: Option<&str>,
    format: Pcm16Format,
    opened: &mpsc::Sender<Result<String, AudioError>>,
    chunks: &mpsc::Sender<Chunk>,
    stop: &AtomicBool,
) {
    let _com = match Com::enter() {
        Ok(com) => com,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    let stream = match open_stream(source, device, format) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    if opened.send(Ok(stream.device.clone())).is_err() {
        return;
    }
    while !stop.load(Ordering::Acquire) {
        let mut pcm = Vec::new();
        if let Err(error) = stream.drain(&mut pcm) {
            let _ = chunks.send(Err(io::Error::other(format!(
                "the {} stream failed: {error}",
                source_noun(source)
            ))));
            return;
        }
        if !pcm.is_empty() && chunks.send(Ok(pcm)).is_err() {
            return;
        }
        std::thread::sleep(POLL);
    }
}

fn open_stream(
    source: Source,
    device: Option<&str>,
    format: Pcm16Format,
) -> Result<Stream, AudioError> {
    let enumerator = com::enumerator()?;
    let direction = source_direction(source);
    let endpoint = match device {
        Some(id) => com::endpoint(&enumerator, direction, id)?,
        None => com::default_endpoint(&enumerator, direction)?
            .ok_or_else(|| AudioError::no_default(direction))?,
    };
    let failed = com::failed(match source {
        Source::Microphone => "cannot record from the microphone",
        Source::Loopback => "cannot record system audio",
    });
    let client: IAudioClient =
        unsafe { endpoint.device.Activate(CLSCTX_ALL, None) }.map_err(&failed)?;
    let wave = wave_format(format);
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            stream_flags(source),
            BUFFER_HNS,
            0,
            &wave,
            None,
        )
    }
    .map_err(&failed)?;
    let reader = unsafe { client.GetService() }.map_err(&failed)?;
    unsafe { client.Start() }.map_err(&failed)?;
    Ok(Stream {
        device: endpoint.id,
        client,
        reader,
        block_align: usize::from(wave.nBlockAlign),
    })
}

fn source_direction(source: Source) -> Direction {
    match source {
        Source::Microphone => Direction::Input,
        Source::Loopback => Direction::Output,
    }
}

fn source_noun(source: Source) -> &'static str {
    match source {
        Source::Microphone => "microphone",
        Source::Loopback => "system audio",
    }
}

fn stream_flags(source: Source) -> u32 {
    let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    match source {
        Source::Microphone => flags,
        Source::Loopback => flags | AUDCLNT_STREAMFLAGS_LOOPBACK,
    }
}

fn wave_format(format: Pcm16Format) -> WAVEFORMATEX {
    let block_align = format.channels.saturating_mul(BYTES_PER_SAMPLE);
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: format.channels,
        nSamplesPerSec: format.sample_rate,
        nAvgBytesPerSec: format.sample_rate.saturating_mul(u32::from(block_align)),
        nBlockAlign: block_align,
        wBitsPerSample: BYTES_PER_SAMPLE * 8,
        cbSize: 0,
    }
}

impl Stream {
    fn drain(&self, pcm: &mut Vec<u8>) -> windows::core::Result<()> {
        while unsafe { self.reader.GetNextPacketSize() }? > 0 {
            let mut data = std::ptr::null_mut();
            let mut frames = 0;
            let mut flags = 0;
            unsafe {
                self.reader
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
            }
            let length = frames as usize * self.block_align;
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            if silent || data.is_null() {
                pcm.resize(pcm.len() + length, 0);
            } else {
                pcm.extend_from_slice(unsafe { std::slice::from_raw_parts(data, length) });
            }
            unsafe { self.reader.ReleaseBuffer(frames)? };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_reads_the_render_endpoint() {
        let cases = [
            (Source::Microphone, Direction::Input, false),
            (Source::Loopback, Direction::Output, true),
        ];
        for (source, direction, loopback) in cases {
            assert_eq!(source_direction(source), direction, "{source:?}");
            assert_eq!(
                stream_flags(source) & AUDCLNT_STREAMFLAGS_LOOPBACK != 0,
                loopback,
                "{source:?}"
            );
            assert_ne!(
                stream_flags(source) & AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                0,
                "{source:?}"
            );
        }
    }
}
