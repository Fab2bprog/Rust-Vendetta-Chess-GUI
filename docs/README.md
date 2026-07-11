# Developer Documentation

This folder holds documentation for **contributors and maintainers** —
architecture notes, design decisions, format specifications, and anything
else too detailed for the main [README](../README.md), which is aimed at
end users.

If you're looking for how to *use* the application, see the
[README](../README.md) instead. If you're looking for how to *contribute*
code, see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Pages

- [**ARCHITECTURE.md**](ARCHITECTURE.md) — how the workspace crates fit
  together (View/Controller/Model layering, dependency graph), with
  diagrams walking through a few representative flows (playing a move,
  running an engine analysis, importing a SCID database).
- [**scid-si4-specification.md**](scid-si4-specification.md) — full
  reverse-engineered binary specification of the SCID `.si4`/`.sn4`/`.sg4`
  format (format version 4.0), reproduced by the `scid` crate's si4 reader.
- [**scid-si5-specification.md**](scid-si5-specification.md) — full
  reverse-engineered binary specification of the SCID `.si5`/`.sn5`/`.sg5`
  format (the successor of si4), reproduced by the `scid` crate's si5
  reader.

## Suggested contents

This folder is intentionally being filled in incrementally. Other pages
that are likely useful here as the project grows:

- **Internationalization** — how `crates/i18n` works and how to add a new
  language.
- **Decision log** — notable design decisions and the reasoning behind
  them, for context on "why is it built this way."

Feel free to add, rename, or restructure pages here — this index is just a
starting point, not a fixed template.
