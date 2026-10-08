use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION,
    CM_NOTIFY_ACTION_DEVICECUSTOMEVENT, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS, HCMNOTIFICATION,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_IO_PENDING, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile, FILE_FLAG_OVERLAPPED};
use windows_sys::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows_sys::Win32::System::IO::{
    CancelIoEx, DeviceIoControl, GetOverlappedResult, OVERLAPPED,
};

const IOCTL_GET_INFORMATION: u32 = 0x0022_2000;
const PORT_INFO_BYTES: usize = 512;
const HOST_CONNECTED_OFFSET: usize = 5;
const HOST_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TRANSFER_POLL_MS: u32 = 50;
const PORT_STATUS_EVENT: GUID = GUID::from_u128(0x2c0f39ac_b156_4237_9c64_8991a18bf35c);
const PORT_STATUS_REASON_OFFSET: usize = 4;
const PORT_STATUS_BYTES: u32 = 8;

pub(super) fn open(path: &Path) -> Result<(BufReader<PortStream>, PortStream)> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OVERLAPPED)
        .open(path)
        .with_context(|| format!("failed to open guest-control device {}", path.display()))?;
    let disconnects = Arc::new(AtomicU64::new(0));
    let notification = register_disconnects(&file, &disconnects)?;
    let mut port = VirtioPort {
        file,
        disconnects,
        session: 0,
        notification,
    };
    while !port
        .host_connected()
        .context("failed to query the guest-control host connection")?
    {
        thread::sleep(HOST_POLL_INTERVAL);
    }
    port.session = port.disconnects.load(Ordering::SeqCst);
    let port = Rc::new(port);
    Ok((
        BufReader::new(PortStream(Rc::clone(&port))),
        PortStream(port),
    ))
}

pub(super) struct PortStream(Rc<VirtioPort>);

impl Read for PortStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for PortStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct VirtioPort {
    file: File,
    disconnects: Arc<AtomicU64>,
    session: u64,
    notification: HCMNOTIFICATION,
}

impl Drop for VirtioPort {
    fn drop(&mut self) {
        unsafe { CM_Unregister_Notification(self.notification) };
    }
}

impl VirtioPort {
    fn session_open(&self) -> bool {
        self.disconnects.load(Ordering::SeqCst) == self.session
            && matches!(self.host_connected(), Ok(true))
    }

    fn handle(&self) -> HANDLE {
        self.file.as_raw_handle()
    }

    fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let event = Event::new()?;
        let mut overlapped = event.overlapped();
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let started = unsafe {
            ReadFile(
                self.handle(),
                buffer.as_mut_ptr(),
                length,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        self.finish(started, &event, &overlapped)
    }

    fn write(&self, buffer: &[u8]) -> io::Result<usize> {
        let event = Event::new()?;
        let mut overlapped = event.overlapped();
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        let started = unsafe {
            WriteFile(
                self.handle(),
                buffer.as_ptr(),
                length,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        self.finish(started, &event, &overlapped)
    }

    fn finish(&self, started: i32, event: &Event, overlapped: &OVERLAPPED) -> io::Result<usize> {
        if started == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
        }
        loop {
            match unsafe { WaitForSingleObject(event.0, TRANSFER_POLL_MS) } {
                WAIT_OBJECT_0 => return self.overlapped_result(overlapped).map(|n| n as usize),
                WAIT_TIMEOUT if self.session_open() => {}
                _ => {
                    unsafe { CancelIoEx(self.handle(), overlapped) };
                    let _ = self.overlapped_result(overlapped);
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "guest-control host disconnected",
                    ));
                }
            }
        }
    }

    fn host_connected(&self) -> io::Result<bool> {
        let event = Event::new()?;
        let mut overlapped = event.overlapped();
        let mut info = [0_u8; PORT_INFO_BYTES];
        let started = unsafe {
            DeviceIoControl(
                self.handle(),
                IOCTL_GET_INFORMATION,
                ptr::null(),
                0,
                info.as_mut_ptr().cast(),
                PORT_INFO_BYTES as u32,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        if started == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                return Err(io::Error::from_raw_os_error(error as i32));
            }
        }
        let transferred = self.overlapped_result(&overlapped)? as usize;
        if transferred <= HOST_CONNECTED_OFFSET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest-control port information is truncated",
            ));
        }
        Ok(info[HOST_CONNECTED_OFFSET] != 0)
    }

    fn overlapped_result(&self, overlapped: &OVERLAPPED) -> io::Result<u32> {
        let mut transferred = 0;
        let ok = unsafe { GetOverlappedResult(self.handle(), overlapped, &mut transferred, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(transferred)
    }
}

fn register_disconnects(file: &File, disconnects: &Arc<AtomicU64>) -> Result<HCMNOTIFICATION> {
    let mut filter: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
    filter.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
    filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE;
    filter.u.DeviceHandle.hTarget = file.as_raw_handle();
    let mut notification = ptr::null_mut();
    let status = unsafe {
        CM_Register_Notification(
            &filter,
            Arc::as_ptr(disconnects).cast(),
            Some(on_port_event),
            &mut notification,
        )
    };
    if status != CR_SUCCESS {
        bail!("failed to watch the guest-control port for host disconnects: CONFIGRET {status}");
    }
    Ok(notification)
}

unsafe extern "system" fn on_port_event(
    _notification: HCMNOTIFICATION,
    context: *const std::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    event: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    if action != CM_NOTIFY_ACTION_DEVICECUSTOMEVENT || event.is_null() {
        return CR_SUCCESS;
    }
    let custom = unsafe { &(*event).u.DeviceHandle };
    let ours = custom.EventGuid.data1 == PORT_STATUS_EVENT.data1
        && custom.EventGuid.data2 == PORT_STATUS_EVENT.data2
        && custom.EventGuid.data3 == PORT_STATUS_EVENT.data3
        && custom.EventGuid.data4 == PORT_STATUS_EVENT.data4;
    if !ours || custom.DataSize < PORT_STATUS_BYTES {
        return CR_SUCCESS;
    }
    let reason = unsafe {
        custom
            .Data
            .as_ptr()
            .add(PORT_STATUS_REASON_OFFSET)
            .cast::<u32>()
            .read_unaligned()
    };
    if reason == 0 {
        let disconnects = unsafe { &*context.cast::<AtomicU64>() };
        disconnects.fetch_add(1, Ordering::SeqCst);
    }
    CR_SUCCESS
}

struct Event(HANDLE);

impl Event {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn overlapped(&self) -> OVERLAPPED {
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = self.0;
        overlapped
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
