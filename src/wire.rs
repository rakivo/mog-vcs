//! Read/write cursor for object payloads. Single place for byte layout and field order.

use crate::hash::Hash;
use anyhow::{Result, bail};

pub trait Encode {
    fn encode(&self, w: &mut WriteCursor<'_>);
}

pub trait Decode: Sized {
    fn decode(r: &mut ReadCursor<'_>) -> Result<Self>;
}

/// Read from a byte slice; advances offset.
#[derive(Clone)]
pub struct ReadCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ReadCursor<'a> {
    #[inline]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[inline]
    pub fn remaining(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    #[inline]
    pub fn at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    #[inline]
    fn ensure(&self, n: usize) -> Result<()> {
        if self.data.len().saturating_sub(self.pos) < n {
            bail!("wire: unexpected end of data");
        }
        Ok(())
    }

    #[inline]
    pub fn read_u32(&mut self) -> Result<u32> {
        self.ensure(4)?;
        let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into()?);
        self.pos += 4;
        Ok(v)
    }

    #[inline]
    pub fn read_u64(&mut self) -> Result<u64> {
        self.ensure(8)?;
        let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into()?);
        self.pos += 8;
        Ok(v)
    }

    #[inline]
    pub fn read_i64(&mut self) -> Result<i64> {
        self.ensure(8)?;
        let v = i64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into()?);
        self.pos += 8;
        Ok(v)
    }

    #[inline]
    pub fn read_hash(&mut self) -> Result<Hash> {
        self.ensure(32)?;
        let mut h = [0u8; 32];
        h.copy_from_slice(&self.data[self.pos..self.pos + 32]);
        self.pos += 32;
        Ok(h)
    }

    #[inline]
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.ensure(n)?;
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Read u32 length then that many bytes.
    #[inline]
    pub fn read_len_prefixed_bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.read_u32()? as usize;
        self.read_bytes(n)
    }

    /// Read u32 length then that many bytes as UTF-8 string.
    #[inline]
    pub fn read_len_prefixed_str(&mut self) -> Result<std::borrow::Cow<'a, str>> {
        let bytes = self.read_len_prefixed_bytes()?;
        Ok(std::str::from_utf8(bytes)?.into())
    }
}

/// Write into a Vec<u8>.
pub struct WriteCursor<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> WriteCursor<'a> {
    #[inline]
    pub fn new(buf: &'a mut Vec<u8>) -> Self {
        Self { buf }
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn write_hash(&mut self, h: &Hash) {
        self.buf.extend_from_slice(h);
    }

    #[inline]
    pub fn write_slice(&mut self, s: &[u8]) {
        self.buf.extend_from_slice(s);
    }

    #[inline]
    pub fn write_len_prefixed_bytes(&mut self, s: &[u8]) {
        self.write_u32(s.len() as u32);
        self.write_slice(s);
    }

    #[inline]
    pub fn write_len_prefixed_str(&mut self, s: &str) {
        self.write_len_prefixed_bytes(s.as_bytes());
    }
}
