//! Low-level, big-endian read cursor, used for
//! `.si4` and `.sn4` (see `si4_specification_fr.txt` §1: "ALL multi-byte
//! integers ... are stored in BIG-ENDIAN").
//!
//! All reads are bounds-checked: reading past the end of the buffer
//! returns an error rather than panicking (truncated/corrupted file).

/// Read cursor over an in-memory buffer, big-endian.
pub struct BeReader<'a> {
    data: &'a [u8],
    pos:  usize,
}

/// Read error: not enough bytes left in the buffer.
#[derive(Debug, Clone, Copy)]
pub struct Eof;

impl<'a> BeReader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Eof> {
        if self.remaining() < n {
            return Err(Eof);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Advances the cursor by `n` bytes without returning them.
    ///
    /// # Errors
    /// [`Eof`] if fewer than `n` bytes remain in the buffer.
    pub fn skip(&mut self, n: usize) -> Result<(), Eof> {
        self.take(n).map(|_| ())
    }

    /// Reads a single unsigned byte.
    ///
    /// # Errors
    /// [`Eof`] if the buffer is exhausted.
    pub fn read_u8(&mut self) -> Result<u8, Eof> {
        Ok(self.take(1)?[0])
    }

    /// Unsigned 2-byte integer, most significant byte first.
    ///
    /// # Errors
    /// [`Eof`] if fewer than 2 bytes remain in the buffer.
    pub fn read_u16(&mut self) -> Result<u16, Eof> {
        let b = self.take(2)?;
        Ok((u16::from(b[0]) << 8) | u16::from(b[1]))
    }

    /// Unsigned 3-byte integer (most significant byte first), used for
    /// `numGames`, `autoLoad`, the `.sn4` name counters, etc.
    ///
    /// # Errors
    /// [`Eof`] if fewer than 3 bytes remain in the buffer.
    pub fn read_u24(&mut self) -> Result<u32, Eof> {
        let b = self.take(3)?;
        Ok((u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]))
    }

    /// Unsigned 4-byte integer, most significant byte first.
    ///
    /// # Errors
    /// [`Eof`] if fewer than 4 bytes remain in the buffer.
    pub fn read_u32(&mut self) -> Result<u32, Eof> {
        let b = self.take(4)?;
        Ok((u32::from(b[0]) << 24)
            | (u32::from(b[1]) << 16)
            | (u32::from(b[2]) << 8)
            | u32::from(b[3]))
    }

    /// Reads `n` raw bytes.
    ///
    /// # Errors
    /// [`Eof`] if fewer than `n` bytes remain in the buffer.
    pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], Eof> {
        self.take(n)
    }

    /// Reads a fixed-size string of `n` bytes, truncated at the first null
    /// byte encountered ("C string" convention used by the `.si4` header
    /// and the custom flag labels).
    ///
    /// # Errors
    /// [`Eof`] if fewer than `n` bytes remain in the buffer.
    pub fn read_fixed_cstr(&mut self, n: usize) -> Result<String, Eof> {
        let b = self.take(n)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Ok(String::from_utf8_lossy(&b[..end]).into_owned())
    }

    /// Reads a null-terminated string of variable length
    /// (used for the non-standard starting FEN in the `.sg4`).
    ///
    /// # Errors
    /// [`Eof`] if no null byte is found before the end of the buffer.
    pub fn read_terminated_cstr(&mut self) -> Result<String, Eof> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&c| c == 0).ok_or(Eof)?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1; // includes the null byte
        Ok(s)
    }
}
