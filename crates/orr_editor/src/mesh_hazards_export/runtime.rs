//! Bounded format inspection, not a signature or a statement of trust.
//!
//! A producer-controlled section can claim capability; it cannot establish that
//! a program is safe. The bundle layer must still require explicit runtime trust
//! and bind the exact inspected bytes to its manifest hash.
use super::RuntimePlatform;

pub(super) const CAPABILITY_MARKER: &[u8] = b"ORR_MESH_HAZARDS_PLAYER_V1";
pub(super) const CAPABILITY_SECTION: &str = ".orrhaz";
const MAX_SECTIONS: usize = 4096;

pub(super) fn validate_runtime(bytes: &[u8]) -> Result<RuntimePlatform, String> {
    if bytes.starts_with(b"MZ") {
        validate_pe(bytes)?;
        Ok(RuntimePlatform::WindowsX86_64)
    } else if bytes.starts_with(b"\x7fELF") {
        validate_elf(bytes)?;
        Ok(RuntimePlatform::LinuxX86_64)
    } else {
        Err("runtime must be a PE32+ or ELF64 executable".into())
    }
}

fn region(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8], String> {
    let end = offset.checked_add(size).ok_or("runtime offset overflow")?;
    bytes
        .get(offset..end)
        .ok_or_else(|| "runtime range is truncated or out of bounds".into())
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        region(bytes, offset, 2)?.try_into().unwrap(),
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        region(bytes, offset, 4)?.try_into().unwrap(),
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        region(bytes, offset, 8)?.try_into().unwrap(),
    ))
}
fn host_size(value: u64) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| "runtime offset exceeds host bounds".into())
}
fn table(bytes: &[u8], offset: usize, count: usize, stride: usize) -> Result<&[u8], String> {
    if count == 0 || count > MAX_SECTIONS {
        return Err("runtime table count is unsupported".into());
    }
    region(
        bytes,
        offset,
        count.checked_mul(stride).ok_or("runtime table overflow")?,
    )
}
fn marker(data: &[u8]) -> Result<(), String> {
    if !data.starts_with(CAPABILITY_MARKER)
        || data[CAPABILITY_MARKER.len()..]
            .iter()
            .any(|&byte| byte != 0)
    {
        return Err(
            "runtime capability section must begin with the exact marker and only zero padding"
                .into(),
        );
    }
    Ok(())
}
fn overlap(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}
fn record_range(
    ranges: &mut Vec<(usize, usize)>,
    offset: usize,
    size: usize,
) -> Result<(), String> {
    if size == 0 {
        return Ok(());
    }
    let range = (
        offset,
        offset.checked_add(size).ok_or("runtime range overflow")?,
    );
    if ranges.iter().any(|&previous| overlap(previous, range)) {
        return Err("runtime sections overlap".into());
    }
    ranges.push(range);
    Ok(())
}

fn validate_pe(bytes: &[u8]) -> Result<(), String> {
    region(bytes, 0, 64)?;
    let pe = u32_at(bytes, 0x3c)? as usize;
    if pe < 64 || region(bytes, pe, 24)?[..4] != *b"PE\0\0" {
        return Err("runtime has an invalid PE signature".into());
    }
    let coff = pe + 4;
    if u16_at(bytes, coff)? != 0x8664 {
        return Err("runtime PE architecture must be AMD64".into());
    }
    let count = u16_at(bytes, coff + 2)? as usize;
    let optional_size = u16_at(bytes, coff + 16)? as usize;
    let characteristics = u16_at(bytes, coff + 18)?;
    if characteristics & 0x0002 == 0 || characteristics & (0x2000 | 0x0100) != 0 {
        return Err("runtime PE must be a 64-bit executable, not a DLL".into());
    }
    let optional_start = pe + 24;
    let optional = region(bytes, optional_start, optional_size)?;
    if optional_size < 112 || u16_at(optional, 0)? != 0x20b {
        return Err("runtime requires a complete PE32+ optional header".into());
    }
    let directory_count = u32_at(optional, 108)? as usize;
    if directory_count > 16 || 112 + directory_count * 8 > optional_size {
        return Err("runtime PE data directory table is invalid".into());
    }
    let section_start = optional_start + optional_size;
    let sections = table(bytes, section_start, count, 40)?;
    let headers_size = u32_at(optional, 60)? as usize;
    let headers_end = section_start + sections.len();
    if headers_size < headers_end {
        return Err("runtime PE sections exceed headers".into());
    }
    region(bytes, 0, headers_size)?;
    let file_alignment = u32_at(optional, 36)? as usize;
    let section_alignment = u32_at(optional, 32)? as u64;
    if !file_alignment.is_power_of_two()
        || !(512..=65536).contains(&file_alignment)
        || !section_alignment.is_power_of_two()
        || section_alignment < file_alignment as u64
        || !headers_size.is_multiple_of(file_alignment)
    {
        return Err("runtime PE alignment is invalid".into());
    }
    let entry = u32_at(optional, 16)? as u64;
    let image_size = u32_at(optional, 56)? as u64;
    if image_size == 0 || !image_size.is_multiple_of(section_alignment) {
        return Err("runtime PE image size is misaligned".into());
    }
    let mut executable_entry = false;
    let mut found = false;
    let mut raw_ranges = vec![(0, headers_size)];
    let mut virtual_ranges = Vec::new();
    for section in sections.as_chunks::<40>().0 {
        let virtual_size = u32_at(section, 8)? as u64;
        let address = u32_at(section, 12)? as u64;
        let raw_size = u32_at(section, 16)? as usize;
        let raw_offset = u32_at(section, 20)? as usize;
        let flags = u32_at(section, 36)?;
        if !address.is_multiple_of(section_alignment)
            || !raw_size.is_multiple_of(file_alignment)
            || (raw_size != 0 && !raw_offset.is_multiple_of(file_alignment))
        {
            return Err("runtime PE section is misaligned".into());
        }
        let virtual_end = address
            .checked_add(virtual_size.max(raw_size as u64))
            .ok_or("runtime PE virtual overflow")?;
        if address < headers_size as u64 || virtual_end > image_size {
            return Err("runtime PE section exceeds image bounds".into());
        }
        record_range(
            &mut virtual_ranges,
            host_size(address)?,
            host_size(virtual_end - address)?,
        )?;
        let raw = region(bytes, raw_offset, raw_size)?;
        record_range(&mut raw_ranges, raw_offset, raw_size)?;
        if entry >= address
            && entry < address + virtual_size.min(raw_size as u64)
            && flags & 0x2000_0000 != 0
        {
            executable_entry = true;
        }
        if &section[..8] == b".orrhaz\0" {
            if found {
                return Err("runtime has duplicate capability sections".into());
            }
            found = true;
            if virtual_size != CAPABILITY_MARKER.len() as u64
                || flags & 0x4000_0040 != 0x4000_0040
                || flags & 0xa000_0080 != 0
            {
                return Err("runtime PE capability section has invalid size or flags".into());
            }
            marker(raw)?;
        }
    }
    if !executable_entry {
        return Err("runtime PE entry point is not executable".into());
    }
    if !found {
        return Err("runtime capability section is missing".into());
    }
    Ok(())
}

fn validate_elf(bytes: &[u8]) -> Result<(), String> {
    let header = region(bytes, 0, 64)?;
    if header[4] != 2
        || header[5] != 1
        || header[6] != 1
        || u16_at(header, 18)? != 62
        || u32_at(header, 20)? != 1
        || !matches!(u16_at(header, 16)?, 2 | 3)
        || u16_at(header, 52)? != 64
    {
        return Err("runtime must be a little-endian ELF64 AMD64 executable".into());
    }
    if u16_at(header, 54)? != 56 || u16_at(header, 58)? != 64 {
        return Err("runtime ELF table entry sizes are invalid".into());
    }
    let ph_offset = host_size(u64_at(header, 32)?)?;
    let sh_offset = host_size(u64_at(header, 40)?)?;
    if ph_offset < 64 || sh_offset < 64 {
        return Err("runtime ELF tables overlap header".into());
    }
    let programs = table(bytes, ph_offset, u16_at(header, 56)? as usize, 56)?;
    let count = u16_at(header, 60)? as usize;
    let sections = table(bytes, sh_offset, count, 64)?;
    let strings_index = u16_at(header, 62)? as usize;
    if strings_index == 0 || strings_index >= count || sections[..64].iter().any(|&v| v != 0) {
        return Err("runtime ELF section-name table or null section is invalid".into());
    }
    let strings_header = &sections[strings_index * 64..(strings_index + 1) * 64];
    if u32_at(strings_header, 4)? != 3 {
        return Err("runtime ELF names require a string table".into());
    }
    let strings = region(
        bytes,
        host_size(u64_at(strings_header, 24)?)?,
        host_size(u64_at(strings_header, 32)?)?,
    )?;
    if strings.first() != Some(&0) || strings.last() != Some(&0) {
        return Err("runtime ELF string table is not terminated".into());
    }
    let mut loads = Vec::new();
    let mut executable_entry = false;
    let entry = u64_at(header, 24)?;
    for program in programs.as_chunks::<56>().0 {
        let offset = host_size(u64_at(program, 8)?)?;
        let file_size = host_size(u64_at(program, 32)?)?;
        region(bytes, offset, file_size)?;
        if u32_at(program, 0)? != 1 {
            continue;
        }
        let address = u64_at(program, 16)?;
        let mem_size = u64_at(program, 40)?;
        let end = address
            .checked_add(mem_size)
            .ok_or("runtime ELF load address overflow")?;
        if file_size as u64 > mem_size {
            return Err("runtime ELF load file size exceeds memory size".into());
        }
        let alignment = u64_at(program, 48)?;
        if alignment > 1
            && (!alignment.is_power_of_two() || address % alignment != offset as u64 % alignment)
        {
            return Err("runtime ELF load alignment is invalid".into());
        }
        let flags = u32_at(program, 4)?;
        if entry >= address && entry < end && entry - address < file_size as u64 && flags & 1 != 0 {
            executable_entry = true;
        }
        loads.push((offset, file_size, address, flags));
    }
    if !executable_entry {
        return Err("runtime ELF entry point is not executable".into());
    }
    let mut found = false;
    let mut ranges = vec![
        (0, 64),
        (ph_offset, ph_offset + programs.len()),
        (sh_offset, sh_offset + sections.len()),
    ];
    if overlap(ranges[1], ranges[2]) {
        return Err("runtime ELF tables overlap".into());
    }
    for section in sections.as_chunks::<64>().0.iter().skip(1) {
        let name_start = u32_at(section, 0)? as usize;
        let suffix = strings
            .get(name_start..)
            .ok_or("runtime ELF section name is out of bounds")?;
        let end = suffix
            .iter()
            .position(|&v| v == 0)
            .ok_or("runtime ELF section name is unterminated")?;
        let name = &suffix[..end];
        let offset = host_size(u64_at(section, 24)?)?;
        let size = host_size(u64_at(section, 32)?)?;
        let kind = u32_at(section, 4)?;
        let alignment = u64_at(section, 48)?;
        let address = u64_at(section, 16)?;
        address
            .checked_add(size as u64)
            .ok_or("runtime ELF section address overflow")?;
        if alignment > 1 && (!alignment.is_power_of_two() || address % alignment != 0) {
            return Err("runtime ELF section alignment is invalid".into());
        }
        if kind != 8 {
            region(bytes, offset, size)?;
            record_range(&mut ranges, offset, size)?;
        }
        if name == CAPABILITY_SECTION.as_bytes() {
            if found {
                return Err("runtime has duplicate capability sections".into());
            }
            found = true;
            let flags = u64_at(section, 8)?;
            let address = u64_at(section, 16)?;
            // Rust may retain #[used] statics with SHF_GNU_RETAIN. Neither
            // accepted flag combination grants write or execute permission.
            if kind != 1 || !matches!(flags, 2 | 0x20_0002) || size != CAPABILITY_MARKER.len() {
                return Err(
                    "runtime ELF capability section has invalid type, flags, or size".into(),
                );
            }
            let mapped = loads.iter().any(|&(start, length, base, flags)| {
                offset >= start
                    && offset - start <= length
                    && size <= length - (offset - start)
                    && base.checked_add((offset - start) as u64) == Some(address)
                    && flags & 4 != 0
                    && flags & 2 == 0
            });
            if !mapped {
                return Err("runtime ELF capability section is not mapped read-only".into());
            }
            marker(region(bytes, offset, size)?)?;
        }
    }
    if !found {
        return Err("runtime capability section is missing".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn valid_test_runtime(platform: RuntimePlatform) -> Vec<u8> {
    tests::fixture(platform)
}
