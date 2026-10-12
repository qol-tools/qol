use std::ffi::c_void;
use std::path::Path;
use std::ptr::null_mut;

use qol_platform::native::wide::wide_nul;
use qol_platform::utf16::from_le_bytes;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};

const TRANSLATION: &str = r"\VarFileInfo\Translation";
const FALLBACK_LANGUAGES: [(u16, u16); 2] = [(0x0409, 0x04b0), (0x0409, 0x04e4)];

#[derive(Debug, Default)]
pub(super) struct VersionFacts {
    pub(super) description: Option<String>,
    pub(super) company: Option<String>,
    pub(super) product: Option<String>,
    pub(super) version: Option<String>,
}

pub(super) fn read(binary: &Path) -> VersionFacts {
    let Some(block) = version_block(binary) else {
        return VersionFacts::default();
    };
    let languages: Vec<(u16, u16)> = translations(&block)
        .into_iter()
        .chain(FALLBACK_LANGUAGES)
        .collect();
    let value = |name: &str| {
        languages
            .iter()
            .find_map(|language| query_string(&block, &string_path(*language, name)))
    };
    VersionFacts {
        description: value("FileDescription"),
        company: value("CompanyName"),
        product: value("ProductName"),
        version: value("ProductVersion").or_else(|| value("FileVersion")),
    }
}

fn string_path((language, codepage): (u16, u16), name: &str) -> String {
    format!(r"\StringFileInfo\{language:04x}{codepage:04x}\{name}")
}

fn version_block(binary: &Path) -> Option<Vec<u8>> {
    let path = wide_nul(binary);
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), null_mut()) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    let read = unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, block.as_mut_ptr().cast()) };
    (read != 0).then_some(block)
}

fn query<'a>(block: &'a [u8], sub_block: &str, unit_bytes: usize) -> Option<&'a [u8]> {
    let sub_block = wide_nul(sub_block);
    let mut value: *mut c_void = null_mut();
    let mut length = 0u32;
    let found = unsafe {
        VerQueryValueW(
            block.as_ptr().cast(),
            sub_block.as_ptr(),
            &mut value,
            &mut length,
        )
    };
    if found == 0 || value.is_null() || length == 0 {
        return None;
    }
    let start = (value as usize).checked_sub(block.as_ptr() as usize)?;
    let end = start.checked_add((length as usize).checked_mul(unit_bytes)?)?;
    block.get(start..end)
}

fn translations(block: &[u8]) -> Vec<(u16, u16)> {
    query(block, TRANSLATION, 1)
        .map(|raw| {
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|pair| {
                    (
                        u16::from_le_bytes([pair[0], pair[1]]),
                        u16::from_le_bytes([pair[2], pair[3]]),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn query_string(block: &[u8], sub_block: &str) -> Option<String> {
    let text = from_le_bytes(query(block, sub_block, 2)?).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_paths_use_lowercase_hex_language_and_codepage() {
        assert_eq!(
            string_path((0x0409, 0x04b0), "CompanyName"),
            r"\StringFileInfo\040904b0\CompanyName"
        );
    }
}
