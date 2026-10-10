use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    IAudioCaptureClient, IAudioClient, IMMDevice, AUDCLNT_SHAREMODE_SHARED,
};
use windows::Win32::System::Com::{CoTaskMemFree, CLSCTX_ALL};

use crate::devices::Direction;
use crate::AudioError;

use super::com;

const POLL: Duration = Duration::from_millis(40);
const CAPTURE_BUFFER_HNS: i64 = 1_000_000;

pub(crate) struct Meter {
    device: String,
    peak: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Meter {
    pub(crate) fn open(direction: Direction) -> Result<Self, AudioError> {
        let peak = Arc::new(AtomicU32::new(0f32.to_bits()));
        let stop = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) = mpsc::channel();
        let worker_peak = Arc::clone(&peak);
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("qol-audio-meter".to_owned())
            .spawn(move || measure(direction, &opened_tx, &worker_peak, &worker_stop))
            .map_err(|error| {
                AudioError::Operation(format!("cannot start the level meter: {error}"))
            })?;
        let opened = opened_rx.recv().unwrap_or_else(|_| {
            Err(AudioError::Operation(
                "the level meter stopped before it opened".to_owned(),
            ))
        });
        match opened {
            Ok(device) => Ok(Self {
                device,
                peak,
                stop,
                worker: Some(worker),
            }),
            Err(error) => {
                let _ = worker.join();
                Err(error)
            }
        }
    }

    pub(crate) fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub(crate) fn peak_before_volume(&self, percent: u32) -> f32 {
        let _ = percent;
        self.peak()
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Tap {
    device: String,
    meter: IAudioMeterInformation,
    capture: Option<Capture>,
}

struct Capture {
    client: IAudioClient,
    reader: IAudioCaptureClient,
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = unsafe { self.client.Stop() };
    }
}

fn measure(
    direction: Direction,
    opened: &mpsc::Sender<Result<String, AudioError>>,
    peak: &AtomicU32,
    stop: &AtomicBool,
) {
    let _com = match com::apartment() {
        Ok(com) => com,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    let tap = match open_tap(direction) {
        Ok(tap) => tap,
        Err(error) => {
            let _ = opened.send(Err(error));
            return;
        }
    };
    if opened.send(Ok(tap.device.clone())).is_err() {
        return;
    }
    while !stop.load(Ordering::Relaxed) {
        match tap.read() {
            Ok(level) => peak.store(level.to_bits(), Ordering::Relaxed),
            Err(_) => return,
        }
        std::thread::sleep(POLL);
    }
}

fn open_tap(direction: Direction) -> Result<Tap, AudioError> {
    let enumerator = com::enumerator()?;
    let endpoint = com::default_endpoint(&enumerator, direction)?
        .ok_or_else(|| AudioError::no_default(direction))?;
    let meter = unsafe { endpoint.device.Activate(CLSCTX_ALL, None) }
        .map_err(com::failed("cannot open the level meter"))?;
    let capture = match direction {
        Direction::Input => Some(open_capture(&endpoint.device)?),
        Direction::Output => None,
    };
    Ok(Tap {
        device: endpoint.id,
        meter,
        capture,
    })
}

fn open_capture(device: &IMMDevice) -> Result<Capture, AudioError> {
    let failed = com::failed("cannot listen to the microphone");
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(&failed)?;
    let format = unsafe { client.GetMixFormat() }.map_err(&failed)?;
    let initialized = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            0,
            CAPTURE_BUFFER_HNS,
            0,
            format,
            None,
        )
    };
    unsafe { CoTaskMemFree(Some(format as *const c_void)) };
    initialized.map_err(&failed)?;
    let reader = unsafe { client.GetService() }.map_err(&failed)?;
    unsafe { client.Start() }.map_err(&failed)?;
    Ok(Capture { client, reader })
}

impl Tap {
    fn read(&self) -> windows::core::Result<f32> {
        if let Some(capture) = &self.capture {
            capture.drain()?;
        }
        unsafe { self.meter.GetPeakValue() }
    }
}

impl Capture {
    fn drain(&self) -> windows::core::Result<()> {
        while unsafe { self.reader.GetNextPacketSize() }? > 0 {
            let mut data = std::ptr::null_mut();
            let mut frames = 0;
            let mut flags = 0;
            unsafe {
                self.reader
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                self.reader.ReleaseBuffer(frames)?;
            }
        }
        Ok(())
    }
}
