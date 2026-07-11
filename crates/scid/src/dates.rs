//! SCID's `dateT` encoding (20 useful bits) and conversion to the standard
//! PGN date format (`"YYYY.MM.DD"`, `"?"` for any unknown field).
//!
//! See `si4_specification_fr.txt` §1.3:
//!   bits 0-4 (5 bits)  : day (0 = unknown)
//!   bits 5-8 (4 bits)  : month (0 = unknown)
//!   bits 9-19 (11 bits): year (0 = unknown)

/// Converts a raw `dateT` value into a PGN date string.
/// `date == 0` (`ZERO_DATE`) gives the fully unknown PGN date.
#[must_use]
pub fn date_to_pgn(date: u32) -> String {
    let day   = date & 0x1F;
    let month = (date >> 5) & 0x0F;
    let year  = (date >> 9) & 0x7FF;

    let y = if year == 0 { "????".to_string() } else { format!("{year:04}") };
    let m = if month == 0 { "??".to_string() } else { format!("{month:02}") };
    let d = if day == 0 { "??".to_string() } else { format!("{day:02}") };
    format!("{y}.{m}.{d}")
}

/// Extracts the year (0 = unknown) from a raw `dateT` value.
/// Used to decode the combined `Dates` field of the `IndexEntry` (§2.8).
#[must_use]
pub fn get_year(date: u32) -> u32 {
    (date >> 9) & 0x7FF
}

/// Extracts the month (0 = unknown) from a raw `dateT` value.
#[must_use]
pub fn get_month(date: u32) -> u32 {
    (date >> 5) & 0x0F
}

/// Extracts the day (0 = unknown) from a raw `dateT` value.
#[must_use]
pub fn get_day(date: u32) -> u32 {
    date & 0x1F
}

/// Builds a raw `dateT` value from its components.
#[must_use]
pub fn make(year: u32, month: u32, day: u32) -> u32 {
    (year << 9) | (month << 5) | day
}
