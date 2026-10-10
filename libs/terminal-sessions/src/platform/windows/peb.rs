use std::ffi::c_void;

use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};

const PEB_PROCESS_PARAMETERS: usize = 0x20;
const PARAMETERS_CURRENT_DIRECTORY: usize = 0x38;
const PARAMETERS_COMMAND_LINE: usize = 0x70;
const MAX_STRING_BYTES: usize = 64 * 1024;

pub(super) struct ProcessStrings {
    pub(super) cwd: Option<String>,
    pub(super) command_line: Option<String>,
}

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Process {
    fn open(pid: u32) -> Option<Process> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        (!handle.is_null()).then_some(Process(handle))
    }

    fn read<const N: usize>(&self, address: usize) -> Option<[u8; N]> {
        let mut buffer = [0u8; N];
        self.read_into(address, &mut buffer).then_some(buffer)
    }

    fn read_into(&self, address: usize, buffer: &mut [u8]) -> bool {
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                self.0,
                address as *const c_void,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut read,
            )
        };
        ok != 0 && read == buffer.len()
    }

    fn parameters(&self) -> Option<usize> {
        let mut info: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let mut length = 0u32;
        let status = unsafe {
            NtQueryInformationProcess(
                self.0,
                ProcessBasicInformation,
                (&mut info as *mut PROCESS_BASIC_INFORMATION).cast(),
                std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                &mut length,
            )
        };
        if status < 0 || info.PebBaseAddress.is_null() {
            return None;
        }
        let peb = info.PebBaseAddress as usize;
        let pointer = self.read::<8>(peb + PEB_PROCESS_PARAMETERS)?;
        usize::try_from(u64::from_le_bytes(pointer))
            .ok()
            .filter(|address| *address != 0)
    }

    fn unicode_string(&self, address: usize) -> Option<String> {
        let header = self.read::<16>(address)?;
        let length = usize::from(u16::from_le_bytes([header[0], header[1]]));
        let buffer = usize::try_from(u64::from_le_bytes(header[8..16].try_into().ok()?)).ok()?;
        if length == 0 || buffer == 0 || length > MAX_STRING_BYTES {
            return None;
        }
        let mut bytes = vec![0u8; length];
        if !self.read_into(buffer, &mut bytes) {
            return None;
        }
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Some(String::from_utf16_lossy(&units))
    }
}

pub(super) fn process_strings(pid: i32) -> ProcessStrings {
    let strings = u32::try_from(pid)
        .ok()
        .and_then(Process::open)
        .and_then(|process| {
            let parameters = process.parameters()?;
            Some(ProcessStrings {
                cwd: process.unicode_string(parameters + PARAMETERS_CURRENT_DIRECTORY),
                command_line: process.unicode_string(parameters + PARAMETERS_COMMAND_LINE),
            })
        });
    strings.unwrap_or(ProcessStrings {
        cwd: None,
        command_line: None,
    })
}
