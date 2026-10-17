use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf, Prefix},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[path = "../windows_room/platform.rs"]
mod platform;

const MAX_PATH_BYTES: usize = 4096;
const MAX_PATH_UNITS: usize = 32_760;
const MAX_COMPONENTS: usize = 64;
const MAX_COMPONENT_UNITS: usize = 255;
const MAX_PAYLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_PE_BYTES: usize = 512 * 1024 * 1024;
const MAX_PE_SECTIONS: usize = 96;

pub(super) fn checked_path(path: &Path, expect_dir: bool) -> Result<PathBuf, String> {
    let text = path.to_str().ok_or("path must be Unicode")?;
    if text.len() > MAX_PATH_BYTES
        || text.contains('\0')
        || text.chars().any(char::is_control)
        || text
            .split(['\\', '/'])
            .any(|part| part == "." || part == "..")
    {
        return Err("path is too long or contains a control/traversal component".into());
    }
    let components: Vec<_> = path.components().collect();
    if components.len() < 3 || components.len() > MAX_COMPONENTS {
        return Err("path must be a bounded absolute Windows disk path".into());
    }
    let prefix = match components.first() {
        Some(Component::Prefix(prefix)) => prefix.kind(),
        _ => return Err("path must use a Windows disk prefix".into()),
    };
    if !matches!(prefix, Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        || !matches!(components.get(1), Some(Component::RootDir))
        || !components[2..]
            .iter()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("path must be an absolute disk path without traversal".into());
    }
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::Normal(value) => {
                let value = value.to_str().ok_or("path component must be Unicode")?;
                validate_component(value)?;
                current.push(value);
            }
            _ => return Err("path contains a non-portable component".into()),
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
        if platform::is_directory(&metadata) {
            if index + 1 == components.len() && !expect_dir {
                return Err("path must name a regular file".into());
            }
            continue;
        }
        if index + 1 != components.len() || expect_dir || !platform::is_regular(&metadata) {
            return Err("path must contain only existing non-reparse components".into());
        }
    }
    if !expect_dir {
        drop(platform::open_read(path)?);
    }
    let canonical = fs::canonicalize(path).map_err(|error| error.to_string())?;
    validate_canonical_length(&canonical)?;
    Ok(canonical)
}

pub(super) fn destination(
    output: &Path,
    project_root: &Path,
    runtime: &Path,
) -> Result<PathBuf, String> {
    let project_root = checked_path(project_root, true)?;
    let runtime = checked_path(runtime, false)?;
    let output = platform::destination(output)?;
    if paths_overlap(&output, &project_root) || paths_overlap(&output, &runtime) {
        return Err("output must not overlap the source project or runtime".into());
    }
    Ok(output)
}

pub(super) fn write_payload(stage: &Path, relative: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        return Err("payload exceeds the 512 MiB limit".into());
    }
    let components = portable_relative_components(relative)?;
    let stage = checked_path(stage, true)?;
    let mut current = stage.clone();
    for component in &components[..components.len() - 1] {
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&current).map_err(|error| error.to_string())?;
                if !platform::is_directory(&metadata) {
                    return Err("payload parent must be a non-reparse directory".into());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    current.push(components.last().ok_or("payload path cannot be empty")?);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(current)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())
}

pub(super) fn validate_pe(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 64 || bytes.len() > MAX_PE_BYTES || bytes.get(..2) != Some(b"MZ") {
        return Err("runtime is not a bounded PE executable".into());
    }
    let pe_offset = usize::try_from(read_u32(bytes, 0x3c)?).map_err(|_| "invalid PE offset")?;
    let file_header = pe_offset.checked_add(4).ok_or("invalid PE offset")?;
    if bytes.get(pe_offset..file_header) != Some(b"PE\0\0") {
        return Err("runtime has no PE signature".into());
    }
    let machine = read_u16(bytes, file_header)?;
    let section_count = usize::from(read_u16(bytes, file_header + 2)?);
    let optional_size = usize::from(read_u16(bytes, file_header + 16)?);
    let characteristics = read_u16(bytes, file_header + 18)?;
    if machine != 0x8664
        || section_count == 0
        || section_count > MAX_PE_SECTIONS
        || characteristics & 0x0002 == 0
        || characteristics & 0x2000 != 0
    {
        return Err("runtime must be an AMD64 executable and not a DLL".into());
    }
    let optional = file_header
        .checked_add(20)
        .ok_or("invalid PE header size")?;
    if optional_size < 112 || read_u16(bytes, optional)? != 0x20b {
        return Err("runtime must use the PE32+ optional header".into());
    }
    let entry_point = read_u32(bytes, optional + 16)?;
    let section_alignment = read_u32(bytes, optional + 32)?;
    let file_alignment = read_u32(bytes, optional + 36)?;
    let image_size = read_u32(bytes, optional + 56)?;
    let headers_size = read_u32(bytes, optional + 60)?;
    let directory_count = read_u32(bytes, optional + 108)? as usize;
    if entry_point == 0
        || !is_power_of_two(section_alignment)
        || !is_power_of_two(file_alignment)
        || section_alignment < file_alignment
        || !(512..=65_536).contains(&file_alignment)
        || image_size == 0
        || image_size > 2_147_483_648
        || headers_size == 0
        || headers_size as usize > bytes.len()
        || headers_size % file_alignment != 0
        || image_size % section_alignment != 0
        || directory_count > optional_size.saturating_sub(112) / 8
    {
        return Err("runtime PE alignment or image bounds are invalid".into());
    }
    let section_table = optional
        .checked_add(optional_size)
        .ok_or("invalid PE optional header size")?;
    let section_table_end = section_table
        .checked_add(
            section_count
                .checked_mul(40)
                .ok_or("invalid section count")?,
        )
        .ok_or("invalid PE section table")?;
    if section_table_end > bytes.len() || section_table_end > headers_size as usize {
        return Err("runtime PE section table is truncated".into());
    }
    let mut raw_ranges = Vec::with_capacity(section_count);
    let mut virtual_ranges = Vec::with_capacity(section_count);
    let mut executable_entry = false;
    for section_index in 0..section_count {
        let section = section_table + section_index * 40;
        let virtual_size = read_u32(bytes, section + 8)?;
        let virtual_address = read_u32(bytes, section + 12)?;
        let raw_size = read_u32(bytes, section + 16)?;
        let raw_pointer = read_u32(bytes, section + 20)?;
        let flags = read_u32(bytes, section + 36)?;
        if virtual_address % section_alignment != 0 {
            return Err("runtime PE section alignment is invalid".into());
        }
        let mapped_size = virtual_size.max(raw_size);
        let virtual_end = virtual_address
            .checked_add(mapped_size)
            .ok_or("runtime PE virtual section overflows")?;
        if virtual_address < headers_size || virtual_end > image_size || mapped_size == 0 {
            return Err("runtime PE virtual section is outside the image".into());
        }
        virtual_ranges.push((virtual_address, virtual_end));
        if raw_size != 0 {
            let raw_end = raw_pointer
                .checked_add(raw_size)
                .ok_or("runtime PE raw section overflows")?;
            if raw_pointer < headers_size
                || raw_pointer % file_alignment != 0
                || raw_size % file_alignment != 0
                || raw_end as usize > bytes.len()
            {
                return Err("runtime PE raw section is outside the file".into());
            }
            raw_ranges.push((raw_pointer, raw_end));
        } else if raw_pointer != 0 {
            return Err("runtime PE empty raw section has a nonzero offset".into());
        }
        if flags & 0x2000_0000 != 0 && entry_point >= virtual_address && entry_point < virtual_end {
            executable_entry = true;
        }
    }
    if !executable_entry || ranges_overlap(&mut raw_ranges) || ranges_overlap(&mut virtual_ranges) {
        return Err("runtime PE section layout or entry point is invalid".into());
    }
    Ok(())
}

pub(super) fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    max_output: usize,
) -> Result<Vec<u8>, String> {
    if max_output > MAX_COMMAND_OUTPUT_BYTES {
        return Err("command output cap exceeds 1 MiB".into());
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("command timeout is too large")?;
    let job = JobObject::new()?;
    use std::os::windows::process::CommandExt;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    if let Err(error) = job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().ok_or("child stdout was not captured")?;
    let stderr = child.stderr.take().ok_or("child stderr was not captured")?;
    let stdout_overflow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stderr_overflow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let capture_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stdout_reader = capture_pipe(
        stdout,
        max_output,
        stdout_overflow.clone(),
        capture_stop.clone(),
    );
    let stderr_reader = capture_pipe(
        stderr,
        max_output,
        stderr_overflow.clone(),
        capture_stop.clone(),
    );
    let status = loop {
        if stdout_overflow.load(std::sync::atomic::Ordering::Relaxed)
            || stderr_overflow.load(std::sync::atomic::Ordering::Relaxed)
        {
            let _ = job.terminate(1);
            let _ = child.kill();
            let _ = child.wait();
            drop(job);
            capture_stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let stdout = join_capture(stdout_reader)?;
            let stderr = join_capture(stderr_reader)?;
            return Err(format!(
                "command output exceeded {max_output} bytes; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            ));
        }
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = job.terminate(1);
            let _ = child.kill();
            let _ = child.wait();
            drop(job);
            capture_stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let stdout = join_capture(stdout_reader)?;
            let stderr = join_capture(stderr_reader)?;
            return Err(format!(
                "command timed out after {timeout:?}; stdout: {}; stderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(job);
    let drain_deadline = Instant::now() + Duration::from_millis(100);
    while (!stdout_reader.is_finished() || !stderr_reader.is_finished())
        && Instant::now() < drain_deadline
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    capture_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let stdout = join_capture(stdout_reader)?;
    let stderr = join_capture(stderr_reader)?;
    if stdout.len() > max_output || stderr.len() > max_output {
        return Err(format!("command output exceeded {max_output} bytes"));
    }
    if !status.success() {
        return Err(format!(
            "command exited with {status}; stdout: {}; stderr: {}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        ));
    }
    Ok(stdout)
}

pub(super) fn publish(stage: &Path, output: &Path) -> Result<(), String> {
    let stage = checked_path(stage, true)?;
    platform::publish(&stage, output)
}

fn validate_component(component: &str) -> Result<(), String> {
    let units = component.encode_utf16().count();
    if component.is_empty()
        || units > MAX_COMPONENT_UNITS
        || component.ends_with(['.', ' '])
        || component.contains(':')
        || component.chars().any(char::is_control)
        || reserved_device_name(component)
    {
        return Err("path contains a reserved or non-portable component".into());
    }
    Ok(())
}

fn reserved_device_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' ']);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        let Some(suffix) = upper.strip_prefix(prefix) else {
            return false;
        };
        suffix.chars().count() == 1
            && suffix
                .chars()
                .next()
                .is_some_and(|character| matches!(character, '1'..='9' | '¹' | '²' | '³'))
    })
}

fn validate_canonical_length(path: &Path) -> Result<(), String> {
    let text = path.to_str().ok_or("canonical path must be Unicode")?;
    if text.len() > MAX_PATH_BYTES || text.encode_utf16().count() >= MAX_PATH_UNITS {
        return Err("canonical path exceeds Windows path limits".into());
    }
    Ok(())
}

fn portable_relative_components(relative: &str) -> Result<Vec<&str>, String> {
    if relative.is_empty()
        || relative.len() > MAX_PATH_BYTES
        || relative.starts_with('/')
        || relative.ends_with('/')
        || relative.contains('\\')
        || relative.contains('\0')
        || relative.chars().any(char::is_control)
    {
        return Err("payload path must be a bounded portable relative path".into());
    }
    let components: Vec<_> = relative.split('/').collect();
    if components.len() > MAX_COMPONENTS {
        return Err("payload path has too many components".into());
    }
    for component in &components {
        validate_component(component)?;
    }
    Ok(components)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left = comparison_path(left);
    let right = comparison_path(right);
    is_within(&left, &right) || is_within(&right, &left)
}

fn comparison_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn is_within(candidate: &str, parent: &str) -> bool {
    candidate == parent
        || candidate
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset.checked_add(2).ok_or("invalid PE offset")?;
    let field = bytes.get(offset..end).ok_or("truncated PE field")?;
    Ok(u16::from_le_bytes([field[0], field[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset.checked_add(4).ok_or("invalid PE offset")?;
    let field = bytes.get(offset..end).ok_or("truncated PE field")?;
    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn is_power_of_two(value: u32) -> bool {
    value != 0 && value.is_power_of_two()
}

fn ranges_overlap(ranges: &mut [(u32, u32)]) -> bool {
    ranges.sort_unstable_by_key(|range| range.0);
    ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
}

fn capture_pipe<R: std::io::Read + std::os::windows::io::AsRawHandle + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<Result<Vec<u8>, String>> {
    std::thread::spawn(move || {
        let capture_limit = limit.saturating_add(1);
        let mut output = Vec::with_capacity(capture_limit.min(8192));
        let mut buffer = [0_u8; 4096];
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let remaining = capture_limit.saturating_sub(output.len());
            if remaining == 0 {
                overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            let mut available = 0_u32;
            let peeked = unsafe {
                PeekNamedPipe(
                    reader.as_raw_handle().cast(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if peeked == 0 {
                let error = std::io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(109 | 232)) {
                    break;
                }
                overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(error.to_string());
            }
            if available == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            let read_limit = remaining.min(buffer.len()).min(available as usize);
            let read_count = match reader.read(&mut buffer[..read_limit]) {
                Ok(read_count) => read_count,
                Err(error) => {
                    overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Err(error.to_string());
                }
            };
            if read_count == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..read_count]);
            if output.len() > limit {
                overflow.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
        Ok(output)
    })
}

fn join_capture(
    reader: std::thread::JoinHandle<Result<Vec<u8>, String>>,
) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| "command output reader panicked".to_string())?
}

struct JobObject(*mut std::ffi::c_void);

impl JobObject {
    fn new() -> Result<Self, String> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut limits: ExtendedLimitInformation = unsafe { std::mem::zeroed() };
        limits.basic.limit_flags = 0x0000_2000;
        let success = unsafe {
            SetInformationJobObject(
                handle,
                9,
                (&mut limits as *mut ExtendedLimitInformation).cast(),
                std::mem::size_of::<ExtendedLimitInformation>() as u32,
            )
        };
        if success == 0 {
            let error = std::io::Error::last_os_error().to_string();
            unsafe { CloseHandle(handle) };
            return Err(error);
        }
        Ok(Self(handle))
    }

    fn assign(&self, child: &std::process::Child) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        let success = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle().cast()) };
        if success == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    fn terminate(&self, exit_code: u32) -> Result<(), String> {
        let success = unsafe { TerminateJobObject(self.0, exit_code) };
        if success == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

#[repr(C)]
struct BasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

#[repr(C)]
struct ExtendedLimitInformation {
    basic: BasicLimitInformation,
    io: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(
        attributes: *mut std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn PeekNamedPipe(
        pipe: *mut std::ffi::c_void,
        buffer: *mut std::ffi::c_void,
        buffer_size: u32,
        bytes_read: *mut u32,
        total_available: *mut u32,
        bytes_left_this_message: *mut u32,
    ) -> i32;
    fn SetInformationJobObject(
        job: *mut std::ffi::c_void,
        class: i32,
        information: *mut std::ffi::c_void,
        information_length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_pe() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x400];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let file_header = 0x84;
        bytes[file_header..file_header + 2].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes[file_header + 2..file_header + 4].copy_from_slice(&1_u16.to_le_bytes());
        bytes[file_header + 16..file_header + 18].copy_from_slice(&0xf0_u16.to_le_bytes());
        bytes[file_header + 18..file_header + 20].copy_from_slice(&0x0002_u16.to_le_bytes());
        let optional = file_header + 20;
        bytes[optional..optional + 2].copy_from_slice(&0x20b_u16.to_le_bytes());
        bytes[optional + 16..optional + 20].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[optional + 32..optional + 36].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[optional + 36..optional + 40].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[optional + 56..optional + 60].copy_from_slice(&0x2000_u32.to_le_bytes());
        bytes[optional + 60..optional + 64].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[optional + 108..optional + 112].copy_from_slice(&16_u32.to_le_bytes());
        let section = optional + 0xf0;
        bytes[section + 8..section + 12].copy_from_slice(&0x100_u32.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[section + 36..section + 40].copy_from_slice(&0x6000_0020_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn validates_bounded_amd64_pe32_plus_executables() {
        assert!(validate_pe(&valid_pe()).is_ok());
        assert!(validate_pe(b"MZ").is_err());
    }

    #[test]
    fn rejects_dll_wrong_machine_truncation_and_overlapping_sections() {
        let mut dll = valid_pe();
        dll[0x80 + 4 + 18..0x80 + 4 + 20].copy_from_slice(&0x2002_u16.to_le_bytes());
        assert!(validate_pe(&dll).is_err());

        let mut wrong_machine = valid_pe();
        wrong_machine[0x84..0x86].copy_from_slice(&0x14c_u16.to_le_bytes());
        assert!(validate_pe(&wrong_machine).is_err());

        let mut truncated = valid_pe();
        truncated.truncate(0x300);
        assert!(validate_pe(&truncated).is_err());

        let mut overlapping = valid_pe();
        let section = 0x80 + 4 + 20 + 0xf0;
        overlapping[section + 20..section + 24].copy_from_slice(&0x000_u32.to_le_bytes());
        assert!(validate_pe(&overlapping).is_err());
    }

    #[test]
    fn command_timeout_and_output_overflow_are_bounded() {
        use std::time::Instant;

        let started = Instant::now();
        let timeout = run_bounded(
            Command::new("cmd.exe").args(["/C", "ping 127.0.0.1 -n 10 >NUL"]),
            Duration::from_millis(50),
            64,
        );
        assert!(timeout.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));

        let started = Instant::now();
        let overflow = run_bounded(
            Command::new("cmd.exe").args(["/C", "for /L %i in (1,1,10000) do @echo x"]),
            Duration::from_secs(5),
            16,
        );
        assert!(overflow.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn validates_windows_existing_paths_and_no_clobber_payloads() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let existing = root.join("runtime.exe");
        fs::write(&existing, b"runtime").unwrap();
        assert_eq!(checked_path(&root, true).unwrap(), root);
        assert_eq!(checked_path(&existing, false).unwrap(), existing);
        assert!(checked_path(&root.join("missing"), true).is_err());
        assert!(checked_path(Path::new(r"\server\share\file"), false).is_err());

        let stage = root.join("stage");
        fs::create_dir(&stage).unwrap();
        write_payload(&stage, "nested/asset.bin", b"payload").unwrap();
        assert_eq!(
            fs::read(stage.join("nested/asset.bin")).unwrap(),
            b"payload"
        );
        assert!(write_payload(&stage, "nested/asset.bin", b"replacement").is_err());
        assert_eq!(
            fs::read(stage.join("nested/asset.bin")).unwrap(),
            b"payload"
        );
        for relative in ["../escape", "NUL/file", "asset:stream", "/rooted"] {
            assert!(write_payload(&stage, relative, b"bad").is_err());
        }
    }
}
