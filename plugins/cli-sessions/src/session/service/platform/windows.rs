use std::collections::{HashMap, HashSet};
use std::mem::offset_of;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

use super::super::ProcessSnapshot;

pub(in super::super) fn process_snapshot() -> Option<ProcessSnapshot> {
    let processes = qol_process::processes().ok()?;
    if processes.is_empty() {
        return None;
    }
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for process in processes {
        let (Ok(pid), Ok(parent)) = (i32::try_from(process.pid), i32::try_from(process.parent))
        else {
            continue;
        };
        children.entry(parent).or_default().push(pid);
    }
    Some(ProcessSnapshot {
        listeners: listening_pids()?,
        children,
    })
}

fn listening_pids() -> Option<HashSet<i32>> {
    let ipv4 =
        listener_rows::<MIB_TCPROW_OWNER_PID>(AF_INET, offset_of!(MIB_TCPTABLE_OWNER_PID, table))?
            .into_iter()
            .map(|row| row.dwOwningPid);
    let ipv6 = listener_rows::<MIB_TCP6ROW_OWNER_PID>(
        AF_INET6,
        offset_of!(MIB_TCP6TABLE_OWNER_PID, table),
    )?
    .into_iter()
    .map(|row| row.dwOwningPid);
    Some(
        ipv4.chain(ipv6)
            .filter_map(|pid| i32::try_from(pid).ok())
            .collect(),
    )
}

fn listener_rows<Row: Copy>(family: u16, rows_offset: usize) -> Option<Vec<Row>> {
    let family = u32::from(family);
    let mut size = 0u32;
    let probe = unsafe {
        GetExtendedTcpTable(
            null_mut(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if probe != ERROR_INSUFFICIENT_BUFFER && probe != NO_ERROR {
        return None;
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8) + 1];
    let mut size = (buffer.len() * 8) as u32;
    let status = unsafe {
        GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            0,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != NO_ERROR {
        return None;
    }
    let base = buffer.as_ptr().cast::<u8>();
    let count = unsafe { base.cast::<u32>().read_unaligned() } as usize;
    let capacity = (buffer.len() * 8).saturating_sub(rows_offset) / std::mem::size_of::<Row>();
    Some(
        (0..count.min(capacity))
            .map(|index| unsafe {
                base.add(rows_offset + index * std::mem::size_of::<Row>())
                    .cast::<Row>()
                    .read_unaligned()
            })
            .collect(),
    )
}
