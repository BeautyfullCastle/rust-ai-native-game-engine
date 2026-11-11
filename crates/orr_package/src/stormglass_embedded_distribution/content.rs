//! Bounded PE32+ header sanity, not authentication or a Windows loader.
fn word(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes.get(offset..offset + 2).ok_or("truncated PE header")?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes.get(offset..offset + 4).ok_or("truncated PE header")?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

// Narrow PE32+ header admission, not a second Windows executable loader.
pub(super) fn validate_pe(bytes: &[u8]) -> Result<(), String> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err("runtime needs a DOS MZ header".into());
    }
    let pe = dword(bytes, 0x3c)? as usize;
    // Bound the offset before addition, including on 32-bit hosts.
    if pe < 64 || pe > bytes.len().saturating_sub(24) || bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("runtime needs a bounded PE signature".into());
    }
    let sections = word(bytes, pe + 6)? as usize;
    let optional_size = word(bytes, pe + 20)? as usize;
    let flags = word(bytes, pe + 22)?;
    if word(bytes, pe + 4)? != 0x8664 || flags & 2 == 0 || flags & 0x2000 != 0 {
        return Err("runtime must be an x64 executable, not a DLL".into());
    }
    let optional = pe + 24;
    let table = optional + optional_size;
    if !(1..=96).contains(&sections)
        || optional_size < 112
        || table + sections * 40 > bytes.len()
        || word(bytes, optional)? != 0x20b
        || !matches!(word(bytes, optional + 68)?, 2 | 3)
    {
        return Err("runtime needs bounded PE32+ Windows application headers".into());
    }
    let entry = dword(bytes, optional + 16)? as u64;
    let headers = dword(bytes, optional + 60)? as usize;
    if headers < table + sections * 40 || headers > bytes.len() {
        return Err("runtime PE header size is invalid".into());
    }
    let mut executable_entry = false;
    for index in 0..sections {
        let section = table + index * 40;
        let virtual_address = dword(bytes, section + 12)? as u64;
        let raw_size = dword(bytes, section + 16)? as u64;
        let raw_start = dword(bytes, section + 20)? as u64;
        if raw_size != 0
            && (raw_start < headers as u64 || raw_start + raw_size > bytes.len() as u64)
        {
            return Err("runtime PE section exceeds snapshot bounds".into());
        }
        if entry >= virtual_address
            && entry < virtual_address + raw_size
            && dword(bytes, section + 36)? & 0x2000_0000 != 0
        {
            executable_entry = true;
        }
    }
    if !executable_entry {
        return Err("runtime PE entry point needs an executable file-backed section".into());
    }
    Ok(())
}
