//! Reading the `.sn5` name file: an APPEND-ONLY journal, WITHOUT a header,
//! varint encoding (LEB128-style) — see `si5_specification_fr.txt` §3,
//! reverse-engineered from `codec_scid5.h` (`NameBase`/`.sn5` handling).
//!
//! A format VERY different from `.sn4` (no alphabetical sorting, no
//! prefix compression, no header, IDs assigned implicitly by
//! insertion order rather than read from disk) — but producing the
//! SAME neutral structure [`crate::names::NameTables`] as `si4::namebase`
//! (V2 Phase C1, 12/07/2026, task #21).
//!
//! Unlike `.si4`/`.sn4`, this format has NO verifiable magic byte
//! at opening time (see §2.1/§3.1 of the spec): any file of the
//! correct structural shape would be accepted with no way to tell it
//! apart — this is a limitation of the format itself, not of this
//! implementation.

use crate::error::ScidError;
use crate::names::NameTables;

/// Reads and reconstructs the 4 name tables of the `.sn5` file — walks all
/// records until EOF (no explicit end marker).
///
/// # Errors
/// [`ScidError::Truncated`] if a varint or a string is truncated at the
/// end of the file.
pub fn read_namebase(data: &[u8]) -> Result<NameTables, ScidError> {
    let mut tables = NameTables::default();
    let mut pos: usize = 0;

    while pos < data.len() {
        let (header, consumed) =
            read_varint(&data[pos..]).ok_or(ScidError::Truncated(".sn5 : varint corrompu"))?;
        pos += consumed;

        let name_type = header & 0b111;
        // Length in bytes of the string that follows (`header >> 3`) —
        // implicitly bounded by the remaining file size via `data.get`
        // below, no need to check a theoretical maximum here.
        let len = usize::try_from(header >> 3).map_err(|_| ScidError::Truncated(".sn5 : longueur de nom absurde"))?;

        let bytes = data.get(pos..pos + len).ok_or(ScidError::Truncated(".sn5 : chaîne de nom tronquée"))?;
        pos += len;

        let name = String::from_utf8_lossy(bytes).into_owned();
        match name_type {
            0 => tables.players.push(name),
            1 => tables.events.push(name),
            2 => tables.sites.push(name),
            3 => tables.rounds.push(name),
            // Type 4 ("NAME_INFO" — database metadata: description,
            // autoload, custom flag labels...) and any other unknown type
            // (future format evolution) are both silently ignored here:
            // not a browsable name, no sequential ID, no current use of
            // this metadata by this crate (V1) — but its string was
            // already read/consumed above, so reading of the following
            // records stays correctly synchronized either way.
            _ => {}
        }
    }

    Ok(tables)
}

/// Decodes a varint integer (LEB128-style, least-significant 7-bit groups
/// first, bit 0x80 = continuation flag) at the start of `data`.
///
/// Returns `(value, bytes_consumed)`, or `None` if `data` runs out
/// before an end-of-varint byte (bit 0x80 clear) is encountered,
/// or if the varint exceeds 64 bits (defense against a corrupted file).
fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    let mut consumed: usize = 0;

    loop {
        let byte = *data.get(consumed)?;
        consumed += 1;
        result |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some((result, consumed));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}
