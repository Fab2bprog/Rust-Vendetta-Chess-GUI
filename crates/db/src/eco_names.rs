//! ECO code → opening name lookup table (PHASE 82, step 11 —
//! a point left open since the start of the phase, see `SUIVI_PLAN_ACTION.md`
//! point 7: "opening (ECO or name — will require an ECO → name lookup
//! table, not yet defined)").
//!
//! **Scope assumed for this step**: this table is used only to
//! *display* a readable opening name wherever a raw ECO code is already
//! displayed (game list, game detail) — it does NOT add a
//! search-by-name in the filters (the existing ECO filter, by prefix,
//! remains unchanged). The step's title ("ECO → opening name table") only
//! mentioned the table itself; a search by name would remain a
//! separate future extension, more invasive (modifying `GameFilter`
//! and the SQL query), not necessary to close out the 11-step plan.
//!
//! **Granularity assumed**: the full ECO classification has 500 codes
//! (A00-E99), many of which only designate very fine sub-variations
//! with no common usage name. Rather than transcribing all 500 entries (risk
//! of typing errors not detectable without a compiler, and with little
//! added value for obscure sub-variations), this table covers the
//! common families/variations by **code ranges**, at the granularity
//! actually useful to a player (e.g. "B90-B99 → Sicilian, Najdorf
//! Variation"). The lookup always keeps the most **specific**
//! (narrowest) range that contains the requested code — a broad range
//! (e.g. "B20-B99 → Sicilian Defense") serves as a fallback when no more
//! precise range covers the code. Five fallback ranges covering each entire
//! letter group (A00-A99, B00-B99, C00-C99, D00-D99, E00-E99) guarantee
//! that no valid ECO code (a letter A-E followed by two digits) is left
//! without a result.
//!
//! Data source: standard ECO classification (FIDE), as
//! summarized notably by <https://en.wikipedia.org/wiki/List_of_ECO_codes>,
//! <https://www.chessprogramming.org/ECO>, and the detailed table at
//! <http://www.eudesign.com/chessops/eco/> (names translated into French for
//! this table). Like the player/tournament names already present in the
//! imported PGN, these opening names are **not** translated into the 40
//! interface languages (`i18n`): they are data belonging to the game itself,
//! treated as such (same logic as `ReferenceGameRow.white`/`.event`,
//! never passed through `Tr`/`i18n::translate`), not interface chrome
//! elements.

/// `(lower bound inclusive, upper bound inclusive, opening name)`. The two
/// bounds always share the same initial letter (direct lexicographic
/// comparison over the 3 characters, valid since the width is fixed).
/// The order of entries does not matter: [`opening_name`] keeps the
/// narrowest enclosing range, regardless of declaration order.
#[rustfmt::skip]
static ECO_RANGES: &[(&str, &str, &str)] = &[
    // ── Full fallback (guarantees complete coverage from A00 to E99) ──────
    ("A00", "A99", "Ouverture de flanc / pion Dame (non classée précisément)"),
    ("B00", "B99", "Ouverture du pion Roi (non classée précisément)"),
    ("C00", "C99", "Ouverture du pion Roi, 1...e5/1...e6 (non classée précisément)"),
    ("D00", "D99", "Jeu de pion Dame (non classé précisément)"),
    ("E00", "E99", "Défense indienne (non classée précisément)"),

    // ── Group A: flank openings, Queen's pawn, Indian defenses ────────────
    ("A00", "A00", "Ouverture irrégulière (Benko, Dunst, polonaise...)"),
    ("A01", "A01", "Ouverture Larsen"),
    ("A02", "A03", "Ouverture Bird"),
    ("A04", "A09", "Ouverture Réti"),
    ("A07", "A08", "Système Barcza"),
    ("A10", "A39", "Ouverture anglaise"),
    ("A11", "A12", "Anglaise, système Caro-Kann"),
    ("A13", "A14", "Anglaise, défense franco-anglaise"),
    ("A15", "A19", "Anglaise, défense anglo-indienne"),
    ("A20", "A29", "Anglaise, variante sicilienne inversée"),
    ("A30", "A39", "Anglaise, variante symétrique"),
    ("A40", "A44", "Pion Dame, défenses irrégulières"),
    ("A41", "A42", "Défense est-indienne ancienne (Old Indian)"),
    ("A43", "A44", "Vieux Benoni"),
    ("A45", "A79", "Défense indienne (générique)"),
    ("A45", "A46", "Attaque Trompowsky / Torre"),
    ("A47", "A49", "Défense ouest-indienne / est-indienne fianchettée"),
    ("A50", "A59", "Gambit Benko (Volga) et apparentés"),
    ("A51", "A52", "Gambit Budapest"),
    ("A56", "A59", "Benoni tchèque / Gambit Benko"),
    ("A60", "A79", "Défense Benoni moderne"),
    ("A80", "A99", "Défense hollandaise"),
    ("A82", "A83", "Gambit Staunton"),
    ("A87", "A89", "Système de Leningrad"),
    ("A90", "A99", "Système Stonewall"),

    // ── Group B: King's pawn openings (other than 1...e5/1...e6) ──────────
    ("B00", "B00", "Défenses irrégulières du pion Roi (Nimzowitsch, Owen)"),
    ("B01", "B01", "Défense scandinave (Centre-Contre)"),
    ("B02", "B05", "Défense Alekhine"),
    ("B06", "B06", "Défense moderne (Robatsch)"),
    ("B07", "B09", "Défense Pirc"),
    ("B10", "B19", "Défense Caro-Kann"),
    ("B13", "B14", "Caro-Kann, variante d'échange"),
    ("B18", "B19", "Caro-Kann, variante classique"),
    ("B20", "B99", "Défense sicilienne"),
    ("B21", "B21", "Sicilienne, gambits (dont attaque Grand Prix)"),
    ("B22", "B22", "Sicilienne, variante Alapine (2.c3)"),
    ("B23", "B26", "Sicilienne, variante fermée"),
    ("B27", "B29", "Sicilienne, divers (O'Kelly, Rubinstein)"),
    ("B30", "B39", "Sicilienne, variante orthodoxe / Rossolimo"),
    ("B33", "B33", "Sicilienne, variante Sveshnikov/Pelikan"),
    ("B34", "B39", "Sicilienne, Dragon accéléré"),
    ("B40", "B49", "Sicilienne, variante Paulsen/Kan/Taimanov"),
    ("B50", "B59", "Sicilienne, divers après 2...d6"),
    ("B51", "B52", "Sicilienne, attaque Rossolimo/Moscou"),
    ("B60", "B69", "Sicilienne, attaque Richter-Rauzer"),
    ("B70", "B79", "Sicilienne, variante Dragon"),
    ("B75", "B79", "Sicilienne, Dragon Yougoslave (attaque Rauzer-Sozin)"),
    ("B80", "B89", "Sicilienne, variante Scheveningen"),
    ("B90", "B99", "Sicilienne, variante Najdorf"),
    ("B97", "B97", "Sicilienne Najdorf, variante du pion empoisonné"),

    // ── Group C: King's pawn openings, 1...e5/1...e6 ───────────────────────
    ("C00", "C19", "Défense française"),
    ("C01", "C01", "Française, variante d'échange"),
    ("C02", "C02", "Française, variante d'avance"),
    ("C03", "C09", "Française, variante Tarrasch"),
    ("C10", "C19", "Française, variante Paulsen/classique"),
    ("C11", "C11", "Française, variante Steinitz"),
    ("C12", "C12", "Française, variante McCutcheon"),
    ("C13", "C13", "Française, variante Burn"),
    ("C15", "C19", "Française, défense Winawer"),
    ("C20", "C29", "Ouvertures ouvertes du pion Roi (partie du centre, Vienne)"),
    ("C21", "C21", "Gambit danois"),
    ("C23", "C24", "Ouverture du fou (Bishop's Opening)"),
    ("C25", "C29", "Partie viennoise"),
    ("C30", "C39", "Gambit du roi"),
    ("C33", "C39", "Gambit du roi accepté"),
    ("C40", "C40", "Contre-gambit letton"),
    ("C41", "C41", "Défense Philidor"),
    ("C42", "C43", "Défense russe (Petrov)"),
    ("C44", "C44", "Partie écossaise, gambit"),
    ("C45", "C45", "Partie écossaise, variante classique"),
    ("C46", "C46", "Partie des trois cavaliers"),
    ("C47", "C49", "Partie des quatre cavaliers"),
    ("C50", "C54", "Giuoco Piano (partie italienne)"),
    ("C51", "C52", "Gambit Evans"),
    ("C55", "C59", "Défense des deux cavaliers"),
    ("C60", "C99", "Ouverture espagnole (Ruy Lopez)"),
    ("C61", "C61", "Espagnole, défense Bird"),
    ("C62", "C62", "Espagnole, défense Steinitz"),
    ("C63", "C63", "Espagnole, défense Schliemann (gambit Jaenisch)"),
    ("C65", "C67", "Espagnole, défense berlinoise"),
    ("C68", "C69", "Espagnole, variante d'échange"),
    ("C70", "C76", "Espagnole, défense Steinitz différée"),
    ("C77", "C99", "Espagnole, défense Morphy"),
    ("C80", "C83", "Espagnole, variante ouverte"),
    ("C84", "C99", "Espagnole, variante fermée"),
    ("C88", "C89", "Espagnole, contre-attaque Marshall"),
    ("C92", "C99", "Espagnole, variantes Zaïtsev/Chigorine/Breyer"),

    // ── Group D: Queen's pawn game, Queen's Gambit, Grünfeld ───────────────
    ("D00", "D05", "Jeu du pion Dame"),
    ("D01", "D01", "Attaque Richter-Veresov"),
    ("D04", "D05", "Système Colle"),
    ("D06", "D06", "Gambit Dame, défenses irrégulières"),
    ("D07", "D07", "Défense Chigorine"),
    ("D08", "D09", "Contre-gambit Albin"),
    ("D10", "D19", "Défense slave"),
    ("D15", "D15", "Slave, gambits divers"),
    ("D17", "D17", "Slave, système tchèque (Slave ouverte)"),
    ("D20", "D29", "Gambit Dame accepté"),
    ("D30", "D69", "Gambit Dame refusé"),
    ("D31", "D31", "Gambit Dame refusé, défense semi-slave"),
    ("D32", "D34", "Gambit Dame, système Tarrasch"),
    ("D35", "D36", "Gambit Dame, variante d'échange"),
    ("D38", "D39", "Variante Ragozine"),
    ("D40", "D42", "Défense semi-Tarrasch"),
    ("D43", "D49", "Défense semi-slave"),
    ("D46", "D49", "Semi-slave, variante Meran"),
    ("D50", "D69", "Gambit Dame refusé, développement Pillsbury"),
    ("D52", "D52", "Défense Cambridge Springs"),
    ("D58", "D59", "Système Tartakover"),
    ("D60", "D69", "Défense orthodoxe"),
    ("D70", "D79", "Défense néo-Grünfeld"),
    ("D80", "D99", "Défense Grünfeld"),
    ("D85", "D89", "Grünfeld, variante d'échange"),
    ("D90", "D99", "Grünfeld, variante des trois cavaliers"),

    // ── Group E: Indian defenses (Catalan, Nimzo, King's Indian) ──────────
    ("E00", "E09", "Ouverture catalane"),
    ("E10", "E19", "Système anti-nimzo-indien"),
    ("E11", "E11", "Défense bogo-indienne"),
    ("E12", "E19", "Défense est-indienne de la Dame (ouest-indienne)"),
    ("E20", "E59", "Défense nimzo-indienne"),
    ("E24", "E29", "Nimzo-indienne, variante Sämisch"),
    ("E32", "E39", "Nimzo-indienne, variante classique"),
    ("E40", "E59", "Nimzo-indienne, système Rubinstein"),
    ("E60", "E99", "Défense est-indienne du roi"),
    ("E62", "E69", "Est-indienne, fianchetto différé"),
    ("E70", "E79", "Est-indienne, système classique / Averbakh"),
    ("E76", "E79", "Est-indienne, attaque des quatre pions"),
    ("E80", "E89", "Est-indienne, variante Sämisch"),
    ("E90", "E99", "Est-indienne, variante classique"),
];

/// Returns the opening name corresponding to the given ECO code (e.g. `"B90"`),
/// or `None` if `eco` does not have the expected shape (a letter `A`-`E` followed
/// by two digits, leading/trailing whitespace ignored).
///
/// Among all the [`ECO_RANGES`] ranges containing `eco`, returns the name
/// of the most **specific** range (the smallest gap between upper bound
/// and lower bound) — see the module documentation.
#[must_use]
pub fn opening_name(eco: &str) -> Option<&'static str> {
    let eco = eco.trim();
    if eco.len() != 3 {
        return None;
    }
    let bytes = eco.as_bytes();
    if !bytes[0].is_ascii_uppercase() || !(b'A'..=b'E').contains(&bytes[0]) {
        return None;
    }
    if !bytes[1].is_ascii_digit() || !bytes[2].is_ascii_digit() {
        return None;
    }

    let mut best: Option<(&'static str, i32)> = None;
    for &(lo, hi, name) in ECO_RANGES {
        if eco >= lo && eco <= hi {
            let span = suffix_num(hi) - suffix_num(lo);
            let keep = match best {
                None => true,
                Some((_, best_span)) => span < best_span,
            };
            if keep {
                best = Some((name, span));
            }
        }
    }
    best.map(|(name, _)| name)
}

/// Extracts the last two characters of an ECO code (`"B90"` → `90`) as an
/// integer, to compare the width of two ranges sharing the same
/// initial letter.
fn suffix_num(code: &str) -> i32 {
    code[1..3].parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::opening_name;

    #[test]
    fn test_specific_codes() {
        assert_eq!(opening_name("B90"), Some("Sicilienne, variante Najdorf"));
        assert_eq!(opening_name("C60"), Some("Ouverture espagnole (Ruy Lopez)"));
        assert_eq!(opening_name("E60"), Some("Défense est-indienne du roi"));
    }

    #[test]
    fn test_narrowest_range_wins_over_broader_one() {
        // B97 must return the precise variation (poisoned pawn), not
        // just the generic Najdorf name (B90-B99).
        assert_eq!(
            opening_name("B97"),
            Some("Sicilienne Najdorf, variante du pion empoisonné")
        );
    }

    #[test]
    fn test_trims_whitespace() {
        assert_eq!(opening_name(" B90 "), Some("Sicilienne, variante Najdorf"));
    }

    #[test]
    fn test_invalid_or_missing_code_returns_none() {
        assert_eq!(opening_name(""), None);
        assert_eq!(opening_name("Z99"), None);
        assert_eq!(opening_name("A1"), None);
        assert_eq!(opening_name("A1X"), None);
    }

    #[test]
    fn test_full_coverage_a_to_e_never_none() {
        // The 5 fallback ranges (A00-A99 .. E00-E99) guarantee that no
        // valid ECO code is left without a result.
        for letter in ['A', 'B', 'C', 'D', 'E'] {
            for n in 0..100 {
                let code = format!("{letter}{n:02}");
                assert!(opening_name(&code).is_some(), "code {code} non couvert");
            }
        }
    }
}
