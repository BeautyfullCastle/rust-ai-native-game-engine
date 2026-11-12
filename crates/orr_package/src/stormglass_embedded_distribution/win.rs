use std::ffi::c_void;
pub type Handle = *mut c_void;
pub const READ: u32 = 0x8000_0000;
pub const DELETE: u32 = 0x0001_0000;
pub const SHARE_READ: u32 = 1;
pub const REPARSE: u32 = 0x0020_0000;
pub const BACKUP: u32 = 0x0200_0000;
#[repr(C)]
#[derive(Default, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub attributes: u32,
    pub creation: [u32; 2],
    pub access: [u32; 2],
    pub write: [u32; 2],
    pub volume: u32,
    pub size_hi: u32,
    pub size_lo: u32,
    pub links: u32,
    pub index_hi: u32,
    pub index_lo: u32,
}
#[repr(C)]
pub struct Rename {
    pub flags: u32,
    pub root: Handle,
    pub length: u32,
    pub name: [u16; 1],
}
#[repr(C)]
#[derive(Default)]
pub struct IoStatus {
    pub status: usize,
    pub information: usize,
}
#[link(name = "kernel32")]
extern "system" {
    pub fn GetFinalPathNameByHandleW(
        handle: Handle,
        buffer: *mut u16,
        capacity: u32,
        flags: u32,
    ) -> u32;
    pub fn GetFileInformationByHandle(handle: Handle, info: *mut FileInfo) -> i32;
    pub fn GetDriveTypeW(path: *const u16) -> u32;
    pub fn GetVolumeInformationByHandleW(
        handle: Handle,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        component: *mut u32,
        flags: *mut u32,
        filesystem: *mut u16,
        filesystem_len: u32,
    ) -> i32;
}
#[link(name = "ntdll")]
extern "system" {
    pub fn NtSetInformationFile(
        handle: Handle,
        io: *mut IoStatus,
        info: *const c_void,
        size: u32,
        class: u32,
    ) -> i32;
}
