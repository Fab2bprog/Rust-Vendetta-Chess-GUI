//! Reading the `.si5` index file: NO header, a raw array of
//! 56-byte records each (12 LITTLE-ENDIAN 32-bit words +
//! 8 raw bytes of `HomePawn` data) — see `si5_specification_fr.txt`
//! §2, reverse-engineered from `codec_scid5.h` (`encode_IndexEntry`/
//! `decode_IndexEntry`) and `indexentry.h` (corresponding C++ bitfields).
//!
//! A format VERY different from `.si4` (no header, words packed
//! differently, LITTLE-ENDIAN instead of BIG-ENDIAN) — but producing the SAME
//! neutral structure [`crate::entry::IndexEntry`] as `si4::index`, which
//! allows `pgn_build`/`si5::database` to remain identical to their
//! si4 equivalents (V2 Phase C1, 12/07/2026, task #21).

use crate::entry::IndexEntry;
use crate::error::ScidError;

/// Fixed size of a `.si5` record (12 32-bit words + 8 bytes of
/// `HomePawn`), see `si5_specification_fr.txt` §2.2.
pub const INDEX_ENTRY_SIZE: usize = 56;

/// Reads a LITTLE-ENDIAN 32-bit word at offset `word_index` (0-11) of a
/// 56-byte record.
fn word_le(record: &[u8], word_index: usize) -> u32 {
    let start = word_index * 4;
    let bytes: [u8; 4] = record[start..start + 4]
        .try_into()
        .expect("découpe de 4 octets toujours valide dans un enregistrement de 56 octets");
    u32::from_le_bytes(bytes)
}

/// Decodes an `IndexEntry` (exactly [`INDEX_ENTRY_SIZE`] bytes).
///
/// Layout of the 12 words — see `si5_specification_fr.txt` §2.2/§2.3 for
/// the full detail (only the fields useful for reconstructing a PGN
/// are kept, the heuristic/derived fields are read for the record
/// but discarded, exactly as for si4):
///   word 0 (bytes 0-3)   : 4 bits comment count (ignored) | 28 bits White ID
///   word 1 (bytes 4-7)   : 4 bits variation count (ignored) | 28 bits Black ID
///   word 2 (bytes 8-11)  : 4 bits NAG count (ignored)      | 28 bits Event ID
///   word 3 (bytes 12-15) : 32 bits Site ID (full word)
///   word 4 (bytes 16-19) : 1 bit Chess960 (ignored)        | 31 bits Round ID
///   word 5 (bytes 20-23) : 12 bits White Elo               | 20 bits Date
///   word 6 (bytes 24-27) : 12 bits Black Elo                | 20 bits `EventDate` (ignored)
///   word 7 (bytes 28-31) : 10 bits `NumHalfMoves` (ignored)   | 22 bits raw Flags
///   word 8 (bytes 32-35) : 17 bits Length                   | 15 bits Offset (high bits 32-46)
///   word 9 (bytes 36-39) : 32 bits Offset (low bits 0-31)
///   word 10 (bytes 40-43): 8 bits `StoredLineCode` (ignored)  | 24 bits `FinalMatSig` (ignored)
///   word 11 (bytes 44-47): 8 bits `HomePawn` count (ignored) | 3+3 bits Elo types (ignored) | 2 bits Result || 16 bits ECO
///   bytes 48-55          : raw `HomePawn` data (ignored)
///
/// # Errors
/// [`ScidError::Truncated`] if `data` is fewer than 56 bytes.
pub fn read_index_entry(data: &[u8]) -> Result<IndexEntry, ScidError> {
    let record = data.get(..INDEX_ENTRY_SIZE).ok_or(ScidError::Truncated("IndexEntry .si5"))?;

    let word0 = word_le(record, 0);
    let white_id = word0 & 0x0FFF_FFFF;

    let word1 = word_le(record, 1);
    let black_id = word1 & 0x0FFF_FFFF;

    let word2 = word_le(record, 2);
    let event_id = word2 & 0x0FFF_FFFF;

    let site_id = word_le(record, 3);

    let word4 = word_le(record, 4);
    let round_id = word4 & 0x7FFF_FFFF;

    let word5 = word_le(record, 5);
    let raw_date = word5 & 0xFFFFF;
    #[allow(clippy::cast_possible_truncation)]
    let white_elo = ((word5 >> 20) & 0xFFF) as u16;

    let word6 = word_le(record, 6);
    #[allow(clippy::cast_possible_truncation)]
    let black_elo = ((word6 >> 20) & 0xFFF) as u16;

    let word7 = word_le(record, 7);
    let flags = word7 & 0x3F_FFFF; // 22 raw bits (bitmask)
    let non_standard_start = (flags & 1) != 0; // bit 0 = START

    let word8 = word_le(record, 8);
    let offset_high15 = u64::from(word8 & 0x7FFF);
    let length = word8 >> 15;

    let offset_low32 = u64::from(word_le(record, 9));
    let offset = (offset_high15 << 32) | offset_low32;

    let word11 = word_le(record, 11);
    #[allow(clippy::cast_possible_truncation)]
    let eco_code = (word11 & 0xFFFF) as u16;
    #[allow(clippy::cast_possible_truncation)]
    let result = ((word11 >> 16) & 0x3) as u8;

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

/// Reads all `IndexEntry` records of the `.si5` file — no header, the
/// number of games is derived directly from the file size (see
/// `si5_specification_fr.txt` §2.1: `n_games = file_size / 56`, must
/// be an exact multiple, otherwise the database is truncated/corrupted — the
/// possible last partial entry is silently ignored rather
/// than failing to open the entire database, the same defense in
/// depth as `si4::index::read_all_entries`).
///
/// # Errors
/// Should never fail in practice (each 56-byte chunk is
/// already validated in size by construction); the `Result` type is kept
/// for symmetry with `si4::index::read_all_entries` and possible future
/// validation.
pub fn read_all_entries(data: &[u8]) -> Result<Vec<IndexEntry>, ScidError> {
    let n = data.len() / INDEX_ENTRY_SIZE;
    let mut entries = Vec::with_capacity(n);
    for i in 0..n {
        let start = i * INDEX_ENTRY_SIZE;
        let entry = read_index_entry(&data[start..start + INDEX_ENTRY_SIZE])?;
        entries.push(entry);
    }
    Ok(entries)
}
