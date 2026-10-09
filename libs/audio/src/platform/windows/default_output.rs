use std::ffi::c_void;

use windows::core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, ERole, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use crate::devices::{Direction, Identity};
use crate::AudioError;

use super::com;

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const ROLES: [ERole; 3] = [eConsole, eMultimedia, eCommunications];

#[repr(transparent)]
#[derive(Clone)]
struct PolicyConfig(IUnknown);

#[repr(C)]
struct PolicyConfigVtbl {
    base: IUnknown_Vtbl,
    before_set_default_endpoint: [usize; 10],
    set_default_endpoint: unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT,
}

unsafe impl Interface for PolicyConfig {
    type Vtable = PolicyConfigVtbl;
    const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
}

pub(crate) fn effective_default(direction: Direction) -> Result<Option<String>, AudioError> {
    com::with_enumerator(|enumerator| {
        Ok(com::default_endpoint(enumerator, direction)?.map(|endpoint| endpoint.id))
    })
}

pub(crate) fn set_default_output(
    direction: Direction,
    output: &Identity,
) -> Result<(), AudioError> {
    com::with_enumerator(|enumerator| {
        let present = com::endpoints(enumerator, direction, DEVICE_STATE_ACTIVE)?
            .iter()
            .any(|endpoint| endpoint.id == output.as_str());
        if !present {
            return Err(AudioError::not_present(direction, output));
        }
        let policy: PolicyConfig =
            unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }.map_err(
                com::failed("cannot reach the Windows default device policy"),
            )?;
        let id = output
            .as_str()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();
        for role in ROLES {
            unsafe {
                (policy.vtable().set_default_endpoint)(policy.as_raw(), PCWSTR(id.as_ptr()), role)
            }
            .ok()
            .map_err(com::failed("Windows refused the default device"))?;
        }
        Ok(())
    })
}
