use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;

const SERVICES_SESSION: u32 = 0;

pub fn interactive_session_id() -> Option<u32> {
    let mut session = 0;
    let ok = unsafe { ProcessIdToSessionId(std::process::id(), &mut session) };
    (ok != 0 && session != SERVICES_SESSION).then_some(session)
}
