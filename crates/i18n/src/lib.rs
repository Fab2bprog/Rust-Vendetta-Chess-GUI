//! Internationalization (i18n) system for `vendetta_chess_gui`.
//!
//! All user-visible strings go through this module. Translations are
//! embedded in the binary via `include_str!()` and loaded once at startup.
//! Changing the language does not require a restart.
//!
//! # Quick usage
//!
//! ```
//! use i18n::{init, set_lang, Lang};
//!
//! // Explicitly initialize in French (the interface's default language
//! // is English, see `Lang::default()` / `#[default]`)
//! init(Lang::Fr);
//!
//! // Translate a key
//! let label = i18n::translate("menu.file.new_game");
//! assert_eq!(label, "Nouvelle partie");
//!
//! // Change language on the fly
//! set_lang(Lang::En);
//! let label = i18n::translate("menu.file.new_game");
//! assert_eq!(label, "New Game");
//! ```

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

// ---------------------------------------------------------------------------
// Supported languages
// ---------------------------------------------------------------------------

/// Languages available in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    /// Français (reference language for translations — see `fr.toml`,
    /// source of truth for the 315 keys (PHASE 68: +1, Assistance
    /// tooltip) — but
    /// is no longer the interface's default language, see `En` below)
    Fr,
    /// English (interface default language: used if no preference has
    /// been saved, or after a full settings reset — changed on
    /// 05/07/2026, explicit user request)
    #[default]
    En,
    /// Italiano
    It,
    /// Español
    Es,
    /// Português
    Pt,
    /// Deutsch
    De,
    /// Svenska
    Sv,
    /// Norsk
    No,
    /// Suomi
    Fi,
    /// Íslenska
    Is,
    /// Polski
    Pl,
    /// Nederlands
    Nl,
    /// Русский
    Ru,
    /// Українська
    Uk,
    /// Română
    Ro,
    /// Ελληνικά
    El,
    /// Magyar
    Hu,
    /// Čeština
    Cs,
    /// Dansk
    Da,
    /// Български
    Bg,
    /// Slovenčina
    Sk,
    /// Hrvatski
    Hr,
    /// Српски
    Sr,
    /// Lietuvių
    Lt,
    /// Slovenščina
    Sl,
    /// Latviešu
    Lv,
    /// Eesti
    Et,
    /// Shqip
    Sq,
    /// Македонски
    Mk,
    /// Bosanski
    Bs,
    /// Malti
    Mt,
    /// Беларуская
    Be,
    /// Lëtzebuergesch
    Lb,
    /// עברית (Hebrew — written right-to-left; the Slint UI stays in LTR
    /// layout, only the text of translated strings is in Hebrew)
    He,
    /// العربية (Arabic — written right-to-left, same note as `He` above)
    Ar,
    /// 中文 (Simplified Chinese)
    Zh,
    /// 日本語 (Japanese)
    Ja,
    /// فارسی (Persian/Farsi — written right-to-left, same note as
    /// `He`/`Ar` above; the language of origin of chess vocabulary:
    /// "checkmate" comes from Persian "شاه مات", shâh mât, "the king is
    /// dead")
    Fa,
    /// हिन्दी (Hindi)
    Hi,
    /// 한국어 (Korean)
    Ko,
}

impl Lang {
    /// All supported languages, in menu order.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Fr, Self::En, Self::It, Self::Es, Self::Pt,
            Self::De, Self::Sv, Self::No, Self::Fi, Self::Is, Self::Pl,
            Self::Nl, Self::Ru, Self::Uk, Self::Ro, Self::El,
            Self::Hu, Self::Cs, Self::Da, Self::Bg, Self::Sk,
            Self::Hr, Self::Sr, Self::Lt, Self::Sl, Self::Lv,
            Self::Et, Self::Sq, Self::Mk, Self::Bs, Self::Mt,
            Self::Be, Self::Lb, Self::He,
            Self::Ar, Self::Zh, Self::Ja,
            Self::Fa, Self::Hi, Self::Ko,
        ]
    }

    /// ISO 639-1 code of the language.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Fr => "fr", Self::En => "en", Self::It => "it",
            Self::Es => "es", Self::Pt => "pt", Self::De => "de",
            Self::Sv => "sv", Self::No => "no", Self::Fi => "fi",
            Self::Is => "is", Self::Pl => "pl",
            Self::Nl => "nl", Self::Ru => "ru", Self::Uk => "uk",
            Self::Ro => "ro", Self::El => "el", Self::Hu => "hu",
            Self::Cs => "cs", Self::Da => "da", Self::Bg => "bg",
            Self::Sk => "sk", Self::Hr => "hr", Self::Sr => "sr",
            Self::Lt => "lt", Self::Sl => "sl", Self::Lv => "lv",
            Self::Et => "et", Self::Sq => "sq", Self::Mk => "mk",
            Self::Bs => "bs", Self::Mt => "mt", Self::Be => "be",
            Self::Lb => "lb", Self::He => "he",
            Self::Ar => "ar", Self::Zh => "zh", Self::Ja => "ja",
            Self::Fa => "fa", Self::Hi => "hi", Self::Ko => "ko",
        }
    }

    /// Index of the language in [`Lang::all`] order (0 = Français, …,
    /// 39 = 한국어). Used to drive the Slint side of the language dropdown
    /// display in Preferences (bugfix from 03/07/2026): Slint directly
    /// indexes its own flag/name array with this value instead of
    /// looking up a code→label mapping (an untried pattern in this
    /// project for an array of structs).
    #[must_use]
    pub fn ui_index(self) -> i32 {
        match self {
            Self::Fr => 0, Self::En => 1, Self::It => 2,
            Self::Es => 3, Self::Pt => 4, Self::De => 5,
            Self::Sv => 6, Self::No => 7, Self::Fi => 8,
            Self::Is => 9, Self::Pl => 10,
            Self::Nl => 11, Self::Ru => 12, Self::Uk => 13,
            Self::Ro => 14, Self::El => 15, Self::Hu => 16,
            Self::Cs => 17, Self::Da => 18, Self::Bg => 19,
            Self::Sk => 20, Self::Hr => 21, Self::Sr => 22,
            Self::Lt => 23, Self::Sl => 24, Self::Lv => 25,
            Self::Et => 26, Self::Sq => 27, Self::Mk => 28,
            Self::Bs => 29, Self::Mt => 30, Self::Be => 31,
            Self::Lb => 32, Self::He => 33,
            Self::Ar => 34, Self::Zh => 35, Self::Ja => 36,
            Self::Fa => 37, Self::Hi => 38, Self::Ko => 39,
        }
    }

    /// Native name of the language (for display in the Language menu).
    #[must_use]
    pub fn native_name(self) -> &'static str {
        match self {
            Self::Fr => "Français",
            Self::En => "English",
            Self::It => "Italiano",
            Self::Es => "Español",
            Self::Pt => "Português",
            Self::De => "Deutsch",
            Self::Sv => "Svenska",
            Self::No => "Norsk",
            Self::Fi => "Suomi",
            Self::Is => "Íslenska",
            Self::Pl => "Polski",
            Self::Nl => "Nederlands",
            Self::Ru => "Русский",
            Self::Uk => "Українська",
            Self::Ro => "Română",
            Self::El => "Ελληνικά",
            Self::Hu => "Magyar",
            Self::Cs => "Čeština",
            Self::Da => "Dansk",
            Self::Bg => "Български",
            Self::Sk => "Slovenčina",
            Self::Hr => "Hrvatski",
            Self::Sr => "Српски",
            Self::Lt => "Lietuvių",
            Self::Sl => "Slovenščina",
            Self::Lv => "Latviešu",
            Self::Et => "Eesti",
            Self::Sq => "Shqip",
            Self::Mk => "Македонски",
            Self::Bs => "Bosanski",
            Self::Mt => "Malti",
            Self::Be => "Беларуская",
            Self::Lb => "Lëtzebuergesch",
            Self::He => "עברית",
            Self::Ar => "العربية",
            Self::Zh => "中文",
            Self::Ja => "日本語",
            Self::Fa => "فارسی",
            Self::Hi => "हिन्दी",
            Self::Ko => "한국어",
        }
    }

    /// Resolves an ISO 639-1 code into a [`Lang`].
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "fr" => Some(Self::Fr), "en" => Some(Self::En),
            "it" => Some(Self::It), "es" => Some(Self::Es),
            "pt" => Some(Self::Pt), "de" => Some(Self::De),
            "sv" => Some(Self::Sv), "no" => Some(Self::No),
            "fi" => Some(Self::Fi), "is" => Some(Self::Is),
            "pl" => Some(Self::Pl),
            "nl" => Some(Self::Nl), "ru" => Some(Self::Ru),
            "uk" => Some(Self::Uk), "ro" => Some(Self::Ro),
            "el" => Some(Self::El), "hu" => Some(Self::Hu),
            "cs" => Some(Self::Cs), "da" => Some(Self::Da),
            "bg" => Some(Self::Bg), "sk" => Some(Self::Sk),
            "hr" => Some(Self::Hr), "sr" => Some(Self::Sr),
            "lt" => Some(Self::Lt), "sl" => Some(Self::Sl),
            "lv" => Some(Self::Lv), "et" => Some(Self::Et),
            "sq" => Some(Self::Sq), "mk" => Some(Self::Mk),
            "bs" => Some(Self::Bs), "mt" => Some(Self::Mt),
            "be" => Some(Self::Be), "lb" => Some(Self::Lb),
            "he" => Some(Self::He),
            "ar" => Some(Self::Ar), "zh" => Some(Self::Zh), "ja" => Some(Self::Ja),
            "fa" => Some(Self::Fa), "hi" => Some(Self::Hi), "ko" => Some(Self::Ko),
            _    => None,
        }
    }
}

// ---------------------------------------------------------------------------
// TOML files embedded at compile time
// ---------------------------------------------------------------------------

/// (ISO code, TOML content) pairs — loaded via `include_str!()`.
const LOCALE_FILES: &[(&str, &str)] = &[
    ("fr", include_str!("../locales/fr.toml")),
    ("en", include_str!("../locales/en.toml")),
    ("it", include_str!("../locales/it.toml")),
    ("es", include_str!("../locales/es.toml")),
    ("pt", include_str!("../locales/pt.toml")),
    ("de", include_str!("../locales/de.toml")),
    ("sv", include_str!("../locales/sv.toml")),
    ("no", include_str!("../locales/no.toml")),
    ("fi", include_str!("../locales/fi.toml")),
    ("is", include_str!("../locales/is.toml")),
    ("pl", include_str!("../locales/pl.toml")),
    ("nl", include_str!("../locales/nl.toml")),
    ("ru", include_str!("../locales/ru.toml")),
    ("uk", include_str!("../locales/uk.toml")),
    ("ro", include_str!("../locales/ro.toml")),
    ("el", include_str!("../locales/el.toml")),
    ("hu", include_str!("../locales/hu.toml")),
    ("cs", include_str!("../locales/cs.toml")),
    ("da", include_str!("../locales/da.toml")),
    ("bg", include_str!("../locales/bg.toml")),
    ("sk", include_str!("../locales/sk.toml")),
    ("hr", include_str!("../locales/hr.toml")),
    ("sr", include_str!("../locales/sr.toml")),
    ("lt", include_str!("../locales/lt.toml")),
    ("sl", include_str!("../locales/sl.toml")),
    ("lv", include_str!("../locales/lv.toml")),
    ("et", include_str!("../locales/et.toml")),
    ("sq", include_str!("../locales/sq.toml")),
    ("mk", include_str!("../locales/mk.toml")),
    ("bs", include_str!("../locales/bs.toml")),
    ("mt", include_str!("../locales/mt.toml")),
    ("be", include_str!("../locales/be.toml")),
    ("lb", include_str!("../locales/lb.toml")),
    ("he", include_str!("../locales/he.toml")),
    ("ar", include_str!("../locales/ar.toml")),
    ("zh", include_str!("../locales/zh.toml")),
    ("ja", include_str!("../locales/ja.toml")),
    ("fa", include_str!("../locales/fa.toml")),
    ("hi", include_str!("../locales/hi.toml")),
    ("ko", include_str!("../locales/ko.toml")),
];

// ---------------------------------------------------------------------------
// Thread-safe global state
// ---------------------------------------------------------------------------

/// All translations — initialized once via `OnceLock`.
static TRANSLATIONS: OnceLock<HashMap<Lang, HashMap<String, String>>> = OnceLock::new();

/// Current language — changeable on the fly via `set_lang`.
///
/// Initial value `Lang::En` (literal, not `Lang::default()`: a `static`
/// initializer must be evaluable in const context, and a derived
/// `Default::default()` function call is not) — consistent with
/// `#[default]` on `Lang::En` above.
static CURRENT_LANG: RwLock<Lang> = RwLock::new(Lang::En);

// ---------------------------------------------------------------------------
// Internal loading
// ---------------------------------------------------------------------------

/// Parses all embedded TOML files into a `HashMap` indexed by [`Lang`].
///
/// Called exactly once by `OnceLock::get_or_init`.
///
/// Robustness audit 11/07/2026, finding 3.8: a language whose entry is
/// malformed (unknown code, or TOML that fails to parse) is now logged
/// and **skipped** instead of panicking the whole process. Before this
/// fix, a single bad `.toml` blocked the startup of the entire
/// application for every user, even a francophone one whose own
/// `fr.toml` was perfectly fine, just because e.g. `ko.toml` had a
/// syntax error — disproportionate given all 40 files are embedded at
/// compile time via `include_str!` (not read from a user-writable path,
/// so this can currently only happen from a bug that slipped past the
/// project's own `test_all_locales_load_without_panic` CI check, not
/// from any runtime corruption). [`translate`]/[`translate_in`] already
/// degrade gracefully for a language missing from the map (falls back to
/// returning the raw key, see their doc) — skipping an entry here is
/// therefore safe and requires no change on the lookup side.
fn load_all() -> HashMap<Lang, HashMap<String, String>> {
    let mut map = HashMap::with_capacity(LOCALE_FILES.len());
    for (code, content) in LOCALE_FILES {
        let Some(lang) = Lang::from_code(code) else {
            eprintln!("i18n: code de langue inconnu '{code}' — langue ignorée");
            continue;
        };
        let table: HashMap<String, String> = match toml::from_str(content) {
            Ok(table) => table,
            Err(e) => {
                eprintln!("i18n: erreur TOML dans '{code}.toml' — langue ignorée : {e}");
                continue;
            }
        };
        map.insert(lang, table);
    }
    map
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initializes the i18n system and sets the startup language.
///
/// Must be called **once** at launch, before any call to [`translate`] or
/// [`t!`]. Subsequent calls only change the language (loading translations
/// is idempotent).
pub fn init(lang: Lang) {
    TRANSLATIONS.get_or_init(load_all);
    set_lang(lang);
}

/// Changes the current language without a restart.
///
/// All subsequent calls to [`translate`] will use this language.
pub fn set_lang(lang: Lang) {
    if let Ok(mut guard) = CURRENT_LANG.write() {
        *guard = lang;
    }
}

/// Returns the current language.
#[must_use]
pub fn current_lang() -> Lang {
    CURRENT_LANG.read().map_or_else(|_| Lang::default(), |g| *g)
}

/// Translates `key` in the current language.
///
/// If the key is not found (missing translation), returns `key` unchanged —
/// never panics, never an empty string.
#[must_use]
pub fn translate(key: &str) -> String {
    let lang = current_lang();
    TRANSLATIONS
        .get()
        .and_then(|map| map.get(&lang))
        .and_then(|table| table.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

/// Translates `key` in an explicit language, independent of the current
/// language.
///
/// Useful for displaying several languages at once (language selection
/// menu).
#[must_use]
pub fn translate_in(lang: Lang, key: &str) -> String {
    // Ensures translations are loaded even without a call to `init`.
    TRANSLATIONS.get_or_init(load_all);
    TRANSLATIONS
        .get()
        .and_then(|map| map.get(&lang))
        .and_then(|table| table.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

// ---------------------------------------------------------------------------
// Macro
// ---------------------------------------------------------------------------

/// Translates an i18n key in the current language.
///
/// Shortcut for [`translate`]. Returns a [`String`].
///
/// # Example
///
/// ```
/// use i18n::{init, Lang, t};
///
/// init(Lang::Fr);
/// assert_eq!(t!("app.name"), "Vendetta Chess");
/// assert_eq!(t!("board.white"), "Blancs");
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::translate($key)
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Initializes translations without touching the global current
    // language. Uses `translate_in` to avoid conflicts between parallel
    // tests.
    fn ensure_loaded() {
        TRANSLATIONS.get_or_init(load_all);
    }

    // -----------------------------------------------------------------------
    // Lang
    // -----------------------------------------------------------------------

    #[test]
    fn test_lang_all_returns_40() {
        assert_eq!(Lang::all().len(), 40);
    }

    #[test]
    fn test_lang_default_is_english() {
        // Changed on 05/07/2026 (explicit user request): the interface's
        // default language is English, not French anymore.
        assert_eq!(Lang::default(), Lang::En);
    }

    #[test]
    fn test_lang_from_code_valid() {
        assert_eq!(Lang::from_code("fr"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("en"), Some(Lang::En));
        assert_eq!(Lang::from_code("pl"), Some(Lang::Pl));
        assert_eq!(Lang::from_code("is"), Some(Lang::Is));
    }

    #[test]
    fn test_lang_from_code_invalid() {
        assert_eq!(Lang::from_code("xx"), None);
        assert_eq!(Lang::from_code(""),   None);
        assert_eq!(Lang::from_code("FR"), None); // case-sensitive
    }

    #[test]
    fn test_lang_code_roundtrip() {
        for &lang in Lang::all() {
            let code = lang.code();
            assert_eq!(Lang::from_code(code), Some(lang), "roundtrip échoue pour {code}");
        }
    }

    #[test]
    fn test_lang_native_name_nonempty() {
        for &lang in Lang::all() {
            assert!(!lang.native_name().is_empty(), "native_name vide pour {lang:?}");
        }
    }

    #[test]
    fn test_lang_ui_index_matches_all_order() {
        for (i, &lang) in Lang::all().iter().enumerate() {
            assert_eq!(
                lang.ui_index(),
                i32::try_from(i).unwrap(),
                "ui_index incohérent avec Lang::all() pour {lang:?}"
            );
        }
    }

    #[test]
    fn test_lang_ui_index_distinct() {
        let mut indices: Vec<i32> = Lang::all().iter().map(|l| l.ui_index()).collect();
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(indices.len(), 40, "ui_index doit être unique par langue");
    }

    // -----------------------------------------------------------------------
    // Loading translations
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_locales_load_without_panic() {
        // Checks that all 40 TOML files are valid.
        ensure_loaded();
        let map = TRANSLATIONS.get().expect("traductions non initialisées");
        assert_eq!(map.len(), 40, "40 langues attendues");
    }

    #[test]
    fn test_all_locales_have_same_key_count() {
        ensure_loaded();
        let map = TRANSLATIONS.get().unwrap();
        let counts: Vec<usize> = Lang::all().iter().map(|l| map[l].len()).collect();
        let first = counts[0];
        for (i, &count) in counts.iter().enumerate() {
            assert_eq!(
                count, first,
                "langue {:?} a {} clés, attendu {}",
                Lang::all()[i], count, first
            );
        }
    }

    // -----------------------------------------------------------------------
    // translate_in (independent of the current language)
    // -----------------------------------------------------------------------

    #[test]
    fn test_translate_in_fr_app_name() {
        assert_eq!(translate_in(Lang::Fr, "app.name"), "Vendetta Chess");
    }

    #[test]
    fn test_translate_in_en_new_game() {
        assert_eq!(translate_in(Lang::En, "menu.file.new_game"), "New Game");
    }

    #[test]
    fn test_translate_in_de_checkmate() {
        assert_eq!(translate_in(Lang::De, "board.checkmate"), "Schachmatt");
    }

    #[test]
    fn test_translate_in_pl_depth() {
        assert_eq!(translate_in(Lang::Pl, "analysis.depth"), "Głębokość");
    }

    #[test]
    fn test_translate_in_fi_cancel() {
        assert_eq!(translate_in(Lang::Fi, "btn.cancel"), "Peruuta");
    }

    #[test]
    fn test_translate_in_sv_draw() {
        assert_eq!(translate_in(Lang::Sv, "board.draw"), "Remi");
    }

    #[test]
    fn test_translate_in_is_ok() {
        assert_eq!(translate_in(Lang::Is, "btn.ok"), "Í lagi");
    }

    #[test]
    fn test_translate_in_no_check() {
        assert_eq!(translate_in(Lang::No, "board.check"), "Sjakk!");
    }

    #[test]
    fn test_translate_in_nl_new_game() {
        assert_eq!(translate_in(Lang::Nl, "menu.file.new_game"), "Nieuw spel");
    }

    #[test]
    fn test_translate_in_ru_checkmate() {
        assert_eq!(translate_in(Lang::Ru, "board.checkmate"), "Мат");
    }

    #[test]
    fn test_translate_in_uk_cancel() {
        assert_eq!(translate_in(Lang::Uk, "btn.cancel"), "Скасувати");
    }

    #[test]
    fn test_translate_in_ro_draw() {
        // "Remiză" (cognate of French "remise") is the standard Romanian
        // chess term for a draw — fixed on 05/07/2026: the test still
        // expected "Egalitate", a value never aligned with `ro.toml`
        // (which does contain "Remiză" for `board.draw` and
        // `game.result.draw`).
        assert_eq!(translate_in(Lang::Ro, "board.draw"), "Remiză");
    }

    #[test]
    fn test_translate_in_el_ok() {
        assert_eq!(translate_in(Lang::El, "btn.ok"), "OK");
    }

    #[test]
    fn test_translate_in_hu_check() {
        assert_eq!(translate_in(Lang::Hu, "board.check"), "Sakk!");
    }

    #[test]
    fn test_translate_in_cs_depth() {
        assert_eq!(translate_in(Lang::Cs, "analysis.depth"), "Hloubka");
    }

    #[test]
    fn test_translate_in_he_cancel() {
        assert_eq!(translate_in(Lang::He, "btn.cancel"), "ביטול");
    }

    #[test]
    fn test_translate_in_ar_cancel() {
        assert_eq!(translate_in(Lang::Ar, "btn.cancel"), "إلغاء");
    }

    #[test]
    fn test_translate_in_zh_cancel() {
        assert_eq!(translate_in(Lang::Zh, "btn.cancel"), "取消");
    }

    #[test]
    fn test_translate_in_ja_cancel() {
        assert_eq!(translate_in(Lang::Ja, "btn.cancel"), "キャンセル");
    }

    #[test]
    fn test_translate_in_fa_checkmate() {
        // Fixed upstream for Arabic (کیش/کیش‌مات confusion): here we
        // directly check "checkmate", not just "check".
        assert_eq!(translate_in(Lang::Fa, "board.checkmate"), "کیش‌مات");
    }

    #[test]
    fn test_translate_in_hi_cancel() {
        assert_eq!(translate_in(Lang::Hi, "btn.cancel"), "रद्द करें");
    }

    #[test]
    fn test_translate_in_ko_cancel() {
        assert_eq!(translate_in(Lang::Ko, "btn.cancel"), "취소");
    }

    #[test]
    fn test_translate_in_missing_key_returns_key() {
        // Nonexistent key → returns the key itself, no panic.
        let result = translate_in(Lang::Fr, "clé.inexistante");
        assert_eq!(result, "clé.inexistante");
    }

    #[test]
    fn test_translate_in_all_languages_spot_check() {
        // Checks that every language translates "app.name" to
        // "Vendetta Chess".
        for &lang in Lang::all() {
            let name = translate_in(lang, "app.name");
            assert_eq!(name, "Vendetta Chess", "app.name incorrect pour {lang:?}");
        }
    }

    // -----------------------------------------------------------------------
    // init / set_lang / current_lang / translate
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_and_get_lang() {
        init(Lang::Fr);
        set_lang(Lang::En);
        assert_eq!(current_lang(), Lang::En);
        // Reset to Fr so as not to disturb other tests.
        set_lang(Lang::Fr);
    }

    #[test]
    fn test_translate_uses_current_lang() {
        init(Lang::Es);
        assert_eq!(translate("board.white"), "Blancas");
        // Reset to Fr.
        set_lang(Lang::Fr);
    }

    #[test]
    fn test_translate_missing_key_fallback() {
        init(Lang::Fr);
        let result = translate("missing.key");
        assert_eq!(result, "missing.key");
    }
}
