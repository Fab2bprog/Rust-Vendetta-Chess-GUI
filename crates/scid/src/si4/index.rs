//! Reading the `.si4` index file: a header (182 bytes) then the
//! `IndexEntry` records (47 bytes each).
//!
//! Ported from `codec_scid4.cpp` (`readIndexHeader`, `decodeIndexEntry`),
//! cross-checked against `si4_specification_fr.txt` §2.

use crate::bytes::BeReader;
use crate::entry::IndexEntry;
use crate::error::ScidError;

pub const INDEX_MAGIC: &[u8; 8] = b"Scid.si\0";
pub const INDEX_HEADER_SIZE: usize = 182;
pub const INDEX_ENTRY_SIZE: usize = 47;
const SCID_DESC_LEN: usize = 107;
const CUSTOM_FLAG_MAX: usize = 6;
const CUSTOM_FLAG_DESC_LEN: usize = 8;

/// Header of the `.si4` file (first 182 bytes).
#[derive(Debug, Clone)]
pub struct Si4Header {
    pub version:     u16,
    pub base_type:   u32,
    pub num_games:   u32,
    pub auto_load:   u32,
    pub description: String,
}

/// Reads the header (182 bytes) at the start of the `.si4` file.
///
/// # Errors
/// [`ScidError::BadMagic`] if the magic number does not match;
/// [`ScidError::BadVersion`] if the version is not 400 (the only si4 version
/// supported by this implementation); [`ScidError::Truncated`] if the
/// file is shorter than 182 bytes.
pub fn read_header(data: &[u8]) -> Result<Si4Header, ScidError> {
    let mut r = BeReader::new(data);

    let magic = r.read_bytes(8).map_err(|_| ScidError::Truncated("en-tête .si4"))?;
    if magic != INDEX_MAGIC {
        return Err(ScidError::BadMagic);
    }

    let trunc = |_| ScidError::Truncated("en-tête .si4");
    let version   = r.read_u16().map_err(trunc)?;
    let base_type = r.read_u32().map_err(trunc)?;
    let num_games = r.read_u24().map_err(trunc)?;
    let auto_load = r.read_u24().map_err(trunc)?;
    let description = r.read_fixed_cstr(SCID_DESC_LEN + 1).map_err(trunc)?;

    // Only version 400 (current si4) is supported for reading by this
    // V1 implementation; si3 databases (version < 400) do not have this
    // 54-byte section and have a header of a different size.
    if version != 400 {
        return Err(ScidError::BadVersion(version));
    }
    for _ in 0..CUSTOM_FLAG_MAX {
        r.skip(CUSTOM_FLAG_DESC_LEN + 1).map_err(trunc)?;
    }
    debug_assert_eq!(r.position(), INDEX_HEADER_SIZE);

    Ok(Si4Header { version, base_type, num_games, auto_load, description })
}

/// Decodes an `IndexEntry` (exactly [`INDEX_ENTRY_SIZE`] bytes).
///
/// # Errors
/// [`ScidError::Truncated`] if `data` is fewer than 47 bytes.
pub fn read_index_entry(data: &[u8]) -> Result<IndexEntry, ScidError> {
    let mut r = BeReader::new(data);
    let trunc = |_| ScidError::Truncated("IndexEntry .si4");

    let offset = u64::from(r.read_u32().map_err(trunc)?);

    let len_low   = u32::from(r.read_u16().map_err(trunc)?);
    let len_flags = r.read_u8().map_err(trunc)?;
    let length = ((u32::from(len_flags) & 0x80) << 9) | len_low;

    let flags16 = u32::from(r.read_u16().map_err(trunc)?);
    let full_flags = ((u32::from(len_flags) & 0x3F) << 16) | flags16;
    let non_standard_start = (full_flags & 1) != 0; // bit 0 = IDX_FLAG_START

    let wb_high    = u32::from(r.read_u8().map_err(trunc)?);
    let white_low  = u32::from(r.read_u16().map_err(trunc)?);
    let black_low  = u32::from(r.read_u16().map_err(trunc)?);
    let white_id = ((wb_high & 0xF0) << 12) | white_low;
    let black_id = ((wb_high & 0x0F) << 16) | black_low;

    let esr_high   = u32::from(r.read_u8().map_err(trunc)?);
    let event_low  = u32::from(r.read_u16().map_err(trunc)?);
    let site_low   = u32::from(r.read_u16().map_err(trunc)?);
    let round_low  = u32::from(r.read_u16().map_err(trunc)?);
    let event_id = ((esr_high & 0xE0) << 11) | event_low;
    let site_id  = ((esr_high & 0x1C) << 14) | site_low;
    let round_id = ((esr_high & 0x03) << 16) | round_low;

    let var_counts = r.read_u16().map_err(trunc)?;
    #[allow(clippy::cast_possible_truncation)]
    let result = ((var_counts >> 12) & 0x0F) as u8;

    let eco_code = r.read_u16().map_err(trunc)?;

    // Combined Dates field (§2.8): Date (low 20 bits) + relative EventDate
    // (high 12 bits). Only the Date is kept in V1 (EventDate is
    // not a standard PGN tag and is not used by `import_one`).
    let date_edate = r.read_u32().map_err(trunc)?;
    let raw_date = date_edate & 0xFFFFF;

    let white_elo_raw = r.read_u16().map_err(trunc)?;
    let black_elo_raw = r.read_u16().map_err(trunc)?;
    let white_elo = white_elo_raw & 0x0FFF;
    let black_elo = black_elo_raw & 0x0FFF;

    // FinalMatSig (4 bytes): derived field, not needed for a valid PGN.
    r.skip(4).map_err(trunc)?;
    // Low NumHalfMoves (1 byte) + combined byte (HomePawnData[0]/high
    // NumHalfMoves): neither is needed to reconstruct the PGN
    // (the actual number of half-moves is derived from actually decoding the moves).
    r.skip(2).map_err(trunc)?;
    // HomePawnData[1..8]: derived heuristic field, ignored.
    r.skip(8).map_err(trunc)?;

    debug_assert_eq!(r.position(), INDEX_ENTRY_SIZE);

    Ok(IndexEntry {
        offset,
        length,
        white_id,
        black_id,
        event_id,
        site_id,
        round_id,
        result,
        eco_code,
        date: raw_date,
        white_elo,
        black_elo,
        non_standard_start,
    })
}

/// Reads all `IndexEntry` records of the `.si4` file (after the header).
///
/// The number of games announced by the header (`num_games`) is compared to
/// the one derivable from the actual file size; the smaller of the two
/// is used, out of caution against a truncated file (defense in depth,
/// not a guarantee of integrity — a corrupted file can still produce
/// individually invalid entries, detected while decoding each
/// game).
///
/// # Errors
/// [`ScidError::Truncated`] if an entry is incomplete.
pub fn read_all_entries(data: &[u8], header: &Si4Header) -> Result<Vec<IndexEntry>, ScidError> {
    let body = &data[INDEX_HEADER_SIZE.min(data.len())..];
    let size_derived = body.len() / INDEX_ENTRY_SIZE;
    let n = (header.num_games as usize).min(size_derived);

    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * INDEX_ENTRY_SIZE;
        let entry = read_index_entry(&body[start..start + INDEX_ENTRY_SIZE])?;
        entries.push(entry);
    }
    Ok(entries)
}
