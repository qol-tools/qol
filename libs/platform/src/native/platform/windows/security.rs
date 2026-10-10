use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{addr_of_mut, null, null_mut};

use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SetSecurityInfo, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    AddAccessAllowedAceEx, EqualSid, GetAce, GetLengthSid, GetTokenInformation, InitializeAcl,
    TokenOwner, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_REVISION,
    CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    TOKEN_INFORMATION_CLASS, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub struct TokenSid(Vec<u64>);

impl TokenSid {
    pub fn current_user() -> io::Result<TokenSid> {
        Self::query(unsafe { GetCurrentProcess() }, TokenUser)
    }

    pub fn current_owner() -> io::Result<TokenSid> {
        Self::query(unsafe { GetCurrentProcess() }, TokenOwner)
    }

    pub fn user_of(process: HANDLE) -> io::Result<TokenSid> {
        Self::query(process, TokenUser)
    }

    pub fn same_as(&self, other: &TokenSid) -> bool {
        self.is(other.sid())
    }

    fn is(&self, sid: PSID) -> bool {
        unsafe { EqualSid(self.sid(), sid) != 0 }
    }

    fn sid(&self) -> PSID {
        unsafe { *self.0.as_ptr().cast::<PSID>() }
    }

    fn query(process: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<TokenSid> {
        let mut raw = null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut needed = 0u32;
        unsafe { GetTokenInformation(token.as_raw_handle(), class, null_mut(), 0, &mut needed) };
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let ok = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                class,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TokenSid(buffer))
    }
}

pub fn is_owner_only(file: &File) -> io::Result<bool> {
    let user = TokenSid::current_user()?;
    let owner = TokenSid::current_owner()?;
    let ours = |sid: PSID| user.is(sid) || owner.is(sid);
    let security = Security::of(file)?;
    if !ours(security.owner) || security.dacl.is_null() {
        return Ok(false);
    }
    let count = unsafe { (*security.dacl).AceCount };
    for index in 0..u32::from(count) {
        let mut ace = null_mut();
        if unsafe { GetAce(security.dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let kind = u32::from(unsafe { (*ace.cast::<ACE_HEADER>()).AceType });
        if kind == ACCESS_DENIED_ACE_TYPE {
            continue;
        }
        if kind != ACCESS_ALLOWED_ACE_TYPE {
            return Ok(false);
        }
        let sid = unsafe { addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart) };
        if !ours(sid.cast()) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn restrict_to_owner(directory: &File) -> io::Result<()> {
    let user = TokenSid::current_user()?;
    let sid_length = unsafe { GetLengthSid(user.sid()) } as usize;
    let length = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_length;
    let mut buffer = vec![0u64; length.div_ceil(8)];
    let acl = buffer.as_mut_ptr().cast::<ACL>();
    if unsafe { InitializeAcl(acl, (buffer.len() * 8) as u32, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let inherit = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;
    if unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, inherit, FILE_ALL_ACCESS, user.sid()) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let status = unsafe {
        SetSecurityInfo(
            directory.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

struct Security {
    owner: PSID,
    dacl: *mut ACL,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl Security {
    fn of(file: &File) -> io::Result<Security> {
        let mut owner = null_mut();
        let mut dacl = null_mut();
        let mut descriptor = null_mut();
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(Security {
            owner,
            dacl,
            descriptor,
        })
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor) };
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::fs::OpenOptionsExt;
    use std::process::Command;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, READ_CONTROL, WRITE_DAC,
    };

    use super::*;

    fn open_directory(path: &std::path::Path, access: u32) -> File {
        std::fs::OpenOptions::new()
            .access_mode(access)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .unwrap()
    }

    #[test]
    fn the_current_user_matches_itself_and_its_own_process() {
        let user = TokenSid::current_user().unwrap();
        let again = TokenSid::user_of(unsafe { GetCurrentProcess() }).unwrap();
        assert!(user.same_as(&again));
    }

    #[test]
    fn a_restricted_directory_and_its_new_files_are_owner_only() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("private");
        std::fs::create_dir(&root).unwrap();
        assert!(!is_owner_only(&open_directory(&root, READ_CONTROL)).unwrap());
        restrict_to_owner(&open_directory(&root, READ_CONTROL | WRITE_DAC)).unwrap();
        assert!(is_owner_only(&open_directory(&root, READ_CONTROL)).unwrap());
        let file = root.join("state.json");
        std::fs::write(&file, b"{}").unwrap();
        assert!(is_owner_only(&File::open(&file).unwrap()).unwrap());
    }

    #[test]
    fn an_extra_grant_is_not_owner_only() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("private");
        std::fs::create_dir(&root).unwrap();
        restrict_to_owner(&open_directory(&root, READ_CONTROL | WRITE_DAC)).unwrap();
        let file = root.join("state.json");
        std::fs::write(&file, b"{}").unwrap();
        let cases = [(&root, "(OI)(CI)R"), (&file, "R")];
        for (path, rights) in cases {
            let granted = Command::new("icacls")
                .arg(path)
                .arg("/grant")
                .arg(format!("*S-1-1-0:{rights}"))
                .output()
                .unwrap();
            assert!(granted.status.success(), "{}", path.display());
        }
        assert!(!is_owner_only(&open_directory(&root, READ_CONTROL)).unwrap());
        assert!(!is_owner_only(&File::open(&file).unwrap()).unwrap());
    }
}
