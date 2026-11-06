//! Handle-based admission and creation-time private ACLs for preferences only.
use std::{
    ffi::c_void,
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Threading::*,
};

pub(super) const READ_SECURITY: u32 = 0x0002_0000; // READ_CONTROL
pub(super) struct Allocation(pub(super) *mut c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: only LocalAlloc-family API outputs are stored here, exactly once.
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub(super) fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(Some(0)).collect()
}
pub(super) struct Security {
    user: Vec<u32>,
    descriptor: Allocation,
}
impl Security {
    pub(super) fn new() -> Result<Self, String> {
        // SAFETY: API outputs point into live aligned buffers or owned API allocations.
        unsafe {
            let mut token = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(last());
            }
            let token = OwnedHandle::from_raw_handle(token);
            let mut size = 0;
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &mut size,
            );
            if size == 0 || size > 65536 {
                return Err("invalid user token size".into());
            }
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            if GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(last());
            }
            let sid = (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            if IsValidSid(sid) == 0 {
                return Err("invalid process user SID".into());
            }
            let length = GetLengthSid(sid);
            let mut user = vec![0u32; (length as usize).div_ceil(4)];
            if CopySid(length, user.as_mut_ptr().cast(), sid) == 0 {
                return Err(last());
            }
            let mut text = ptr::null_mut();
            if ConvertSidToStringSidW(sid, &mut text) == 0 {
                return Err(last());
            }
            let text_owner = Allocation(text.cast());
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid_text = String::from_utf16(std::slice::from_raw_parts(text, len))
                .map_err(|_| "invalid SID encoding")?;
            drop(text_owner);
            Self::from_sddl(
                user,
                &format!("O:{sid_text}D:P(A;;FA;;;{sid_text})(A;;FA;;;SY)(A;;FA;;;BA)"),
            )
        }
    }
    fn from_sddl(user: Vec<u32>, text: &str) -> Result<Self, String> {
        let text = wide(std::ffi::OsStr::new(text));
        let mut descriptor = ptr::null_mut();
        // SAFETY: text is NUL-terminated; descriptor is returned as a LocalFree allocation.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(last());
        }
        Ok(Self {
            user,
            descriptor: Allocation(descriptor),
        })
    }
    pub(super) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }
    #[cfg(test)]
    pub(super) fn test_acl(acl: &str) -> Self {
        let current = Self::new().unwrap();
        let mut text = ptr::null_mut();
        // SAFETY: valid owned SID, output allocation is retained until conversion completes.
        unsafe {
            assert_ne!(
                ConvertSidToStringSidW(current.user.as_ptr().cast_mut().cast(), &mut text),
                0
            );
            let _owner = Allocation(text.cast());
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid = String::from_utf16(std::slice::from_raw_parts(text, len)).unwrap();
            Self::from_sddl(current.user, &format!("O:{sid}{acl}")).unwrap()
        }
    }
    pub(super) fn private(&self, file: &File) -> Result<(), String> {
        // SAFETY: GetSecurityInfo returns owner/DACL pointers within the owned descriptor.
        unsafe {
            let mut owner = ptr::null_mut();
            let mut acl = ptr::null_mut();
            let mut descriptor = ptr::null_mut();
            let code = GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut acl,
                ptr::null_mut(),
                &mut descriptor,
            );
            let _allocation = Allocation(descriptor);
            if code != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(code as i32).to_string());
            }
            if owner.is_null()
                || IsValidSid(owner) == 0
                || EqualSid(owner, self.user.as_ptr().cast_mut().cast()) == 0
                || acl.is_null()
                || IsValidAcl(acl) == 0
            {
                return Err("settings require current-user ownership and a private DACL".into());
            }
            for index in 0..u32::from((*acl).AceCount) {
                let mut ace = ptr::null_mut();
                if GetAce(acl, index, &mut ace) == 0 {
                    return Err(last());
                }
                let header = &*ace.cast::<ACE_HEADER>();
                // A deny entry cannot grant access. Reject unrecognized grant formats.
                if header.AceType == 1 {
                    continue;
                } // ACCESS_DENIED_ACE_TYPE
                if header.AceType != 0
                    || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
                {
                    return Err("unsupported settings ACL entry".into());
                }
                let sid = ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast();
                if IsValidSid(sid) == 0
                    || GetLengthSid(sid) as usize + 8 > usize::from(header.AceSize)
                {
                    return Err("invalid settings ACL SID".into());
                }
                if EqualSid(sid, self.user.as_ptr().cast_mut().cast()) == 0
                    && IsWellKnownSid(sid, WinLocalSystemSid) == 0
                    && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0
                {
                    return Err(
                        "settings ACL grants access beyond user, SYSTEM and Administrators".into(),
                    );
                }
            }
        }
        Ok(())
    }
    pub(super) fn create(&self, path: &std::path::Path) -> io::Result<File> {
        let name = wide(path.as_os_str());
        let attributes = self.attributes();
        // SAFETY: all pointers stay live through CreateFileW; successful handle is uniquely owned.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | READ_SECURITY,
                0,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_handle(handle) })
        }
    }
}
pub(super) fn last() -> String {
    io::Error::last_os_error().to_string()
}
