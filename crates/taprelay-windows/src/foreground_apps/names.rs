//! Friendly process names from executable version resources; never used as identity.
use windows::{
    Win32::{
        Globalization::GetUserDefaultUILanguage,
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
    },
    core::PCWSTR,
};

pub fn executable_name(path: &str) -> Option<String> {
    if !taprelay_core::foreground_app::valid_executable(path) {
        return None;
    }
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let data = unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }
        // Keep the native resource buffer DWORD-aligned.
        let mut data = vec![0u32; (size as usize).div_ceil(4)];
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, data.as_mut_ptr().cast()).ok()?;
        data
    };
    resource_name(&data, unsafe { GetUserDefaultUILanguage() })
}

fn resource_name(data: &[u32], language: u16) -> Option<String> {
    let mut translations = query(data, r"\VarFileInfo\Translation", 1)
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pair| {
            (
                u16::from_le_bytes([pair[0], pair[1]]),
                u16::from_le_bytes([pair[2], pair[3]]),
            )
        })
        .collect::<Vec<_>>();
    translations.sort_by_key(|(id, _)| *id != language);
    // Some executables omit or misreport Translation. These English codepages
    // are also tried by .NET FileVersionInfo after the advertised translations.
    for fallback in [(0x0409, 0x04b0), (0x0409, 0x04e4), (0x0409, 0)] {
        if !translations.contains(&fallback) {
            translations.push(fallback);
        }
    }
    for field in ["FileDescription", "ProductName"] {
        for (language, codepage) in &translations {
            let key = format!(r"\StringFileInfo\{language:04x}{codepage:04x}\{field}");
            if let Some(name) = query(data, &key, 2).and_then(decode_name) {
                return Some(name);
            }
        }
    }
    None
}

fn query<'a>(data: &'a [u32], key: &str, unit_bytes: usize) -> Option<&'a [u8]> {
    let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let mut value = std::ptr::null_mut();
    let mut length = 0;
    unsafe {
        if !VerQueryValueW(
            data.as_ptr().cast(),
            PCWSTR(key.as_ptr()),
            &mut value,
            &mut length,
        )
        .as_bool()
            || value.is_null()
        {
            return None;
        }
        // Strings report UTF-16 units; translation arrays report bytes.
        let bytes = (length as usize).checked_mul(unit_bytes)?;
        let offset = (value as usize).checked_sub(data.as_ptr() as usize)?;
        if offset.checked_add(bytes)? > std::mem::size_of_val(data) {
            return None;
        }
        Some(std::slice::from_raw_parts(value.cast(), bytes))
    }
}

fn decode_name(bytes: &[u8]) -> Option<String> {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    let name = String::from_utf16(&units).ok()?.trim().to_owned();
    (!name.is_empty() && !name.chars().any(char::is_control)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(value: &str) -> Vec<u8> {
        value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    // Minimal native VERSIONINFO blocks; tests exercise VerQueryValueW itself.
    fn block(key: &str, value_len: u16, kind: u16, value: &[u8], children: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 6];
        bytes.extend(utf16(key));
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes.extend(value);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        bytes.extend(children);
        let length = bytes.len() as u16;
        bytes[..2].copy_from_slice(&length.to_le_bytes());
        bytes[2..4].copy_from_slice(&value_len.to_le_bytes());
        bytes[4..6].copy_from_slice(&kind.to_le_bytes());
        bytes
    }

    fn resource(entries: &[(&str, &str, &str)], translation: Option<(u16, u16)>) -> Vec<u32> {
        let mut tables = Vec::new();
        for (codepage, field, name) in entries {
            let value = utf16(name);
            let entry = block(field, (value.len() / 2) as u16, 1, &value, &[]);
            tables.extend(block(codepage, 0, 1, &[], &entry));
        }
        let mut children = block("StringFileInfo", 0, 1, &[], &tables);
        if let Some((language, codepage)) = translation {
            let value = [language.to_le_bytes(), codepage.to_le_bytes()].concat();
            let entry = block("Translation", 4, 0, &value, &[]);
            children.extend(block("VarFileInfo", 0, 1, &[], &entry));
        }
        block("VS_VERSION_INFO", 0, 0, &[], &children)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| u32::from_le_bytes(*bytes))
            .collect()
    }

    #[test]
    fn missing_or_incorrect_translation_falls_back_to_common_codepages() {
        for codepage in ["040904b0", "040904e4", "04090000"] {
            for translation in [None, Some((0x0411, 0x04b0))] {
                let data = resource(
                    &[(codepage, "FileDescription", "Fallback app")],
                    translation,
                );
                assert_eq!(
                    resource_name(&data, 0x0804).as_deref(),
                    Some("Fallback app")
                );
            }
        }
    }

    #[test]
    fn localized_description_wins_and_product_name_is_a_fallback() {
        let data = resource(
            &[
                ("080404b0", "FileDescription", "本地名称"),
                ("040904b0", "FileDescription", "English name"),
            ],
            Some((0x0804, 0x04b0)),
        );
        assert_eq!(resource_name(&data, 0x0804).as_deref(), Some("本地名称"));
        let data = resource(&[("040904b0", "ProductName", "Product name")], None);
        assert_eq!(
            resource_name(&data, 0x0804).as_deref(),
            Some("Product name")
        );
        let data = resource(&[("040904b0", "FileDescription", " \n ")], None);
        assert_eq!(resource_name(&data, 0x0804), None);
    }

    #[test]
    fn reads_system_executable_name_and_handles_missing_metadata() {
        let path =
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("explorer.exe");
        assert!(executable_name(&path.to_string_lossy()).is_some());
        assert!(executable_name(r"C:\missing-taprelay-test\missing.exe").is_none());
        assert!(executable_name("invalid").is_none());
        assert!(executable_name(&std::env::current_exe().unwrap().to_string_lossy()).is_none());
    }
}
