//! Byte reader/writer used by the message codecs. All integers are little
//! endian, all lengths are `u32` (never `usize`). The reader never
//! allocates from an untrusted count before checking it against the bytes
//! that are actually left.
use crate::msg::ProtoError;

pub(crate) struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { out: Vec::with_capacity(64) }
    }
    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    /// `u32` length, then the bytes.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.out.extend_from_slice(v);
    }
    pub fn raw(&mut self, v: &[u8]) {
        self.out.extend_from_slice(v);
    }
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub fn remaining(&self) -> u64 {
        self.bytes.len() as u64
    }
    pub fn take(&mut self, n: u64) -> Result<&'a [u8], ProtoError> {
        if n > self.remaining() {
            return Err(ProtoError::Truncated);
        }
        let (head, rest) = self.bytes.split_at(n as usize);
        self.bytes = rest;
        Ok(head)
    }
    pub fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }
    pub fn u32(&mut self) -> Result<u32, ProtoError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, ProtoError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, ProtoError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// `u32` length (at most `max`), then that many bytes.
    pub fn bytes(&mut self, max: u32) -> Result<Vec<u8>, ProtoError> {
        let len = self.u32()?;
        if len > max {
            return Err(ProtoError::TooLarge);
        }
        Ok(self.take(u64::from(len))?.to_vec())
    }
    /// An element count, checked so `count * min_elem_size` bytes remain.
    pub fn count(&mut self, min_elem_size: u64, max: u32) -> Result<u32, ProtoError> {
        let n = self.u32()?;
        if n > max {
            return Err(ProtoError::TooLarge);
        }
        if u64::from(n) * min_elem_size > self.remaining() {
            return Err(ProtoError::Truncated);
        }
        Ok(n)
    }
    pub fn finish(&self) -> Result<(), ProtoError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(ProtoError::Corrupt("trailing bytes"))
        }
    }
}
