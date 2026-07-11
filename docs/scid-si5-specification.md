# SCID `si5` Database Format — Technical Specification

> **Reverse-engineering notice** — This specification was produced through
> independent reverse engineering of the reference SCID C++ source code
> (`scid-code-si5`, `src/` tree), reflecting the exact read/write behavior
> implemented by the `CodecSCID5` class (`codec_scid5.h`) and the
> supporting classes `Index`, `IndexEntry`, `NameBase`, `ByteBuffer`,
> `FileMap` and `Filebuf`, as well as by the `Game` class (`game.cpp`) for
> encoding/decoding a game's content. It is **not** an official document
> of the SCID project. (*Author: Fabrice Garcia*)

This document is self-contained: all the information needed to fully read
or write an si5 database (the three files `.si5`, `.sn5`, `.sg5`) is
contained here, with no need to refer to any other document.

**Historical context** (for information only): si5 is the successor of
the si4 format. It is a direct evolution of si4, but at two very different
speeds: the **container** (index and name files) was entirely redesigned,
while the **content** of a game (moves, extra PGN tags, comments,
variations, NAGs) was carried over almost identically from si4, down to
the bit (see §4). Main author of this redesign: Fulvio Benini (copyright
2014-2017 for the general "Codec" architecture, 2022 for `CodecSCID5`), in
addition to Shane Hudson and Pascal Georges, the historical authors of the
si4 format.

## Table of contents

1. [Overview, capacity limits and conventions](#1-overview-capacity-limits-and-conventions)
2. [`.si5` file (index) — fixed 56-byte records, no header](#2-si5-file-index--fixed-56-byte-records-no-header)
3. [`.sn5` file (names and database metadata)](#3-sn5-file-names-and-database-metadata)
4. [`.sg5` file (game data: moves, tags, comments, variations)](#4-sg5-file-game-data-moves-tags-comments-variations)
5. [Operational summary: reading / writing an si5 database](#5-operational-summary-reading--writing-an-si5-database)
6. [Summary of analyzed source files](#6-summary-of-analyzed-source-files)

An si5 database consists of **three files** sharing the same base name
with different suffixes:

| Suffix | Role |
|---|---|
| `<base>.si5` | Index file — fixed-size record table, NO header |
| `<base>.sn5` | Name file — players, tournaments, sites, rounds + database metadata |
| `<base>.sg5` | Game file — moves, comments, variations, extra PGN tags |

> **Note** — None of the three files is usable on its own: the `.si5`
> references name IDs resolved in the `.sn5`, and references data blocks
> (offset + length) located in the `.sg5`.

---

## 1. Overview, capacity limits and conventions

### 1.1 Basic types

| Type | Definition |
|---|---|
| `byte` | unsigned integer, 8 bits |
| `uint16_t` | unsigned integer, 16 bits |
| `uint32_t` | unsigned integer, 32 bits |
| `uint64_t` | unsigned integer, 64 bits |
| `dateT` | unsigned integer (at least 20 usable bits), encoding §1.3 |
| `eloT` | unsigned integer (at least 12 usable bits) |
| `ecoT` | unsigned integer, 16 bits (raw ECO code) |
| `resultT` | 2 bits: 0=no result (`*`), 1=White wins (1-0), 2=Black wins (0-1), 3=Draw (1/2-1/2) |
| `idNumberT` | unsigned integer, 32 bits (name identifier, reference into `.sn5`) |

### 1.2 Capacity limits

| Limit | Value |
|---|---:|
| Maximum number of games | 2³² − 2 (about 4.29 billion) |
| Maximum size of the `.sg5` file | 2⁴⁷ bytes (128 TB, 47 usable offset bits) |
| Maximum size of one game's data | 131,072 bytes (128 KB, 17 usable bits) |
| Unique player names (max) | 2²⁸ (about 268 million) |
| Unique tournament names (max) | 2²⁸ (about 268 million) |
| Unique site names (max) | 2³² (about 4.3 billion) |
| Unique round names (max) | 2³¹ (about 2.1 billion) |

A dedicated per-game bit indicates whether the game is standard chess or
Chess960 (random starting-position variant); the moves themselves are
encoded the same way in both cases (§4).

### 1.3 Date encoding (`dateT`, over 20 usable bits)

A date encodes year/month/day in a single integer:

| Bits | Field | Range |
|---|---|---|
| 0-4 (5 bits) | day of month | 0-31 (0 = unknown day) |
| 5-8 (4 bits) | month | 0-15 (0 = unknown month) |
| 9-19 (11 bits) | year | 0-2047 (0 = unknown year) |

```
date = (year << 9) | (month << 5) | day
```

A date with value 0 means "no date information". A numerically larger
date is chronologically more recent. This same encoding is used both for
the game's date and for the tournament date (EventDate) — see §2.3 for
how they are stored in the si5 `IndexEntry` (each on its own full,
independent 20 bits, without the relative compression used by the older
format).

### 1.4 Endianness conventions (important and NOT uniform across this format)

- The 12 32-bit words that make up each record of the `.si5` (index) file
  are stored in **little-endian** order (least significant byte first):
  for a word with value V, byte 0 equals `V & 0xFF`, byte 1 equals
  `(V>>8) & 0xFF`, byte 2 equals `(V>>16) & 0xFF`, byte 3 equals
  `(V>>24) & 0xFF`.
- The `.sn5` (names) file uses a "varint" encoding (LEB128-style, see
  §3.3), which is neither big-endian nor little-endian in the classic
  sense, but a concatenation of 7-bit groups, least significant first.
- The `.sg5` (games) file consists almost exclusively of 1-byte fields
  (like the older format); there is therefore no endianness ambiguity at
  this level, except for the diagonal Queen move case, where the 2nd byte
  is a raw 0-255 value (not really a multi-byte integer, so not affected
  either).

### 1.5 Pieces, squares and directions (used by move encoding, §4)

Piece types (colorless): `KING=1, QUEEN=2, ROOK=3, BISHOP=4, KNIGHT=5, PAWN=6`.

Squares: numbered 0 (square A1) to 63 (square H8), `square = (rank*8) +
file`, rank and file indexed 0-7 (file 0 = "a" file, rank 0 = rank "1").
`file(square) = square & 7`; `rank(square) = (square >> 3) & 7`.

---

## 2. `.si5` file (index) — fixed 56-byte records, no header

### 2.1 Complete absence of a header

The `.si5` file contains **no header at all**: no magic bytes, no version
number, no database description. The file is a raw array of 56-byte
records, starting at byte 0.

```
Number of games = file_size / 56   (must be an exact multiple of 56,
                                     otherwise the database is corrupt)
Position of game N's record (0-based) = 56 * N
```

The database's metadata (database type, description, autoload, labels of
the 6 custom flags) are stored in the `.sn5` file as special entries (see
§3.4) — there is therefore no "header vs. records" distinction within the
`.si5` itself: format identification and metadata are entirely
externalized.

> **Practical consequence** — Nothing in the `.si5` file itself allows
> one to distinguish an si5 database from any arbitrary binary file of
> the right size. It is the file extension (and the application context,
> which explicitly selects the SCID5 codec when opening) that determines
> this.

### 2.2 Index record (56 bytes), general structure

A record is made up of 12 32-bit words (48 bytes), each stored in
**little-endian** order, followed by 8 raw bytes of "home pawn" data. Each
32-bit word typically combines TWO pieces of information via simple bit
sharing: n high bits for a small field, and (32-n) low bits for an
identifier or a larger value. General formula for reading a word packed
as n/(32-n) bits:

```
large_field (32-n bits, low)  = word_value & ((1 << (32-n)) - 1)
small_field (n bits, high)    = word_value >> (32-n)
```

| Offset | Size | Content (high → low) |
|---:|---:|---|
| 0 | 4 | 4 bits: comment count (encoded) \| 28 bits: White player ID |
| 4 | 4 | 4 bits: variation count (encoded) \| 28 bits: Black player ID |
| 8 | 4 | 4 bits: NAG count (encoded) \| 28 bits: tournament (Event) ID |
| 12 | 4 | 32 bits: site ID (no bit sharing on this word) |
| 16 | 4 | 1 bit: variant (0=standard, 1=Chess960) \| 31 bits: round ID |
| 20 | 4 | 12 bits: White Elo \| 20 bits: game date |
| 24 | 4 | 12 bits: Black Elo \| 20 bits: tournament date (EventDate) |
| 28 | 4 | 10 bits: half-move count \| 22 bits: raw flags (bitmask) |
| 32 | 4 | 17 bits: game data length \| 15 bits: high bits (32-46) of the offset |
| 36 | 4 | 32 bits: low bits (0-31) of the offset within the `.sg5` |
| 40 | 4 | 8 bits: stored line code (StoredLineCode) \| 24 bits: final material signature |
| 44 | 4 | 8 bits: HomePawn entry count \| 3 bits: White Elo type \| 3 bits: Black Elo type \| 2 bits: result &nbsp;\|\|&nbsp; 16 bits: ECO code |
| 48 | 8 | Raw HomePawn data (8 bytes, without the count, which is in the previous word) |

**Total = 12×4 + 8 = 56 bytes.**

> **Note on the word at offset 44** — its high 16 bits themselves break
> down into 8+3+3+2 bits (HomePawn count, White Elo type, Black Elo type,
> result), while its low 16 bits carry the full ECO code (0-65535).

The game's offset within the `.sg5` is thus stored over **47 bits** in
total (15 high bits in the word at offset 32 + 32 low bits in the word at
offset 36), which allows addressing up to 2⁴⁷ bytes (128 TB).

### 2.3 Detail and semantics of each field

- **White / Black player ID** (28 bits): reference into the `.sn5`,
  PLAYER type (§3.2). Assigned sequentially when the name is inserted
  (§3.1).
- **Tournament ID** (Event, 28 bits), **site ID** (Site, 32 bits — the
  only identifier that occupies an entire word with no bit sharing),
  **round ID** (Round, 31 bits): references into the `.sn5`,
  EVENT/SITE/ROUND types.
- **"Variant" bit**: 0 = standard chess, 1 = Chess960. Moves remain
  encoded the same way in both cases (§4); only the starting position
  (which must then be non-standard, see §4.4) changes meaning: it
  represents one of the 960 possible starting positions instead of a
  "freely customized" position.
- **Game date / EventDate** (20 bits each): standard `dateT` encoding
  (§1.3), stored here **independently and absolutely**: the full 20 bits
  of `dateT` are used directly for each of the two dates, with no
  proximity limit between them.
- **White / Black Elo** (12 bits each, value 0-4000) and their rating
  type (3 bits each, values 0-7). Common type values: 0=Elo, 1=generic
  Rating, 2=Rapid, 3=ICCF, 4=USCF, 5=DWZ, 6=BCF.
- **Result** (2 bits): 0=none (`*`), 1=White (1-0), 2=Black (0-1),
  3=Draw (1/2-1/2).
- **Number of half-moves** in the main line (10 bits, 0-1023).
- **Flags** (22 raw bits, a single unified bit space — bitmask):

  | Bit | Value | Name | Meaning |
  |---:|---:|---|---|
  | 0 | 1 | `START` | Non-standard starting position |
  | 1 | 2 | `PROMO` | Contains at least one promotion |
  | 2 | 4 | `UPROMO` | Contains at least one under-promotion |
  | 3 | 8 | `DELETE` | Game marked for deletion |
  | 4 | 16 | `WHITE_OP` | "White opening" annotation |
  | 5 | 32 | `BLACK_OP` | "Black opening" annotation |
  | 6 | 64 | `MIDDLEGAME` | "Middlegame" annotation |
  | 7 | 128 | `ENDGAME` | "Endgame" annotation |
  | 8 | 256 | `NOVELTY` | "Theoretical novelty" annotation |
  | 9 | 512 | `PAWN` | "Pawn structure" annotation |
  | 10 | 1024 | `TACTICS` | "Tactics" annotation |
  | 11 | 2048 | `KSIDE` | "Kingside play" annotation |
  | 12 | 4096 | `QSIDE` | "Queenside play" annotation |
  | 13 | 8192 | `BRILLIANCY` | "Brilliancy" annotation |
  | 14 | 16384 | `BLUNDER` | "Blunder" annotation |
  | 15 | 32768 | `USER` | Generic user-defined flag |
  | 16 | 65536 | `CUSTOM1` | Custom flag #1 |
  | 17 | 131072 | `CUSTOM2` | Custom flag #2 |
  | 18 | 262144 | `CUSTOM3` | Custom flag #3 |
  | 19 | 524288 | `CUSTOM4` | Custom flag #4 |
  | 20 | 1048576 | `CUSTOM5` | Custom flag #5 |
  | 21 | 2097152 | `CUSTOM6` | Custom flag #6 |

  All 22 bits occupy a single word; the custom flags are no longer
  scattered into a separate byte as in the older format.

- **Length of the game's data** in the `.sg5` (17 bits, 0 to 131071
  bytes).
- **StoredLineCode** (8 bits): code (0 = none) identifying the longest
  pre-recorded opening line whose initial moves exactly match the
  beginning of the game. Purely informational heuristic field, used to
  speed up certain opening searches; may be set to 0 by a minimal writer.
- **Final material signature** (`FinalMatSig`, 24 bits): compact count of
  the material present on the board at the end of the game. Bit layout
  (LSB = bit 0): bits 0-3 = Black Pawn count (0-8), bits 4-5 = Black
  Knight count (capped at 3), bits 6-7 = Black Bishop count (capped at
  3), bits 8-9 = Black Rook count (capped at 3), bits 10-11 = Black Queen
  count (capped at 3), bits 12-15 = White Pawn count (0-8), bits 16-17 =
  White Knight count (capped at 3), bits 18-19 = White Bishop count
  (capped at 3), bits 20-21 = White Rook count (capped at 3), bits 22-23
  = White Queen count (capped at 3). Derived field, recomputable from the
  game's decoded final position.
- **Variation / comment / NAG counters** (4 bits each) — approximate,
  compressed representation of an actual count, using the following
  mapping table (4-bit value → actual count):

  | 4-bit code | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
  |---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
  | Actual count | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 15 | 20 | 30 | 40 | 50+ |

  Reciprocal encoding (actual count → 4-bit value): x≤10 → x; x≤12 → 10;
  x≤17 → 11; x≤24 → 12; x≤34 → 13; x≤44 → 14; otherwise → 15. These
  counters are rounded estimates intended for fast searches; the exact
  real count is obtained only by fully decoding the corresponding `.sg5`
  blob (§4).
- **ECO code** (16 bits, raw value, 0 = none).
- **HomePawn data** (1 count byte + 8 data bytes, 9 bytes total, split
  between the word at offset 44 — high part — and the final 8 bytes of
  the record): history of pawns having left their home square (a2-h2 /
  a7-h7). The count (0-16) indicates the number of valid entries among
  the 16 nibbles available in the 8 data bytes; each entry is a square
  index (0-15, encoded as a nibble) designating which pawn, among the 16
  original pawns, left its home square at that point in the game (the
  chronological order of changes is preserved). Derived heuristic field,
  used to speed up position/opening searches; may be left at 0 (zero
  count) by a minimal writer, or for a game with a non-standard starting
  position.

---

## 3. `.sn5` file (names and database metadata)

### 3.1 General principle: append-only log, no header

The `.sn5` is a simple stream of records appended one after another, in
**arrival order** (not alphabetically sorted), **without** prefix
compression, and **without** a header. Each record is read directly
after the previous one; reading stops at the end of the file (EOF).

The identifiers (`idNumberT`) of names of each type are **not** explicitly
stored in this file: they are assigned **implicitly and sequentially** in
insertion order, starting at 0, separately for each type (PLAYER, EVENT,
SITE, ROUND). The N-th record read for the PLAYER type (starting from 0)
automatically receives ID N.

### 3.2 Name types

| Code | Type |
|---:|---|
| 0 | `NAME_PLAYER` |
| 1 | `NAME_EVENT` |
| 2 | `NAME_SITE` |
| 3 | `NAME_ROUND` |
| 4 | `NAME_INFO` (technical pseudo-type: database metadata, see §3.4; is **not** a "name" type in the strict sense and does not get a sequential ID) |

### 3.3 Record format: varint (length*8 + type) followed by data

Each record begins with an integer encoded as a **varint** (LEB128-style,
7-bit groups, least significant first, high bit of each byte =
continuation flag):

```
varint_value = (string_length << 3) | type
```

that is, the 3 low-order bits of the decoded integer carry the name type
(0-4), and the rest of the integer (`value >> 3`) carries the length in
bytes of the string that immediately follows (without a stored null
terminator).

Decoding a varint: read bytes one at a time; each byte contributes its 7
low-order bits, concatenated in reading order (the first byte read = the
low-order bits of the final result); as long as the high bit (0x80) of
the byte read is 1, one must continue reading the next byte; as soon as a
byte has its high bit at 0, it is the last byte of this varint. Formally:

```
result = 0; shift = 0
repeat:
    byte = read_one_byte()
    result |= (byte & 0x7F) << shift
    shift += 7
while (byte & 0x80) != 0
type = result & 0b111
length = result >> 3
```

Encoding a varint (the reverse operation): while the value is ≥ 128,
write `(value & 0x7F) | 0x80` and shift the value right by 7 bits;
finally write the remaining value (< 128) as-is (high bit 0, ending the
encoding).

Immediately after the varint come the raw bytes of the string (length =
`varint_value >> 3` bytes, without a null terminator).

End of file = end of the record list (no explicit end marker; reading
simply stops at the file's EOF).

### 3.4 Special case: NAME_INFO type (4) — database metadata

Entries of type `NAME_INFO` do not represent a user-facing "name" but a
key/value pair concerning the database itself (the equivalent of the old
index header). The data string of these entries is the **direct
concatenation** of the key name and its value (no explicit separator
between the two):

```
stored_data = <key_name> <value>
```

where `<key_name>` is one of the following fixed strings, recognized by
prefix when reading (the key is stripped from the start of the string
read, the rest is the value):

```
"type", "description", "autoload",
"flag1", "flag2", "flag3", "flag4", "flag5", "flag6"
```

**Example:** to set the database description to "My test database", a
`NAME_INFO` (4) record is appended to the `.sn5` containing literally the
string `"descriptionMy test database"` (28 characters), preceded by the
varint `(28<<3)|4 = 228`.

A key may be written several times over time (the file being
append-only): the **last** occurrence encountered when reading the file
from start to end takes precedence (it overwrites the previous value in
memory). An unknown key (matching none of the prefixes in the list above)
is silently ignored on read — which allows new metadata fields to be
added in the future without breaking read compatibility with older
versions of the software.

### 3.5 Distinctive points of this name format

- No alphabetical sorting, no common-prefix compression: each name is
  stored in full, one after another, in the order it was added to the
  database.
- No usage-frequency counter stored per name: it can be recomputed on the
  fly by scanning the index if needed (counting the occurrences of each
  ID in the `.si5` records).
- The file is **purely append-only**: modifying an existing name in
  place is not possible; any correction would be done via a new record.

---

## 4. `.sg5` file (game data: moves, tags, comments, variations)

### 4.1 Low-level container

The `.sg5` file is a sequence of variable-length blobs, one per game,
located by (Offset, Length) in the corresponding `IndexEntry` of the
`.si5` (§2.2/§2.3). Space-management particularities:

- Data is grouped into virtual blocks of 131,072 bytes (128 KB — the same
  value as the per-game size limit).
- If a new game's data does not fit in the remaining space of the current
  block, the remaining space is filled with padding bytes (not
  meaningful) and the game is written at the start of the next block. The
  `.sg5` file may therefore contain "wasted" byte regions between two
  consecutive games when the second one had to be shifted to the next
  block. This has no effect on reading: each game remains addressed
  solely by its exact (Offset, Length) pair.
- Read access is via memory-mapping of the whole file; writing is
  append-only with an internal buffer, explicitly synchronized (flush)
  before memory-mapped reads become up to date.

### 4.2 Structure of a game blob — exact and mandatory order

| Step | Content | Reference |
|---|---|---|
| (a) | List of non-standard PGN tags | §4.3 |
| (b) | 1 byte of game flags | §4.4 |
| (c) | [optional] null-terminated FEN string | §4.4 |
| (d) | List of moves (variations/NAGs/comments as nested tokens), ended by the end-of-game token | §4.5 |
| (e) | Comments text block | §4.6 |

> **Note** — The standard PGN tags (the "Seven Tag Roster": Event, Site,
> Date, Round, White, Black, Result), as well as WhiteElo, BlackElo and
> ECO, are **not** in this blob: they are fully represented in the
> `IndexEntry` (`.si5`) and the name file (`.sn5`). Only extra PGN tags
> (Annotator, PlyCount, WhiteCountry, etc., and any custom tag) are
> stored in this blob.

### 4.3 (a) Encoding of non-standard PGN tags

For each extra tag, in the order they were added to the game:

**1. Encoding of the tag NAME**

| Case | Encoding |
|---|---|
| Common tag (exact match to table below) | 1 byte = 240 + (1-based position in the table), i.e. a value between 241 and 250 |
| Custom tag (any name, 1 to 240 characters) | 1 byte = name length (1-240), then N raw bytes = the name (no null terminator). An empty tag name (length 0) is not allowed: value 0 is reserved for the end-of-list marker. |

Table of "common tags" (on-disk codes 241-250):

| Code | Tag | Code | Tag | Code | Tag |
|---:|---|---:|---|---:|---|
| 241 | WhiteCountry | 245 | EventDate (text) | 249 | Source |
| 242 | BlackCountry | 246 | Opening | 250 | SetUp |
| 243 | Annotator | 247 | Variation | | |
| 244 | PlyCount | 248 | Setup | | |

> **Special legacy case, code 255 (0xFF)** — kept only for **read**
> compatibility with a very old format ("SCID2"): this code is not an
> index into the table above but signals a compact binary encoding of
> the EventDate over 3 additional raw bytes that immediately follow. A
> conformant writer does not need to produce this case (since the
> EventDate is natively a field of the `IndexEntry`, §2.3); a robust
> reader must nevertheless know how to recognize it and correctly
> consume/skip it.

**2. Encoding of the tag VALUE (always present)**

```
1 byte = value length (0-255)
then N raw bytes = the value (WITHOUT null terminator)
```

Exception: if the tag name was coded as 255, the value that follows is
**always** exactly 3 raw bytes, with no preceding length byte (see the
special case above).

**End of the list:** after the last tag (or immediately if there are no
extra tags), 1 byte with value 0x00.

### 4.4 (b)(c) Game flags and non-standard starting position

1 byte, bits used:

| Bit | Value | Name | Meaning |
|---:|---:|---|---|
| 0 | 1 | non-standard starting position | The game does not start from the standard chess starting position. For a Chess960 game (`IndexEntry`'s "variant" bit set to 1, §2.3), this bit is almost always also set to 1. |
| 1 | 2 | promotion flag | The game contains at least one promotion |
| 2 | 4 | under-promotion flag | The game contains at least one under-promotion (not a Queen) |
| 3-7 | — | unused | zero |

If bit 0 is set to 1, the following field is present: a null-terminated
(0x00) character string containing the full FEN representation (position,
side to move, castling rights, en-passant square, halfmove clock, full
move number) of the game's starting position.

### 4.5 (d) Encoding of the move list, variations, NAGs and comments

This section is encoded by a recursive traversal that processes the main
line (depth 0) and then each sub-variation encountered (depth > 0), in
the order in which they appear:

1. If a comment precedes the very first move of this line/variation (a
   "pre-game comment" or "start-of-variation comment"): emit the
   `ENCODE_COMMENT` token (byte value = 12). This token only indicates
   the presence of a comment; the actual text will be in the final block
   (§4.6), read in the same traversal order ("with markers" mode — see
   the important nuance at the end of this section).
2. For each move in the line, until the end of this line/variation:
   1. write the encoded move (1 byte, or 2 bytes for a diagonal Queen
      move — §4.5.1);
   2. for each NAG attached to this move: emit `ENCODE_NAG` (value 11)
      followed by 1 byte = NAG code (value between 0 and 215);
   3. if this move has a comment: emit `ENCODE_COMMENT` (value 12)
      [marker only, text in the final block];
   4. for each sub-variation branching from this move (0 or more, in
      order): emit `ENCODE_START_MARKER` (value 13), then recursively
      encode the content of this sub-variation; at its end emit
      `ENCODE_END_MARKER` (value 14) — **except** if this sub-variation
      is the very last thing in the entire blob (absolute end of the
      game), in which case this variation-end token is omitted since it
      is immediately followed by the end-of-game token.
3. At the end of the current line/variation (main line, depth 0): emit
   `ENCODE_END_GAME` (value 15).

Table of control tokens (full byte values, not simple nibbles):

| Value | Token | Detail |
|---:|---|---|
| 11 | `ENCODE_NAG` | followed by 1 byte: NAG code |
| 12 | `ENCODE_COMMENT` | presence marker, no inline data |
| 13 | `ENCODE_START_MARKER` | start of a sub-variation |
| 14 | `ENCODE_END_MARKER` | end of a sub-variation |
| 15 | `ENCODE_END_GAME` | end of the main line / of the game |

These 5 values are reserved with **no possible ambiguity** with an actual
move: a move byte always equals `(pieceIndex << 4) | code`, and the King
always occupies piece index 0. The King only uses codes 0 to 10 (§4.5.1).
A full byte with a value of 11 to 15 would therefore correspond to "King
(index 0), code 11-15", a combination never produced by an actual King
move — it is therefore exclusively reserved for the control tokens above.

#### 4.5.1 Detailed move encoding

General formula for a move byte:

```
byte = ((pieceIndex & 0x0F) << 4) | (code & 0x0F)
```

**pieceIndex** (high 4 bits, 0-15) designates which piece of the side to
move is moving, by its index in the list of (up to 16) pieces of that
side (index 0 = always the King; indices 1-15 correspond to the 15 other
original pieces of the side, including the 8 pawns; a promoted pawn keeps
its original index but changes type). This is **not** the piece's type.

> **Non-obvious behavior — index remapping on capture** — This index →
> square mapping is **not** fixed for the whole game: it matches the 16
> starting squares only at the very beginning of the game (or, for a
> non-standard starting position, the assignment order described in
> §4.4). It then changes on every normal move (the square of the moving
> piece is simply updated) and, less obviously, on **every capture**: the
> index that becomes free (the captured piece's index) is reassigned to
> whichever piece currently holds the highest active index of its own
> side — removal by "swap with the last active element" of a
> variable-size list. A decoder that assumes this mapping stays stable
> for the whole game will produce incorrect moves or a visibly corrupted
> stream as soon as a capture affects a higher-indexed piece — including
> for a Rook castling later in the game: its index cannot be assumed
> fixed (1 or 7) either, beyond the very first capture. This behavior,
> identical to si4, is verified against the reference implementation
> (`position.cpp`, `Position::DoSimpleMove`, capture-handling section);
> not described elsewhere in this document.

**code** (low 4 bits, 0-15) has a meaning that depends on the type of the
piece that is moving (determined during decoding by looking up the
current board at the source square):

**KING** (pieceIndex always 0):

| Square difference (destination − source) | Code |
|---|---:|
| -9 | 1 |
| -8 | 2 |
| -7 | 3 |
| -1 | 4 |
| +1 | 5 |
| +7 | 6 |
| +8 | 7 |
| +9 | 8 |
| -2 (queenside castling) | 9 |
| +2 (kingside castling) | 10 |

Null move (destination==source): the **entire byte equals 0x00**
(pieceIndex=0, code=0). Codes 11-15 are never used (reserved for the
control tokens).

**KNIGHT:**

| Square difference | Code |
|---|---:|
| -17 | 1 |
| -15 | 2 |
| -10 | 3 |
| -6 | 4 |
| +6 | 5 |
| +10 | 6 |
| +15 | 7 |
| +17 | 8 |

**ROOK:**

| Condition | Code |
|---|---|
| Same rank as source square | `code = file(destination)` (0-7, 0=a-file, 7=h-file) |
| Same file as source square | `code = 8 + rank(destination)` (8-15, rank 0 = rank 1) |

**BISHOP:** `code = file(destination)`, plus 8 if the diagonal taken is
of the "up-left/down-right" type (product of the rank and file
differences is negative). Gives a code of 0-7 for an "up-right/down-left"
diagonal, 8-15 for the other diagonal.

**QUEEN** — the only piece that can occupy 2 bytes:

| Move type | Encoding |
|---|---|
| "Rook-like" move (same rank or same file) | Identical to the Rook encoding above (a single byte, code 0-15) |
| Diagonal move (2 bytes) | 1st byte = pieceIndex/file(source) (a sentinel, normally invalid as a Rook move); 2nd byte = (destination square) + 64, raw byte (64-127) |

**Decoding rule:** if code(1st byte) ≥ 8 → vertical move; otherwise if
code ≠ file(source) → horizontal move; otherwise (code==file(source)) →
read a 2nd byte, destination square = (value of the 2nd byte) − 64.

**PAWN** — code 0-15, combining the move type and an optional promotion:

| Code | Move type |
|---:|---|
| 0 | capture left (no promotion) |
| 1 | one-square advance (no promotion) |
| 2 | capture right (no promotion) |
| 3, 4, 5 | same as 0, 1, 2 respectively, with promotion to Queen |
| 6, 7, 8 | same as 0, 1, 2 respectively, with promotion to Rook |
| 9, 10, 11 | same as 0, 1, 2 respectively, with promotion to Bishop |
| 12, 13, 14 | same as 0, 1, 2 respectively, with promotion to Knight |
| 15 | two-square advance (only from starting square; never combined with a promotion) |

```
code = (0|1|2 depending on move type) + 3 * (promotion rank)
promotion rank: 0=none, 1=Queen, 2=Rook, 3=Bishop, 4=Knight
```

> **Encoded move size** — 1 byte in all cases, **except** the diagonal
> Queen move, which occupies exactly 2 bytes.

### 4.6 (e) Comments text block

After the end-of-game token (`ENCODE_END_GAME`), a final block contains
the text of the game's comments (main line and all sub-variations
combined), each comment being written as a character string
**terminated by a null byte** (0x00).

**Normal mode ("with markers", the one described in §4.5):** there are
exactly as many strings in this block as `ENCODE_COMMENT` tokens emitted
in section (d), in the same traversal order (before the game's/
variation's first move if there is a pre-game comment, then after each
marked move, recursing down into variations in order).

> **Important nuance — a second possible mode ("without markers")** —
> The format also provides for an alternative, more compact mode for
> games that are almost entirely commented, where no `ENCODE_COMMENT`
> token is emitted in the move list; instead, the comments block contains
> ONE null-terminated string PER MOVE of the game (an empty string for a
> move without a comment), in the natural move order. A decoder **must**
> handle both cases, including a **mixed** case (some comments explicitly
> marked at the start of decoding, then, beyond the last explicit marker,
> the remaining strings in the block are assigned sequentially, one at a
> time, to the following moves in order — whether or not they have a
> comment).

Robust decoding algorithm covering both cases:

1. Decode the move list (§4.5). Every time an `ENCODE_COMMENT` token is
   encountered, record (in an ordered list) the move it relates to.
2. Read the comments block (e): first, in order, one string for each
   move recorded in step 1 (the classic "with markers" mode).
3. If there are unconsumed strings left in the block after processing
   all explicit markers: assign them sequentially, one per move,
   starting from the move immediately following the last marked move
   (or from the very first move of the game if no marker was
   encountered in step 1), and advancing move by move until the
   remaining strings are exhausted.

As of today, in the reference code, only the classic "with markers" mode
is actually **produced** when writing (the choice of the "without
markers" mode, although implemented and available in the code, is
explicitly forced in favor of the "with markers" mode there, in order to
remain bit-compatible with the older si4 format). A writer aiming only
for compatibility with si5 databases actually produced today can
therefore limit itself to the "with markers" mode; a decoder aiming to be
robust against a possible future evolution must, however, implement both.

---

## 5. Operational summary: reading / writing an si5 database

### 5.1 To READ an si5 database

1. Open the `.si5`: `n_games = file_size / 56`. Read sequentially (or
   via direct access at position `56*N`) each 56-byte record and decode
   it per §2.2/§2.3.
2. Open and fully read the `.sn5` from the start to EOF: for each
   record, decode the varint (type + length), read the string, and
   depending on the type either add it to the name table of the
   corresponding type (ID = sequential insertion position), or
   (`NAME_INFO` type) update the corresponding database metadata by
   prefix.
3. For a given game N: resolve its 5 name identifiers via the `.sn5`
   tables, then read Length bytes at Offset in the `.sg5` (memory
   mapping or direct read) and decode the blob per §4.2 to §4.6 (tags,
   flags, optional FEN, moves/variations/NAGs, then comments).

### 5.2 To WRITE (append a game to) an si5 database

1. Resolve or create the 5 needed name identifiers (White, Black,
   Event, Site, Round): for a new name, append it to the end of the
   `.sn5` (varint + raw string) and assign it the next available ID for
   its type (= number of names already stored for that type).
2. Encode the game's content into a binary blob per §4.2 to §4.6 (tags,
   flags, optional FEN, moves/variations/NAGs/comments, in "with
   markers" mode for maximum compatibility).
3. Write this blob into the `.sg5`: if the remaining space in the
   current 128 KB block is insufficient, fill the rest of the block
   with padding then start over at the beginning of the next block;
   note the actual write Offset.
4. Build the 56-byte record (§2.2/§2.3) with all fields (Offset,
   Length, name IDs, absolute dates, Elo, ECO, Result, flags unified
   over 22 bits, 4-bit counters, variant bit, NumHalfMoves,
   HomePawnData, FinalMatSig, StoredLineCode) and append it to the end
   of the `.si5` (at position `56 * current_number_of_games`).
5. Synchronize (flush) the three files.

---

## 6. Summary of analyzed source files

| File | Content |
|---|---|
| `src/codec.h` | Abstract `ICodecDatabase` interface (Codec MEMORY/PGN/SCID4/SCID5), common entry point |
| `src/codec_scid5.h` | Complete si5 codec implementation (header-only): `encode_IndexEntry`/`decode_IndexEntry` (§2), `.sn5` handling (§3), `.sg5` block management (§4.1) |
| `src/codec_scid4.cpp` | Reimplementation of the legacy si4 codec (allows direct comparison to confirm the elements carried over identically into si5) |
| `src/indexentry.h` | Unified `IndexEntry` class (C++ bitfields), used in memory by both codecs |
| `src/index.h` | `Index` class: in-memory container agnostic of the on-disk format (chunked vector of `IndexEntry`) |
| `src/namebase.h` | `NameBase` class: in-memory name container + `TagRoster` struct (referenced Seven Tag Roster) |
| `src/bytebuf.h` | Logic for decoding tags, the starting position and moves (`encodeTags`/`decodeTags`/`decodeMove`/`nextMove` functions, `commonTags[]` table) |
| `src/game.cpp` | Complete logic for encoding a game (`encodeMove`, `encodeMovelist`, `encodeComments`, `countComments`, `Game::Encode`, `mainlineInfo`) and decoding (`Game::DecodeVariation`, `decodeComments`) — source of the detail in §4.6 on the two comment-encoding modes |
| `src/filebuf.h` | Buffered low-level I/O, big-endian |
| `src/filemap.h` | Memory-mapped I/O + append buffer, used for the `.sg5` |
| `src/common.h` | Basic types |

---

*See also: [`scid-si4-specification.md`](scid-si4-specification.md) — the
predecessor format, and [`ARCHITECTURE.md`](ARCHITECTURE.md) for how the
`scid` crate fits into the rest of Vendetta Chess GUI.*
