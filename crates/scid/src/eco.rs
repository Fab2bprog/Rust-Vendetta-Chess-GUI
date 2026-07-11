//! Conversion of the numeric ECO code (`ecoT`) stored in the `IndexEntry` to
//! its standard textual representation (`"B01"`, `"C10"`, ...).
//!
//! Ported directly from `eco_ToString()` (`src/misc.cpp` of the provided
//! SCID source tree). Numbering: no code = 0, `A00` = 1, `A01` = 132
//! (each base code = previous + 131; the remaining 130 sub-codes are
//! the SCID extension `a`, `a1`..`a4`, `b`, `b1`..`b4`, ..., `z`, `z1`..`z4`).
//! Only the base code (3 characters) is produced here: the
//! SCID-specific extensions have no standard PGN equivalent and are not
//! needed for a valid ECO tag.

/// Converts a raw ECO code into a `"A00"`..`"E99"` string, or `None` if the
/// code is 0 (`ECO_None`, no classification).
#[must_use]
pub fn eco_to_string(eco_code: u16) -> Option<String> {
    if eco_code == 0 {
        return None;
    }
    let code = u32::from(eco_code) - 1;
    let basic_code = code / 131; // 131 = 26 * 5 + 1 sub-codes

    let letter = u8::try_from(basic_code / 100).ok()? + b'A';
    let tens   = (basic_code % 100) / 10;
    let units  = basic_code % 10;
    if letter > b'E' {
        // Out-of-bounds code ("A".."E" expected): corrupted data, ignored
        // rather than producing an invalid ECO tag.
        return None;
    }

    Some(format!("{}{tens}{units}", letter as char))
}
