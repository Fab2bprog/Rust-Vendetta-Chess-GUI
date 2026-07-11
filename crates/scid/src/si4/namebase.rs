//! Reading the `.sn4` name file: a header (36 bytes) then, for each
//! name type (PLAYER, EVENT, SITE, ROUND, in that order), the
//! front-coded records sorted alphabetically.
//!
//! Ported from `codec_scid4.cpp::namefileRead`, cross-checked against
//! `si4_specification_fr.txt` §3.

use crate::bytes::BeReader;
use crate::error::ScidError;
use crate::names::NameTables;

pub const NAMEBASE_MAGIC: &[u8; 8] = b"Scid.sn\0";
pub const HEADER_SIZE: usize = 36;

/// Index of the 4 name types, in the order they appear in the file.
pub const NAME_PLAYER: usize = 0;
pub const NAME_EVENT: usize = 1;
pub const NAME_SITE: usize = 2;
pub const NAME_ROUND: usize = 3;
const NUM_NAME_TYPES: usize = 4;

/// Reads and reconstructs the 4 name tables of the `.sn4` file.
///
/// # Errors
/// [`ScidError::BadMagic`] if the magic number does not match;
/// [`ScidError::Truncated`] if the file is incomplete or inconsistent
/// (front-coded prefix longer than the name, out-of-bounds ID...).
pub fn read_namebase(data: &[u8]) -> Result<NameTables, ScidError> {
    let mut r = BeReader::new(data);
    let trunc = |_| ScidError::Truncated("en-tête .sn4");

    let magic = r.read_bytes(8).map_err(trunc)?;
    if magic != NAMEBASE_MAGIC {
        return Err(ScidError::BadMagic);
    }
    r.skip(4).map_err(trunc)?; // timeStamp, obsolete, unused

    let mut num_names = [0u32; NUM_NAME_TYPES];
    for n in &mut num_names {
        *n = r.read_u24().map_err(trunc)?;
    }
    let mut max_freq = [0u32; NUM_NAME_TYPES];
    for f in &mut max_freq {
        *f = r.read_u24().map_err(trunc)?;
    }
    debug_assert_eq!(r.position(), HEADER_SIZE);

    // Defensive bound (11/07/2026, robustness audit finding 2.1):
    // `num_names[nt]` is fully file-controlled (raw `u24`, up to
    // 16,777,215) and was used directly below to size a `Vec<String>`
    // allocation, unlike `si4::index::read_all_entries` which already
    // clamps its own entry count by the file's real size. A corrupted or
    // malicious `.sn4` file of a few dozen bytes could claim ~16M names
    // per type and trigger ~1.6 GB of allocation before a single further
    // byte is validated — worse than an ordinary panic, since a failed
    // allocation aborts the whole process in Rust rather than unwinding.
    // Each record needs at least 1 on-disk byte (id/frequency/length are
    // never omitted), so no type can legitimately claim more names than
    // there are bytes left in the file after the header: reject early
    // with the usual `Truncated` error instead of allocating.
    let remaining_after_header = data.len().saturating_sub(r.position());
    if num_names.iter().any(|&n| n as usize > remaining_after_header) {
        return Err(ScidError::Truncated(".sn4 : nombre de noms incohérent avec la taille du fichier"));
    }

    let mut tables = NameTables {
        players: vec![String::new(); num_names[NAME_PLAYER] as usize],
        events:  vec![String::new(); num_names[NAME_EVENT] as usize],
        sites:   vec![String::new(); num_names[NAME_SITE] as usize],
        rounds:  vec![String::new(); num_names[NAME_ROUND] as usize],
    };

    for nt in [NAME_PLAYER, NAME_EVENT, NAME_SITE, NAME_ROUND] {
        let dest = match nt {
            NAME_PLAYER => &mut tables.players,
            NAME_EVENT  => &mut tables.events,
            NAME_SITE   => &mut tables.sites,
            _           => &mut tables.rounds,
        };
        read_one_type(&mut r, num_names[nt], max_freq[nt], dest)?;
    }

    Ok(tables)
}

/// Reads the `num_names` front-coded records of a name type, and
/// places them into `dest` at the index given by their `id` field (NOT the
/// reading order, which is alphabetical — see spec §3.3).
fn read_one_type(
    r: &mut BeReader<'_>,
    num_names: u32,
    max_freq: u32,
    dest: &mut [String],
) -> Result<(), ScidError> {
    let trunc = |_| ScidError::Truncated(".sn4 : enregistrement de nom");
    // Front-coding operates on raw BYTES (not necessarily valid UTF-8
    // characters taken in isolation): we therefore accumulate the previous
    // name as `Vec<u8>`, not as `String`, to avoid any risk of a panic
    // from slicing in the middle of a multi-byte character (accented
    // names...). The conversion to UTF-8 (with replacement, "lossy")
    // only happens once the full name has been reconstructed.
    let mut prev_name: Vec<u8> = Vec::new();

    for i in 0..num_names {
        let id = if num_names >= 65536 {
            r.read_u24().map_err(trunc)?
        } else {
            u32::from(r.read_u16().map_err(trunc)?)
        };

        // Usage frequency: obsolete, but must be read to correctly
        // advance the cursor (its on-disk size depends on the
        // overall maximum frequency of the type, not on the individual value).
        if max_freq >= 65536 {
            r.skip(3).map_err(trunc)?;
        } else if max_freq >= 256 {
            r.skip(2).map_err(trunc)?;
        } else {
            r.skip(1).map_err(trunc)?;
        }

        let length = usize::from(r.read_u8().map_err(trunc)?);
        let prefix = if i > 0 { usize::from(r.read_u8().map_err(trunc)?) } else { 0 };
        if prefix > length || prefix > prev_name.len() {
            return Err(ScidError::Truncated(".sn4 : préfixe front-codé invalide"));
        }

        let new_chars = length - prefix;
        let suffix = r.read_bytes(new_chars).map_err(trunc)?;

        let mut name_bytes = Vec::with_capacity(length);
        name_bytes.extend_from_slice(&prev_name[..prefix]);
        name_bytes.extend_from_slice(suffix);

        let slot = dest
            .get_mut(id as usize)
            .ok_or(ScidError::Truncated(".sn4 : ID de nom hors bornes"))?;
        *slot = String::from_utf8_lossy(&name_bytes).into_owned();
        prev_name = name_bytes;
    }
    Ok(())
}
