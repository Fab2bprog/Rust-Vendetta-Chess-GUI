# SCID `si4` Database Format — Technical Specification

> **Reverse-engineering notice** — This specification was produced through
> independent reverse engineering of the reference SCID C++ source code
> (`scidvspc-code-r3655-si4`, `src/` tree), reflecting the exact read/write
> behavior implemented by `Index::*`, `IndexEntry::*`, `NameBase::*`,
> `GFile::*` and `Game::Encode`/`Decode`. It is **not** an official document
> of the SCID project. (*Author: Fabrice Garcia*)

Format version 4.0 / `SCID_VERSION=400`. Companion document to the
`scid` crate (Vendetta Chess GUI project).

## Table of contents

1. [Basic types and global constants](#1-basic-types-and-global-constants)
2. [`.si4` file (index)](#2-si4-file-index)
3. [`.sn4` file (names: players, tournaments, sites, rounds)](#3-sn4-file-names-players-tournaments-sites-rounds)
4. [`.sg4` file (game data: moves, tags, comments, variations)](#4-sg4-file-game-data-moves-tags-comments-variations)
5. [Derived / optional fields (search optimization)](#5-derived--optional-fields-search-optimization)
6. [Complete algorithm for creating an si4 database](#6-complete-algorithm-for-creating-an-si4-database)
7. [Summary of analyzed source files](#7-summary-of-analyzed-source-files)

**General convention:** all multi-byte integers in this format (across the
three files `.si4`, `.sn4`, `.sg4`) are stored in **big-endian** order (most
significant byte first), written/read via the `WriteTwoBytes` /
`WriteThreeBytes` / `WriteFourBytes` primitives and their `Read*`
counterparts, which stack bytes using 8-bit shifts starting from the most
significant byte. There is no particular alignment (no structural padding):
each field is concatenated immediately after the previous one.

A SCID si4 database consists of **three files** sharing the same base name
with different suffixes:

| Suffix | Role |
|---|---|
| `<base>.si4` | Index file — per-game metadata + database header |
| `<base>.sn4` | Name file — players, tournaments, sites, rounds |
| `<base>.sg4` | Game file — moves, comments, variations, extra PGN tags |

> **Note** — None of the three files is usable on its own: the `.si4`
> references name IDs resolved in the `.sn4`, and references data blocks
> (offset + length) located in the `.sg4`.

---

## 1. Basic types and global constants

### 1.1 Types

| Type | Definition |
|---|---|
| `byte` | unsigned integer, 8 bits |
| `ushort` | unsigned integer, 16 bits |
| `uint` | unsigned integer, 32 bits |
| `versionT` | `ushort` (2 bytes) |
| `gameNumberT` | `uint` (game identifier/number, 0-based; on disk often truncated to 3 bytes) |
| `idNumberT` | `uint` (name identifier; on disk packed into 16 to 20 bits depending on the field, see §2.4) |
| `dateT` | `uint`, compact encoding described in §1.3 |
| `eloT` | `ushort`, encoding described in §2.6 |
| `ecoT` | `ushort` (raw ECO code) |
| `resultT` | `byte`: 0=no result (`*`), 1=White wins (1-0), 2=Black wins (0-1), 3=Draw (1/2-1/2) |
| `matSigT` | `uint`, encoding described in §5.2 |

### 1.2 Format constants

| Constant | Value | Meaning |
|---|---:|---|
| `SCID_VERSION` | 400 | Current format version "4.0"; the si4 format requires exactly this value, no forward/backward compatibility |
| `MAX_GAMES` | 16,777,214 | 2²⁴ − 1 − 1; limit due to the `numGames` field being 3 bytes, minus 1 for the special semantics of the `autoLoad` field |
| `MAX_GAME_LENGTH` | 131,072 | Maximum size in bytes of an encoded game blob in the `.sg4` (17 usable bits) |
| `MAX_ELO` | 4000 | Elo stored in 12 bits |

### 1.3 Date encoding (`dateT`, 32 bits, only 20 bits used for a "simple" game date)

A date encodes year/month/day in a single integer:

| Bits | Field | Range |
|---|---|---|
| 0-4 (5 bits) | day of month | 0-31 (0 = unknown day) |
| 5-8 (4 bits) | month | 0-15 (0 = unknown month) |
| 9-19 (11 bits) | year | 0-2047 (0 = unknown year) |

```
date = (year << 9) | (month << 5) | day
ZERO_DATE = 0   // no date information
```

A numerically larger date is chronologically more recent.

---

## 2. `.si4` file (index)

### 2.1 Constants

| Constant | Value |
|---|---|
| Suffix | `.si4` |
| Magic (8 bytes) | `53 63 69 64 2E 73 69 00` (hex) = `"Scid.si\0"` (ASCII) |
| `INDEX_HEADER_SIZE` | 182 bytes |
| `INDEX_ENTRY_SIZE` | 47 bytes (per game) |

### 2.2 File header (182 bytes, at the start of the file, exact order)

| Offset | Size | Field | Description |
|---:|---:|---|---|
| 0 | 8 | `magic` | Raw bytes `"Scid.si\0"` (with the final `\0` included in the 8 bytes) |
| 8 | 2 | `version` | `versionT`, big-endian. Must be 400 (0x0190) for a valid, writable si4 base. |
| 10 | 4 | `baseType` | `uint`, big-endian. Database type (tournament, theory, etc.); in practice 0 for most databases. |
| 14 | 3 | `numGames` | `uint`, big-endian, 3 bytes: number of games currently stored. |
| 17 | 3 | `autoLoad` | `uint`, big-endian, 3 bytes: number of the game to auto-load. 0 or 2 ⇒ load game 1 (0-based #0); 1 ⇒ no autoload; N (N≥2) ⇒ load game (N−1), 0-based. |
| 20 | 108 | `description` | C string (107 usable characters + 1 final null byte). Free-form description of the database. |
| 128 | 54 | `customFlagDesc[6]` | 6 blocks of 9 bytes each (8 usable characters + 1 null byte). Label for each of the 6 "custom flags" (§2.5). |

**Total = 182 bytes** (8+2+4+3+3+108+54).

> **Validation on open** — The magic must match exactly; the version field
> must be between `SCID_OLDEST_VERSION` (400) and `SCID_VERSION` (400)
> inclusive — in practice **exactly 400** for a conformant si4
> readable/writable by the current reference version.

### 2.3 Index record (`IndexEntry`) — 47 bytes per game

Located immediately after the header, at position **182 + 47 × game_number**
(0-based). Exact on-disk field order (confirmed by `IndexEntry::Read`/`Write`):

| Offset | Size | Field | Description |
|---:|---:|---|---|
| 0 | 4 | `Offset` | `uint`, big-endian: byte position of the game blob within the `.sg4`. |
| 4 | 2 | `Length_Low` | `ushort`, big-endian: low 16 bits of the blob's length in the `.sg4`. |
| 6 | 1 | `Length_High` | byte: see combined decoding, §2.5. |
| 7 | 2 | `Flags` | `ushort`, big-endian: see §2.5. |
| 9 | 1 | `WhiteBlack_High` | byte: high bits of the White/Black IDs. |
| 10 | 2 | `WhiteID_Low` | `ushort`, big-endian: low 16 bits of the White player's ID. |
| 12 | 2 | `BlackID_Low` | `ushort`, big-endian: low 16 bits of the Black player's ID. |
| 14 | 1 | `EventSiteRnd_High` | byte: high bits of the Event/Site/Round IDs. |
| 15 | 2 | `EventID_Low` | `ushort`, big-endian: low 16 bits of the event (tournament) ID. |
| 17 | 2 | `SiteID_Low` | `ushort`, big-endian: low 16 bits of the site ID. |
| 19 | 2 | `RoundID_Low` | `ushort`, big-endian: low 16 bits of the round ID. |
| 21 | 2 | `VarCounts` | `ushort`, big-endian: see decoding §2.7. |
| 23 | 2 | `EcoCode` | `ushort`, big-endian: raw ECO code (0 = none). |
| 25 | 4 | `Dates` | `uint`, big-endian: combined Date + EventDate, see §2.8. |
| 29 | 2 | `WhiteElo` | `ushort`, big-endian: see §2.6. |
| 31 | 2 | `BlackElo` | `ushort`, big-endian: see §2.6. |
| 33 | 4 | `FinalMatSig` | `uint`, big-endian: material signature (low 24 bits, §5.2) + `StoredLineCode` (high 8 bits). |
| 37 | 1 | `NumHalfMoves` (low) | byte: low 8 bits of the half-move count. |
| 38 | 1 | (combined) | bits 0-5 = `HomePawnData[0]` (6 usable bits); bits 6-7 = high bits 8-9 of `NumHalfMoves`. |
| 39 | 8 | `HomePawnData[1..8]` | 8 raw bytes (see §5.3). |

**Total = 47 bytes** (4+2+1+2+1+2+2+1+2+2+2+2+2+4+2+2+4+1+1+8).

```
// Combined byte at offset 38:
actual NumHalfMoves (10 bits, 0-1023) = byte[37] | ((byte[38] >> 6) << 8)
actual HomePawnData[0]                = byte[38] & 0x3F
```

> **Contractual order** — This on-disk order is **different** from the
> order in which the fields are declared in the C++ definition of the
> `IndexEntry` class; the order described above (that of the
> `Read()`/`Write()` functions) is the real, contractual on-disk order, and
> **must** be reproduced identically by any compatible implementation.

### 2.4 Decoding of name identifiers (16+4 / 16+3 / 16+2 bit packing)

Each `IndexEntry` references 5 identifiers (indices into the `.sn4` file,
separately per name type — see §3), each stored in two parts: a 16-bit low
part in a dedicated field, and a 2-to-4-bit high part taken from a byte
shared with another identifier.

| Identifier | Width | Formula |
|---|---|---|
| White ID | 20 bits | `(WhiteBlack_High >> 4) << 16 \| WhiteID_Low` |
| Black ID | 20 bits | `(WhiteBlack_High & 0x0F) << 16 \| BlackID_Low` |
| Event ID | 19 bits | `(EventSiteRnd_High >> 5) << 16 \| EventID_Low` |
| Site ID | 19 bits | `((EventSiteRnd_High >> 2) & 7) << 16 \| SiteID_Low` |
| Round ID | 18 bits | `(EventSiteRnd_High & 3) << 16 \| RoundID_Low` |

Writing (updating a shared "High" byte without disturbing the other half):

```
WhiteBlack_High   : bits 4-7 = White ID >> 16 ; bits 0-3 = Black ID >> 16
EventSiteRnd_High : bits 5-7 = Event ID >> 16 ; bits 2-4 = Site ID >> 16 ; bits 0-1 = Round ID >> 16
```

### 2.5 Flags field (`ushort`, bitmask)

Per-game indicators stored in the `IndexEntry` (**independent** of the
`.sg4` blob's encoding flags, §4.4):

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

The 6 "custom flags" (labeled in the header, §2.2) are **not** in this
Flags field: they occupy bits 0-5 of the `Length_High` byte (offset 6 of
the `IndexEntry`): custom flag *n* (1≤n≤6) = bit (n−1) of the
`Length_High` byte. Bit 7 of `Length_High` carries the 17th (most
significant) bit of the length; bit 6 is unused/reserved.

```
// Actual length of the .sg4 blob (17 bits, up to 131071 bytes):
Length = Length_Low + ((Length_High & 0x80) << 9)
```

### 2.6 Elo encoding (`WhiteElo` / `BlackElo`, `ushort`)

| Bits | Field | Detail |
|---|---|---|
| 0-11 (12 bits) | Elo value | 0 to 4000 (`MAX_ELO`) |
| 12-15 (4 bits) | `RatingType` | 0=Elo, 1=Rating (generic), 2=Rapid, 3=ICCF, 4=USCF, 5=DWZ, 6=BCF |

### 2.7 VarCounts encoding (`ushort`)

Approximate counters compressed into 4 bits each, plus the game result:

| Bits | Field |
|---|---|
| 0-3 | number of variations (encoded, see mapping table) |
| 4-7 | number of comments (encoded) |
| 8-11 | number of NAGs (encoded) |
| 12-15 | `resultT` (game result, 0-3, see §1.1) |

Decoding table (4-bit value → actual count):

| 4-bit code | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Actual count | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 15 | 20 | 30 | 40 | 50+ |

Encoding (actual count → 4-bit value): x≤10 → x; x≤12 → 10; x≤17 → 11;
x≤24 → 12; x≤34 → 13; x≤44 → 14; otherwise → 15.

> **Note** — These counters are rounded **estimates** intended for fast
> searches; the exact real number of variations/comments/NAGs is obtained
> only by fully decoding the corresponding `.sg4` blob.

### 2.8 Dates field encoding (`uint`, 32 bits) — combined game Date + EventDate

| Bits | Field |
|---|---|
| 0-19 | game Date, standard `dateT` encoding (§1.3) |
| 20-31 | "codedDate" for the EventDate (tournament start date), relative to the game's year |

```
codedDate = (month << 5) | day | (((eventYear + 4 - gameYear) & 7) << 9)
  bits 0-4  (5 bits) : EventDate day
  bits 5-8  (4 bits) : EventDate month
  bits 9-11 (3 bits) : coded year offset = (eventYear - gameYear) + 4
```

If `codedDate == 0` (year offset = 0), no EventDate is stored.

> **Constraint** — The EventDate can only be encoded if its year is within
> 3 years of the game's year (`|eventYear − gameYear| ≤ 3`); otherwise it
> is silently omitted (equivalent to `ZERO_DATE`). For a more distant
> EventDate, only the "EventDate" text PGN tag (a non-standard `.sg4` tag,
> §4.3) can preserve it faithfully.

Decoding:

```
gameYear = Date >> 9                       (over the full low 20 bits)
eventMonth = (codedDate >> 5) & 15 ; eventDay = codedDate & 31
offset = (codedDate >> 9) & 7 ; if offset==0 -> no EventDate
eventYear = gameYear + offset - 4
```

---

## 3. `.sn4` file (names: players, tournaments, sites, rounds)

### 3.1 Constants

| Constant | Value |
|---|---|
| Suffix | `.sn4` |
| Magic (8 bytes) | `"Scid.sn\0"` (7 ASCII characters + null byte) |

4 name types, throughout the whole format, always in this order:

| Code | Type |
|---:|---|
| 0 | PLAYER |
| 1 | EVENT (tournament) |
| 2 | SITE |
| 3 | ROUND |

| Name type | Maximum count | Constraint source |
|---|---:|---|
| PLAYER | 2²⁰ − 1 = 1,048,575 | White/Black ID packing, §2.4 |
| EVENT | 2¹⁹ − 1 = 524,287 | Event ID packing, §2.4 |
| SITE | 2¹⁹ − 1 = 524,287 | Site ID packing, §2.4 |
| ROUND | 2¹⁸ − 1 = 262,143 | Round ID packing, §2.4 |

### 3.2 File header (36 bytes, exact order)

| Offset | Size | Field | Description |
|---:|---:|---|---|
| 0 | 8 | `magic` | `"Scid.sn\0"` (raw bytes) |
| 8 | 4 | `timeStamp` | `uint`, big-endian (timestamp, often 0) |
| 12 | 3 | `numNames[PLAYER]` | `uint`, big-endian, 3 bytes |
| 15 | 3 | `numNames[EVENT]` | same |
| 18 | 3 | `numNames[SITE]` | same |
| 21 | 3 | `numNames[ROUND]` | same |
| 24 | 3 | `maxFrequency[PLAYER]` | `uint`, big-endian, 3 bytes (maximum usage frequency among names of this type, across the whole database) |
| 27 | 3 | `maxFrequency[EVENT]` | same |
| 30 | 3 | `maxFrequency[SITE]` | same |
| 33 | 3 | `maxFrequency[ROUND]` | same |

**Total = 36 bytes** (8+4+3×4+3×4).

### 3.3 File body: front-coded, alphabetically sorted records

For each name type, in the order PLAYER, then EVENT, then SITE, then
ROUND, exactly `numNames[type]` consecutive records, **sorted
alphabetically** (not by ID order), with common-prefix compression
("front coding").

For the i-th name (i = 0-based, in alphabetical order) of this type:

| Field | Size rule |
|---|---|
| a. Identifier (`idNumberT`) | 3 bytes if `numNames[type] ≥ 65536`, else 2 bytes (big-endian). Depends on the TOTAL number of names of this type. |
| b. Usage frequency | 3 bytes if `maxFrequency[type] ≥ 65536`; else 2 bytes if `≥ 256`; else 1 byte. Depends on the global MAXIMUM frequency for this type. |
| c. Total length of the name | 1 raw byte (0-255) |
| d. Common-prefix length with preceding name | 1 raw byte. Absent for the very first name (i=0) of each type. |
| e. Non-shared suffix of the name | (total_length − prefix_length) raw bytes, no null terminator |

Reconstruction on read: `full_name = (first "prefix_length" characters of
the preceding name in alphabetical order) + (the read suffix)`.

> **Writer requirement** — The names of each type must imperatively be
> sorted alphabetically **before** being front-coded and written,
> otherwise prefix reconstruction would be incorrect.

---

## 4. `.sg4` file (game data: moves, tags, comments, variations)

### 4.1 Low-level container

The `.sg4` file is a plain **raw concatenation** of variable-length blobs,
one per game, without a file header, without delimiters between blobs,
without padding. Each blob is located exclusively by the (Offset, Length)
pair of the corresponding `IndexEntry` in the `.si4` (§2.3).

A conformant writer can therefore simply append each blob after the
previous one (Offset of blob N+1 = Offset of blob N + Length of blob N for
a simple sequential append), and write the resulting Offset into the
`IndexEntry`.

> **Implementation detail** — The reference implementation uses a
> 131072-byte cache-block mechanism (`GF_BLOCKSIZE`, equal to
> `MAX_GAME_LENGTH`), but this is an I/O optimization internal to this
> specific implementation and does not affect the actual binary structure
> of the file, as long as one writes/reads the exact bytes at position
> Offset over Length bytes.

**Note on modifying an existing database:** when an existing game is
replaced by a version of a different size, the reference implementation
does not overwrite the bytes in place but appends the new version at the
end of the file (updating Offset and Length in the `IndexEntry`); the old
bytes then become unreferenced "garbage", reclaimed only by a compaction
operation (complete rewrite of all three files from scratch). This only
concerns editing, not the read structure of the format.

### 4.2 Structure of a game blob — exact and mandatory order

Produced by `Game::Encode`, consumed by `Game::Decode`:

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
> `IndexEntry` (`.si4`) and the name file (`.sn4`). Only extra PGN tags
> (Annotator, PlyCount, WhiteCountry, etc., and any custom tag) are stored
> in this blob.

### 4.3 (a) Encoding of non-standard PGN tags

For each extra tag (up to 40, `MAX_TAGS`), in the order they were added to
the game:

**1. Encoding of the tag NAME**

| Case | Encoding |
|---|---|
| Common tag (exact match to table below) | 1 byte = 240 + (1-based position in the table), i.e. a value between 241 and 254 |
| Custom tag (any other name) | 1 byte = name length (0-240), then N raw bytes = the name (no null terminator) |

Table of "common tags" (`MAX_TAG_LEN`=240; on-disk codes = 241+index):

| Code | Tag | Code | Tag | Code | Tag |
|---:|---|---:|---|---:|---|
| 241 | WhiteCountry | 245 | EventDate (text) | 249 | Source |
| 242 | BlackCountry | 246 | Opening | 250 | SetUp |
| 243 | Annotator | 247 | Variation | 251-254 | reserved (unused) |
| 244 | PlyCount | 248 | Setup | | |

> **Special case — code 255 (0xFF)** — Is **not** an index into the table
> above but signals a compact binary encoding of the EventDate over 3
> additional raw bytes that immediately follow (a 24-bit `dateT`,
> reconstructed by 3 successive 8-bit shifts). This test (b==255) takes
> priority over the "common tag" test in the decoder.

**2. Encoding of the tag VALUE (always present)**

```
1 byte = value length (0-254)
then N raw bytes = the value (WITHOUT null terminator)
```

**End of the list:** after the last tag (or immediately if there are no
extra tags), 1 byte with value 0x00.

### 4.4 (b)(c) Game flags and non-standard starting position

1 byte, bits used:

| Bit | Value | Name | Meaning |
|---:|---:|---|---|
| 0 | 1 | `NonStandardStart` | The game does not start from the standard chess starting position |
| 1 | 2 | `PromotionsFlag` | The game contains at least one promotion |
| 2 | 4 | `UnderPromosFlag` | The game contains at least one under-promotion (not a Queen) |
| 3-7 | — | unused | zero |

> **Not the same as IndexEntry flags** — These are **not** the same flags
> as those of the `IndexEntry` (§2.5), even though Start/Promo/UnderPromo
> are duplicated there for fast search purposes (`ie->SetStartFlag` /
> `SetPromotionsFlag` / `SetUnderPromoFlag`).

If the `NonStandardStart` bit is set to 1, the following field is present:
a null-terminated (0x00) character string containing the full FEN
representation (all fields: position, side to move, castling rights,
en-passant square, halfmove clock, full move number) of the game's
starting position.

### 4.5 (d) Encoding of the move list, variations, NAGs and comments

This section is encoded by a recursive function that processes the main
line (depth 0) and then each sub-variation encountered (depth > 0), in the
order in which they appear:

1. If a comment precedes the very first move of this line/variation (a
   "pre-game comment" or "start-of-variation comment"): emit the
   `ENCODE_COMMENT` token (byte value = 12). This token only indicates the
   presence of a comment; the actual text will be in the final block
   (§4.6), read in the same traversal order.
2. For each move in the line, until the end of this line/variation:
   1. write the encoded move (1 byte, or 2 bytes for a diagonal Queen
      move — §4.5.1);
   2. for each NAG attached to this move (up to 8 per move): emit
      `ENCODE_NAG` (value 11) followed by 1 byte = NAG code (0-215);
   3. if this move has a comment: emit `ENCODE_COMMENT` (value 12)
      [marker only];
   4. for each sub-variation branching from this move (0 or more, in
      order): emit `ENCODE_START_MARKER` (value 13), then recursively
      encode the sub-variation's content (ending with its own
      `ENCODE_END_MARKER` since its depth > 0).
3. At the end of the current line/variation: if depth == 0 (main line),
   emit `ENCODE_END_GAME` (value 15); otherwise (sub-variation), emit
   `ENCODE_END_MARKER` (value 14).

> **Why two end tokens** — Having two distinct end tokens lets the decoder
> detect corruption: a game **must** end with `ENCODE_END_GAME`, never
> with `ENCODE_END_MARKER`, and vice versa for a sub-variation.

Table of control tokens (full byte values, not simple nibbles):

| Value | Token | Detail |
|---:|---|---|
| 11 | `ENCODE_NAG` | followed by 1 byte: NAG code |
| 12 | `ENCODE_COMMENT` | presence marker, no inline data |
| 13 | `ENCODE_START_MARKER` | start of a sub-variation |
| 14 | `ENCODE_END_MARKER` | end of a sub-variation |
| 15 | `ENCODE_END_GAME` | end of the main line / of the game |

These 5 values are reserved with **no possible ambiguity** with an actual
move, for the following reason: a move byte is always
`(pieceIndex << 4) | code`, and the King **always** occupies piece index 0
(a constraint enforced by the implementation: "Kings must be piece Number
zero"). The King only uses codes 0 to 10 (§4.5.1). A full byte with a
value of 11 to 15 would therefore correspond to "King (index 0), code
11-15", a combination that is never produced by an actual King move — it
is therefore available and exclusively reserved for the control tokens
above.

#### 4.5.1 Detailed move encoding (`makeMoveByte` / `decodeMove` functions)

General formula for a move byte:

```
byte = ((pieceIndex & 0x0F) << 4) | (code & 0x0F)
```

**pieceIndex** (high 4 bits, 0-15) designates which piece of the side to
move is moving, by its index in the list of (up to 16) pieces of that side
(index 0 = always the King; indices 1-15 correspond to the 15 other
original pieces of the side, including the 8 pawns; a pawn that gets
promoted keeps its original index but changes type). This is **not** the
piece's type.

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
> fixed (1 or 7) either, beyond the very first capture. Behavior verified
> against the reference implementation (`position.cpp`,
> `Position::DoSimpleMove`, capture-handling section); not described
> elsewhere in this document.

**code** (low 4 bits, 0-15) has a meaning that depends on the type of the
piece that is moving (determined during decoding by looking up the
current board at the source square):

**KING** (pieceIndex always 0):

| Square difference (to − from) | Code |
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

Null move (to==from): the **entire byte equals 0x00** (pieceIndex=0,
code=0). Codes 11-15 are never used (reserved, see above).

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
| Same rank as source square | `code = file(to)` (destination file, 0-7, 0=a-file, 7=h-file) |
| Same file as source square | `code = 8 + rank(to)` (8-15, rank 0 = rank 1) |

**BISHOP:** `code = file(to)`, plus 8 if the diagonal taken is of the
"up-left/down-right" type (i.e. the product of the rank and file
differences is negative). Gives a code of 0-7 for an "up-right/down-left"
diagonal, 8-15 for the other diagonal.

**QUEEN** — the only piece that can occupy 2 bytes:

| Move type | Encoding |
|---|---|
| "Rook-like" move (same rank or same file) | Identical to the Rook encoding above (a single byte, code 0-15) |
| Diagonal move (2 bytes) | 1st byte = pieceIndex/file(from) (a sentinel, normally invalid as a Rook move); 2nd byte = (destination square) + 64, raw byte (64-127) |

**Decoding rule:** if code(1st byte) ≥ 8 → vertical move; otherwise if
code ≠ file(from) → horizontal move; otherwise (code==file(from)) → read
a 2nd byte, destination square = (value of the 2nd byte) − 64 (must be
between 64 and 127 inclusive, otherwise the game is considered corrupted).

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
> Queen move which occupies exactly 2 bytes. This is the only exception
> to the "one byte per move" rule.

### 4.6 (e) Comments text block

After the end-of-game token (`ENCODE_END_GAME`), a final block contains
the text of **all** the game's comments (main line and all sub-variations
combined), in the exact order in which the `ENCODE_COMMENT` tokens were
encountered while traversing section (d) above (depth-first traversal,
sub-variations processed immediately after the move that introduces them,
before continuing the main line).

Each comment is written as a character string **terminated by a null
byte** (0x00). There are exactly as many strings in this block as
`ENCODE_COMMENT` tokens emitted in section (d).

---

## 5. Derived / optional fields (search optimization)

The following fields, present in the `IndexEntry`, are **derived data**
computed from the game's actual content (fully recomputable from the
decoded `.sg4` blob). They speed up certain searches (by position, by
material, by opening) in the reference implementation, but their exact
value is **not** required for the format's structural validity: a minimal
writer may set them to zero, at the cost of degraded search performance in
software that relies on them (including Scid itself); a complete writer
should compute them correctly for full compatibility with Scid's
fast-search features.

### 5.1 `StoredLineCode` (high 8 bits of the `FinalMatSig` field, §2.3)

Code (0 = none) identifying the longest pre-recorded opening line
("StoredLine", an opening book internal to the reference implementation)
whose initial moves exactly match the beginning of the game. Purely
informational/ancillary.

### 5.2 `FinalMatSig` (low 24 bits of the `FinalMatSig` field, §2.3)

Material signature of the game's **final** position. `uint`, 32 bits (24
usable bits), bit layout (LSB at bit 0):

| Bits | Field | Cap |
|---|---|---|
| 0-3 | Black Pawn count | 0-8 (or more if abnormal, uncapped) |
| 4-5 | Black Knight count | capped at 3 |
| 6-7 | Black Bishop count | capped at 3 |
| 8-9 | Black Rook count | capped at 3 |
| 10-11 | Black Queen count | capped at 3 |
| 12-15 | White Pawn count | 0-8 |
| 16-17 | White Knight count | capped at 3 |
| 18-19 | White Bishop count | capped at 3 |
| 20-21 | White Rook count | capped at 3 |
| 22-23 | White Queen count | capped at 3 |

> **Accepted loss of information** — A count of 4 or more copies of a
> piece other than a pawn (e.g. after several promotions) is capped at
> the value 3 in this field — reserved for heuristic search use.

### 5.3 `HomePawnData` (9 bytes of the `IndexEntry`, §2.3)

History of pawns having left their home square. Byte 0 (6 usable bits,
shared with `NumHalfMoves`, §2.3): number of valid entries in the
following 8 bytes (up to 16 possible entries, 2 per byte, one nibble
each).

Each entry is a nibble encoding log2(delta) of a "HomePawnSig" — a 16-bit
bitmask representing the 16 pawns' home squares (a2-h2 and a7-h7) still
occupied by a pawn of their original side — at each half-move where this
bitmask changes (a pawn permanently leaves its home square, whether by
moving or being captured). Purely heuristic field used to speed up
position/opening searches; not required to correctly decode the game's
moves themselves.

If the game has a non-standard starting position, this field is left at
zero (byte 0 = 0, no entries).

---

## 6. Complete algorithm for creating an si4 database

Operational summary.

### 6.1 To WRITE an si4 database from a set of games (e.g. PGN import)

1. Create an empty `.sg4` file.
2. Create an empty `.si4` file (the final header, §2.2, will be
   (re)written on close, once `numGames` is known — but can also be kept
   up to date along the way).
3. For each game, in arrival order:
   1. encode the game's content (extra tags, flags, optional FEN,
      moves/variations/NAGs/comments) into a binary blob per §4;
   2. append this blob to the end of the `.sg4` file, note its Offset and
      Length;
   3. resolve (or create) the White/Black/Event/Site/Round identifiers in
      the 4 in-memory name tables (sorted and front-coded only when the
      `.sn4` is finally written, §3.3);
   4. build the `IndexEntry` (47 bytes, §2.3) with all fields;
   5. append this `IndexEntry` to the end of the `.si4` file (at position
      182 + 47×game_number); increment `numGames`.
4. Once all games have been processed: sort each name table
   alphabetically, write the complete `.sn4` file (36-byte header +
   front-coded names, §3).
5. (Re)write the `.si4` header (182 bytes, §2.2) with the final
   `numGames` and the other database metadata.

### 6.2 To READ an si4 database

1. Open the `.si4`, check the magic and version, read the header (182
   bytes).
2. Load the `.sn4` entirely into memory: read the header (36 bytes), then
   for each name type in the order PLAYER/EVENT/SITE/ROUND, read
   `numNames[type]` front-coded records and rebuild the full name list
   indexed by ID (§3.3).
3. To display/search game N (0-based): read the `IndexEntry` at position
   182+47×N in the `.si4` (§2.3), extract Offset and Length, resolve the
   5 name IDs via the `.sn4` table.
4. Read Length bytes at position Offset in the `.sg4`, and decode the
   blob per §4 (tags, flags, optional starting position,
   moves/variations/NAGs, then comments block) to fully reconstruct the
   game (moves, annotations, sub-variations, comments, complete PGN tags
   by combining the index's STR with the blob's extra tags).

---

## 7. Summary of analyzed source files

| File | Content |
|---|---|
| `src/common.h` | Basic types, pieces, squares, directions |
| `src/date.h` | `dateT` encoding |
| `src/mfile.h/.cpp` | Low-level I/O primitives (confirm big-endian) |
| `src/index.h/.cpp` | Complete `.si4` format (header + IndexEntry) |
| `src/namebase.h/.cpp` | Complete `.sn4` format |
| `src/gfile.h/.cpp` | Low-level `.sg4` container |
| `src/matsig.h` | Material signature encoding |
| `src/game.h/.cpp` | Complete encoding of a game's content (the core of the `.sg4` format: `Game::Encode` / `Game::Decode` and all per-piece-type move encoding/decoding functions) |
| `src/pgnscid.cpp` | Example program orchestrating the whole process (PGN → si4) |
| `src/bytebuf.h/.cpp` | Buffer read/write primitives used by `Game::Encode`/`Decode` (raw bytes, fixed-length strings, null-terminated strings) |

---

*See also: [`scid-si5-specification.md`](scid-si5-specification.md) — the
successor format, and [`ARCHITECTURE.md`](ARCHITECTURE.md) for how the
`scid` crate fits into the rest of Vendetta Chess GUI.*
