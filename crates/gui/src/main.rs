//! Point d'entrée de Vendetta Chess GUI.

// Masque la fenêtre console qui s'ouvre sinon derrière la fenêtre Slint au
// double-clic sur l'exécutable Windows (et dont la fermeture tuait
// l'application). N'a d'effet que sur `target_os = "windows"` (ignoré
// ailleurs) et seulement en release : en debug on garde la console pour
// voir les logs/panics pendant le développement.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// Clippy (04/07/2026) : ce fichier assemble l'UI Slint avec le reste du
// domaine (échiquier, horloges, moteurs, tournois, puzzles) et manipule donc
// beaucoup d'indices/compteurs (`usize`/`i32`/`u32`) et de durées (`i64`/`u64`
// millisecondes) dont les bornes réelles (échiquier 0-7, ply/parties/ms d'une
// partie d'échecs) sont très loin des limites de précision/représentation des
// types cibles. Un seul `#![allow]` de fichier plutôt que des dizaines
// d'attributs locaux redondants (même esprit que le `#![allow(...)]` déjà en
// tête de `game_controller.rs`).
#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use std::{cell::{Cell, RefCell}, rc::Rc};
use std::fmt::Write as _;

use game_config::{GameConfig, GameMode, HumanColor, TimeControl};
use game_config::persist as gc_persist;
use gui::{AppWindow, GameDetailMoveRow, OpeningMoveRow, ReferenceGameListRow, SavedEngine, ScoreBar, TournamentStanding, Tr, UciOptionItem, analysis_bridge::DualAnalysisBridge, chess_clock::ChessClock, engine_scan, game_bridge::GameBridge, game_controller::{self, GameController}, i18n_bridge, pdf_export, png_export, prefs, puzzle_session::{MoveOutcome, PuzzleSession}, run_guarded_thread};
use gui::tournament_runner::{TournamentRunner, db_path as tournament_db_path};
use i18n::Lang;
use slint::{ComponentHandle as _, Model as _, ModelRc, VecModel};
use uci::protocol::GoLimits;
use chess_core::polyglot::PolyglotBook;
use chess_core::types::piece::Color;

/// Affichage prêt à l'emploi d'un score en pions, perspective blancs
/// (suivi ergonomie 10/07/2026 : bloc d'infos de "Détail de la partie").
/// Signe + 2 décimales (ex. "+1.25", "-0.34", "+0.00"). Pas de traitement
/// spécial "Mat" ici (aurait nécessité un texte codé en dur hors i18n) :
/// un score de mat forcé reste affiché saturé à ±50.00, exactement comme
/// `analysis_bridge::score_to_f32` le fait déjà pour la barre d'évaluation
/// et la courbe du plateau principal — convention existante, pas une
/// nouvelle incohérence.
fn format_score_display(score: f32) -> String {
    format!("{score:+.2}")
}

// ── Calcul des courbes SVG ────────────────────────────────────────────────────

/// Constantes de la viewbox SVG utilisée par `ScoreGraph`.
const VW: f32 = 1000.0;
const VH: f32 = 100.0;

/// Génère les commandes SVG des deux courbes (blancs et noirs).
///
/// Retourne `(white_path, black_path)`.
///
/// - `white_path` : aire remplie depuis la ligne centrale **vers le haut**
///   proportionnellement au score positif (avantage blancs).
/// - `black_path` : aire remplie depuis la ligne centrale **vers le bas**
///   proportionnellement au score négatif (avantage noirs).
fn compute_score_paths(scores: &[f32]) -> (String, String) {
    let n = scores.len();
    if n == 0 {
        return (String::new(), String::new());
    }

    let half = VH / 2.0;
    let bw = VW / n as f32;
    let xs: Vec<f32> = (0..n).map(|i| (i as f32 + 0.5) * bw).collect();

    let norms: Vec<f32> = scores
        .iter()
        .map(|&s| (s / 5.0).clamp(-1.0, 1.0))
        .collect();

    // Y de la courbe blancs (monte au-dessus du centre si score > 0)
    let yw: Vec<f32> = norms.iter().map(|&n| half - n.max(0.0) * (half - 2.0)).collect();
    // Y de la courbe noirs  (descend en-dessous si score < 0)
    let yb: Vec<f32> = norms.iter().map(|&n| half + (-n).max(0.0) * (half - 2.0)).collect();

    let white = build_area_path(&xs, &yw, half);
    let black = build_area_path(&xs, &yb, half);
    (white, black)
}

/// Construit un chemin SVG d'aire remplie entre `center_y` et la courbe `(xs, ys)`.
///
/// Utilise des béziers cubiques à tangentes horizontales pour des transitions
/// lisses entre chaque point.
fn build_area_path(xs: &[f32], ys: &[f32], center_y: f32) -> String {
    let n = xs.len();
    if n == 0 { return String::new(); }

    // Départ : bord gauche à la hauteur du premier point
    let mut p = format!("M 0 {:.2} L 0 {:.2} L {:.2} {:.2}",
        center_y, ys[0], xs[0], ys[0]);

    // Béziers cubiques entre les points successifs (tangentes horizontales)
    for i in 1..n {
        let cx = f32::midpoint(xs[i - 1], xs[i]);
        let _ = write!(p, " C {:.2} {:.2} {:.2} {:.2} {:.2} {:.2}",
            cx, ys[i - 1],   // CP1
            cx, ys[i],        // CP2
            xs[i], ys[i]);     // arrivée
    }

    // Extension au bord droit, retour à center_y, fermeture
    let _ = write!(p, " L {:.2} {:.2} L {:.2} {:.2} L 0 {:.2} Z",
        VW, ys[n - 1], VW, center_y, center_y);
    p
}

// ── Helpers d'affichage ───────────────────────────────────────────────────────

/// Met à jour toutes les propriétés Slint liées à l'état de la partie.
fn refresh_game_state(win: &AppWindow, ctrl: &GameController, lang: Lang) {
    win.set_squares(ModelRc::new(VecModel::from(ctrl.build_squares())));
    win.set_moves(ModelRc::new(VecModel::from(ctrl.build_move_rows())));
    win.set_viewed_ply(ctrl.viewed_ply_slint());
    // PHASE 26, Étape 3 : toujours resynchronisé ici plutôt que ponctuellement
    // dans chaque callback — le bandeau "Créer/Terminer une variante" ne doit
    // jamais afficher un état périmé après un coup, une promotion, un undo…
    win.set_variation_editing(ctrl.is_variation_editing());
    win.set_is_white_turn(ctrl.is_white_turn());
    win.set_status_text(i18n::translate_in(lang, ctrl.status_key()).into());
    push_captured_pieces(win, ctrl);

    let over = ctrl.is_over();
    win.set_is_game_over(over);
    if over {
        let reason_key = ctrl.end_reason_key();
        let result_key = ctrl.status_key();
        win.set_game_over_result(i18n::translate_in(lang, result_key).into());
        win.set_game_over_reason(i18n::translate_in(lang, reason_key).into());
    }
}

/// Met à jour les bandes de pièces capturées (au-dessus/en-dessous de
/// l'échiquier) et le différentiel de matériel depuis l'état courant du
/// contrôleur. Respecte le ply visualisé en mode historique (voir
/// `GameController::captured_summary`).
fn push_captured_pieces(win: &AppWindow, ctrl: &GameController) {
    let (white_trophies, black_trophies, diff) = ctrl.captured_summary();
    win.set_captured_by_white(ModelRc::new(VecModel::from(white_trophies)));
    win.set_captured_by_black(ModelRc::new(VecModel::from(black_trophies)));
    win.set_material_diff(diff);
}

/// Construit les limites UCI optimales pour le moteur-joueur.
///
/// - Horloge de partie active → `go wtime X btime Y winc Z binc Z`
///   Le moteur gère son temps dans les limites de l'horloge, comme un humain.
/// - Pas d'horloge → `None` : le thread moteur utilise son movetime fixe (Level/MoveTime).
fn build_go_limits(clock: &ChessClock) -> Option<GoLimits> {
    if !clock.has_clock() { return None; }
    let inc = clock.increment_ms().max(0) as u64;
    Some(GoLimits {
        wtime: Some(clock.white_ms().max(1) as u64),
        btime: Some(clock.black_ms().max(1) as u64),
        winc:  Some(inc),
        binc:  Some(inc),
        ..GoLimits::default()
    })
}

/// Met à jour les 4 propriétés Slint de l'horloge depuis un `ChessClock`.
fn push_clock_to_window(win: &AppWindow, clk: &ChessClock) {
    let wms = clk.white_ms();
    let bms = clk.black_ms();
    win.set_white_clock_text(ChessClock::format(wms).into());
    win.set_black_clock_text(ChessClock::format(bms).into());
    win.set_white_clock_ms(wms.max(0).min(i64::from(i32::MAX)) as i32);
    win.set_black_clock_ms(bms.max(0).min(i64::from(i32::MAX)) as i32);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convertit un code ISO ("fr", "en", …) en [`Lang`].
///
/// Délègue à `Lang::from_code` (crates/i18n) plutôt que de dupliquer un match
/// ici : l'ancienne version ne couvrait que les 11 premières langues et
/// faisait silencieusement retomber tout code inconnu sur le français — un
/// piège découvert lors de l'extension à 33 langues (les 22 nouvelles
/// auraient toutes été mal reconnues sans ce correctif).
///
/// Retombe sur `Lang::default()` (anglais, voir `#[default]` sur `Lang::En`
/// dans `crates/i18n`) plutôt que sur `Lang::Fr` codé en dur — changement du
/// 05/07/2026, demande explicite de l'utilisateur.
fn parse_lang_code(code: &str) -> Lang {
    Lang::from_code(code).unwrap_or_default()
}

// ── Flèche conseil ───────────────────────────────────────────────────────────

/// Génère les commandes SVG d'une flèche dans le référentiel de l'échiquier (viewbox 8×8).
///
/// Prend en compte le flip : si `flipped=true`, les positions visuelles sont inversées.
/// Retourne une chaîne vide si la case source est identique à la destination.
// Clippy : `#[allow(similar_names)]` — `vc_from`/`vr_from`/`vc_to`/`vr_to`
// (colonne/rangée visuelles, origine/destination) sont volontairement
// symétriques, pas une confusion accidentelle.
#[allow(clippy::similar_names)]
fn compute_hint_arrow(
    from_row: i32, from_col: i32,
    to_row: i32, to_col: i32,
    flipped: bool,
) -> String {
    // Conversion en coordonnées visuelles selon la perspective
    let (vc_from, vr_from, vc_to, vr_to) = if flipped {
        (7 - from_col, 7 - from_row, 7 - to_col, 7 - to_row)
    } else {
        (from_col, from_row, to_col, to_row)
    };

    // Centres des cases en unités-case (0.0 .. 8.0)
    let x1 = vc_from as f32 + 0.5;
    let y1 = vr_from as f32 + 0.5;
    let x2 = vc_to   as f32 + 0.5;
    let y2 = vr_to   as f32 + 0.5;

    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 { return String::new(); }

    // Vecteur unitaire de direction + perpendiculaire
    let ux = dx / len;
    let uy = dy / len;
    let px = -uy;
    let py =  ux;

    // Dimensions de la flèche en unités-case
    let shaft_half: f32 = 0.10;  // demi-largeur du fût
    let head_half:  f32 = 0.26;  // demi-largeur de la tête
    let head_len:   f32 = 0.38;  // longueur de la tête

    // Point où le fût se termine (base de la tête)
    let hx = x2 - ux * head_len;
    let hy = y2 - uy * head_len;

    // 7 sommets du polygone en flèche :
    //  p1, p2  : flancs gauche/droite à la base du fût (côté source)
    //  p3, p4  : flancs gauche/droite à la jonction fût/tête
    //  p5, p6  : coins gauche/droite de la base de la tête (plus larges)
    //  p7      : pointe de la flèche
    let p = |x: f32, y: f32| format!("{x:.3} {y:.3}");

    let p1  = p(x1 + px * shaft_half,  y1 + py * shaft_half);
    let p2  = p(x1 - px * shaft_half,  y1 - py * shaft_half);
    let p3  = p(hx + px * shaft_half,  hy + py * shaft_half);
    let p4  = p(hx - px * shaft_half,  hy - py * shaft_half);
    let p5  = p(hx + px * head_half,   hy + py * head_half);
    let p6  = p(hx - px * head_half,   hy - py * head_half);
    let p7  = p(x2, y2);

    format!("M {p1} L {p3} L {p5} L {p7} L {p6} L {p4} L {p2} Z")
}

// ── Helpers gestion des moteurs ───────────────────────────────────────────────

/// Met à jour la propriété `saved-engines` de l'`AppWindow` depuis la liste Rust.
///
/// PHASE 73 : `path` porte désormais le chemin **relatif** à
/// `VendettaChess/` (via `app_paths::to_relative_string`) — utilisé tel quel
/// pour l'affichage (Préférences), la sélection (assistant Nouvelle partie)
/// et les comparaisons d'identité (dropdown "moteur conseil", voir
/// [`hint_engine_path_for_window`]). `prefs::SavedEngine.path` (interne,
/// résolu absolu par `prefs::load_engines`) n'est pas modifié — seule cette
/// conversion, faite ici juste avant l'envoi à Slint, change. La résolution
/// en absolu nécessaire pour réellement lancer un moteur n'a lieu qu'au tout
/// dernier moment dans `GameBridge::init` (`game_bridge.rs`).
fn update_engines_in_window(win: &AppWindow, engines: &[prefs::SavedEngine]) {
    let slint_engines: Vec<SavedEngine> = engines
        .iter()
        .map(|e| SavedEngine {
            name: e.name.clone().into(),
            path: app_paths::to_relative_string(std::path::Path::new(&e.path)).into(),
        })
        .collect();
    win.set_saved_engines(ModelRc::new(VecModel::from(slint_engines)));
}

/// PHASE 73 — convertit le chemin (absolu) du moteur conseil courant en sa
/// forme relative, pour la seule propriété Slint `hint-engine-path` :
/// `HintEngineDropdown` la compare à `engine.path` (désormais relatif lui
/// aussi, voir [`update_engines_in_window`]) pour savoir quelle ligne
/// surligner. Le `Rc<RefCell<Option<String>>>` `hint_engine_path` côté Rust
/// n'est pas concerné par cette conversion : il reste en absolu, utilisé
/// pour réellement lancer le moteur conseil (`on_request_hint`) et
/// synchroniser le moteur d'analyse (`sync_analysis_engine`).
fn hint_engine_path_for_window(path: Option<&str>) -> String {
    path.map_or_else(String::new, |p| app_paths::to_relative_string(std::path::Path::new(p)))
}

// ── Helpers books d'ouvertures Polyglot (PHASE 15, Étape 6 ; PHASE 24, Étape 6) ─

/// Recharge un book Polyglot depuis un chemin de fichier connu (PHASE 24 :
/// `ouvertures/blancs.bin`/`noirs.bin`, via [`book_path_if_exists`]).
///
/// Retourne `None` si aucun chemin n'est fourni, ou si le fichier a
/// disparu/changé/est devenu invalide depuis le dernier chargement réussi en
/// Préférences — dans ce cas le jeu continue normalement, comme si aucun book
/// n'était configuré (aucun crash, aucun blocage).
fn load_runtime_book(path: Option<String>) -> Option<PolyglotBook> {
    let path = path?;
    PolyglotBook::open(std::path::Path::new(&path)).ok()
}

/// Retourne `path` sous forme de `String` s'il existe sur le disque, `None`
/// sinon (PHASE 24, Étape 6) — utilisé pour vérifier la présence de
/// `ouvertures/blancs.bin`/`noirs.bin` avant de tenter de les charger.
fn book_path_if_exists(path: &std::path::Path) -> Option<String> {
    path.exists().then(|| path.to_string_lossy().into_owned())
}

/// Tente de jouer, sans appeler le moteur, un ou plusieurs coups depuis les
/// books Polyglot configurés (Blancs/Noirs), tant que le camp au trait en a
/// un pour la position courante. Boucle nécessaire en M vs M si les deux
/// camps ont un book (leurs coups s'enchaînent sans calcul moteur).
///
/// Ne fait jamais rien en mode tournoi (décision explicite, PHASE 15) : le
/// book ne s'applique qu'aux parties normales (H vs M, M vs M, H vs H — sans
/// effet dans ce dernier cas puisqu'aucun camp moteur n'est déclenché de
/// toute façon).
///
/// Ne s'applique qu'au(x) camp(s) réellement joué(s) par un moteur
/// (`game_bridge.has_white_engine()`/`has_black_engine()`) — jamais à un camp
/// humain, même si un book est chargé pour ce camp. Sans cette vérification,
/// un book configuré pour la couleur d'un joueur humain lui "volerait" son
/// premier coup dès le lancement de la partie.
///
/// Retourne `true` si au moins un coup de book a été joué — l'appelant doit
/// alors rafraîchir l'affichage et recalculer le trait/FEN avant de décider
/// si le moteur doit être sollicité pour le camp désormais au trait.
fn try_play_book_moves(
    win: &AppWindow,
    controller: &Rc<RefCell<GameController>>,
    chess_clock: &Rc<RefCell<ChessClock>>,
    game_bridge: &Rc<RefCell<GameBridge>>,
    book_white: &Rc<RefCell<Option<PolyglotBook>>>,
    book_black: &Rc<RefCell<Option<PolyglotBook>>>,
) -> bool {
    // Plafond dur (robustesse audit 11/07/2026, finding 3.3) : la boucle
    // ci-dessous est déjà bornée indirectement par la détection de
    // répétition triple de `ChessGame::is_over()` (un cycle A↔B entre les
    // deux books finit par être vu comme nulle et `is_over` devient vrai),
    // mais aucun garde-fou n'existait avant ce plafond si un fichier `.bin`
    // Polyglot corrompu ou pathologiquement construit enchaînait un très
    // grand nombre de positions distinctes avant de reboucler — la boucle
    // tournerait alors sur le thread UI sans jamais rendre la main, gelant
    // l'interface le temps que ça dure. 200 coups de book enchaînés dépasse
    // très largement tout usage réel (même une ouverture de livre géante
    // ne va jamais aussi loin sans qu'un moteur ne prenne le relais).
    // Déclarée en tête de fonction plutôt que juste avant la boucle
    // (clippy::items_after_statements — un `const` après des instructions
    // porte à confusion, sa portée couvrant en réalité toute la fonction).
    const MAX_BOOK_MOVES: u32 = 200;

    gui::debug_log::log_event("try_play_book_moves_called", &serde_json::json!({
        "tournament_mode": win.get_is_tournament_mode(),
        "variation_editing": controller.borrow().is_variation_editing(),
    }));
    if win.get_is_tournament_mode() {
        return false;
    }
    // PHASE 26, Étape 2 : aucun automatisme (book compris) ne doit jouer à
    // la place de l'utilisateur pendant l'édition de variante.
    if controller.borrow().is_variation_editing() {
        gui::debug_log::log_event("try_play_book_moves_blocked_variation_editing", &serde_json::json!({}));
        return false;
    }

    let mut played_any = false;
    let mut sans_played: Vec<String> = Vec::new();
    let mut moves_played: u32 = 0;

    loop {
        if moves_played >= MAX_BOOK_MOVES {
            gui::debug_log::log_event(
                "try_play_book_moves_hit_hard_cap",
                &serde_json::json!({ "cap": MAX_BOOK_MOVES }),
            );
            break;
        }
        moves_played += 1;

        let (is_white_turn, is_over, fen) = {
            let ctrl = controller.borrow();
            (ctrl.is_white_turn(), ctrl.is_over(), ctrl.current_fen())
        };
        if is_over {
            break;
        }

        // Camp humain ? Le book ne s'applique jamais à un camp non-moteur.
        let side_has_engine = {
            let bridge = game_bridge.borrow();
            if is_white_turn { bridge.has_white_engine() } else { bridge.has_black_engine() }
        };
        if !side_has_engine {
            break;
        }

        let book_ref = if is_white_turn { book_white } else { book_black };
        let uci_opt = {
            let book_opt = book_ref.borrow();
            book_opt.as_ref().and_then(|book| {
                chess_core::types::Position::from_fen(&fen)
                    .ok()
                    .and_then(|pos| book.pick_uci_move(&pos))
            })
        };
        let Some(uci) = uci_opt else { break; };

        let applied = controller.borrow_mut().apply_uci_move_from_book(&uci);
        if !applied {
            // Coup de book illisible dans cette position (fichier incohérent
            // avec les règles réelles) — abandon silencieux, le moteur ou
            // l'humain reprend la main normalement.
            break;
        }

        // Bonus Fischer/incrément éventuel pour le camp qui vient de jouer.
        // L'ordre d'application n'a pas d'effet visible tant que la boucle
        // reste dans le même tour d'événements Slint (le tick d'affichage de
        // l'horloge est un timer séparé) — seul le décompte final importe.
        chess_clock.borrow_mut().apply_move_bonus(is_white_turn);
        played_any = true;

        if let Some(san) = controller.borrow().last_move_san() {
            sans_played.push(san);
        }
    }

    // Notification visuelle "coup de book joué" (demande utilisateur du
    // 03/07/2026 : aucun moyen de vérifier que le book est réellement
    // utilisé). Un seul toast pour toute la série de coups enchaînés dans cet
    // appel (ex. M vs M où les deux camps ont un book). Le compteur
    // `book-notification-seq` est incrémenté à chaque déclenchement — même si
    // le texte est identique au précédent — car le callback `changed` côté
    // Slint ne se redéclenche que sur changement réel de valeur.
    if !sans_played.is_empty() {
        let text = if sans_played.len() == 1 {
            i18n::translate("status.book_move_single").replace("{move}", &sans_played[0])
        } else {
            i18n::translate("status.book_move_multiple")
                .replace("{moves}", &sans_played.join(" "))
        };
        win.set_book_notification_text(text.into());
        win.set_book_notification_seq(win.get_book_notification_seq() + 1);
    }

    played_any
}

// ── Helpers mode Puzzle (PHASE 14, Étape 6) ────────────────────────────────────

/// Phrase objectif neutre affichée dès le début de la résolution, quel que
/// soit le réglage "Avec thème"/"Sans indice" (qui ne conditionne que
/// l'affichage du thème en plus, voir [`set_puzzle_banner_for_new_puzzle`]).
fn puzzle_objective_title(hero_white: bool) -> String {
    if hero_white {
        i18n::translate("status.puzzle_turn_white")
    } else {
        i18n::translate("status.puzzle_turn_black")
    }
}

/// Sous-titre "Thème : … • Cote …" à partir de la ligne de puzzle d'origine.
/// Utilisé aussi bien pour la révélation immédiate (réglage "Avec thème")
/// que pour le bandeau de résultat final (thème toujours révélé une fois la
/// tentative terminée, quel que soit le réglage choisi au départ).
fn puzzle_theme_subtitle(session: &PuzzleSession) -> String {
    let p = session.puzzle();
    let themes = if p.themes.trim().is_empty() { "—" } else { p.themes.trim() };
    i18n::translate("status.puzzle_theme_subtitle")
        .replace("{theme}", themes)
        .replace("{rating}", &p.rating.to_string())
}

/// Initialise le bandeau `PuzzleControlBar` pour un puzzle qui vient d'être
/// chargé (phrase objectif + thème immédiatement révélé si `puzzle-hint-theme`
/// est actif, sinon masqué jusqu'à la fin de la tentative).
fn set_puzzle_banner_for_new_puzzle(win: &AppWindow, session: &PuzzleSession) {
    let hero_white = session.hero_color() == Color::White;
    win.set_puzzle_title(puzzle_objective_title(hero_white).into());
    let subtitle = if win.get_puzzle_hint_theme() {
        puzzle_theme_subtitle(session)
    } else {
        String::new()
    };
    win.set_puzzle_subtitle(subtitle.into());
    win.set_puzzle_solved(false);
    win.set_puzzle_failed(false);
    win.set_puzzle_show_reveal(true);
}

/// Suffixe `" (N erreur(s))"` (ou `" (après N erreur(s))"` si `after` est
/// `true`) à ajouter au titre du bandeau de résultat — chaîne vide si
/// aucun coup faux n'a été tenté (PHASE 14, Étape 7).
fn error_count_suffix(errors: u32, after: bool) -> String {
    match errors {
        0 => String::new(),
        1 => i18n::translate(if after {
            "status.error_suffix_one_after"
        } else {
            "status.error_suffix_one_now"
        }),
        n => i18n::translate(if after {
            "status.error_suffix_many_after"
        } else {
            "status.error_suffix_many_now"
        })
        .replace("{n}", &n.to_string()),
    }
}

/// Bandeau affiché une fois la tentative terminée (résolue, séquence
/// interrompue, révélée, ou abandonnée) — révèle systématiquement le thème
/// et la cote, indépendamment du réglage "Avec thème"/"Sans indice" qui ne
/// conditionne que l'affichage *pendant* la résolution. Précise en plus le
/// nombre de coups faux tentés (PHASE 14, Étape 7), sauf séquence corrompue
/// (donnée invalide, sans rapport avec la performance du joueur).
fn finish_puzzle_banner(win: &AppWindow, session: &PuzzleSession) {
    let solved = session.is_solved();
    win.set_puzzle_solved(solved);
    win.set_puzzle_failed(!solved);
    win.set_puzzle_show_reveal(false);

    let errors = session.wrong_attempts_count();
    let title = if solved {
        format!(
            "{}{}",
            i18n::translate("status.puzzle_solved_title"),
            error_count_suffix(errors, false)
        )
    } else if session.is_broken() {
        i18n::translate("status.puzzle_broken_title")
    } else if session.is_revealed() {
        format!(
            "{}{}",
            i18n::translate("status.puzzle_revealed_title"),
            error_count_suffix(errors, true)
        )
    } else {
        format!(
            "{}{}",
            i18n::translate("status.puzzle_failed_title"),
            error_count_suffix(errors, true)
        )
    };
    win.set_puzzle_title(title.into());
    win.set_puzzle_subtitle(puzzle_theme_subtitle(session).into());
}

/// Formate et pousse les statistiques globales de puzzles (PHASE 14, Étape 8)
/// dans `puzzle-stats-text` — "127 tenté(s) · 89 résolu(s) · 70 % de
/// réussite", ou un texte neutre si aucune tentative n'a encore été
/// enregistrée. Affichée uniquement en Préférences (choix validé avec
/// l'utilisateur — pas dans l'assistant ni pendant la résolution).
fn push_puzzle_stats(win: &AppWindow, stats: db::repository::puzzle_repo::PuzzleStats) {
    let text = if stats.total_attempted <= 0 {
        i18n::translate("status.no_puzzle_attempts")
    } else {
        let rate = (stats.total_solved as f64 / stats.total_attempted as f64 * 100.0).round() as i64;
        i18n::translate("status.puzzle_stats_summary")
            .replace("{attempted}", &stats.total_attempted.to_string())
            .replace("{solved}", &stats.total_solved.to_string())
            .replace("{rate}", &rate.to_string())
    };
    win.set_puzzle_stats_text(text.into());
}

/// Enregistre la tentative en base selon la règle de comptage validée avec
/// l'utilisateur (03/07/2026, voir [`PuzzleSession::outcome_for_stats`]) —
/// n'écrit rien si la tentative est neutre (abandon sans coup faux tenté).
/// Rafraîchit ensuite l'affichage Préférences (`push_puzzle_stats`) pour
/// rester à jour sans redémarrer l'application (PHASE 14, Étape 8).
/// Échec silencieux si la base est inaccessible, cohérent avec le traitement
/// des autres erreurs DB non bloquantes ailleurs dans le projet.
fn record_puzzle_stats(win: &AppWindow, session: &PuzzleSession) {
    let Some(result) = session.outcome_for_stats() else { return; };
    if let Ok(conn) = db::schema::open_and_migrate(&tournament_db_path()) {
        let _ = db::repository::puzzle_repo::record_attempt(&conn, session.puzzle().id, result);
        if let Ok(stats) = db::repository::puzzle_repo::global_stats(&conn) {
            push_puzzle_stats(win, stats);
        }
    }
}

/// Tire un puzzle aléatoire en base et l'affiche, prêt à résoudre.
///
/// Factorise la logique commune entre le lancement depuis l'assistant
/// (`on_setup_start_puzzle`) et le bouton "Puzzle suivant" (`on_puzzle_next`) :
/// tirage, construction de la session, réinitialisation complète de
/// l'affichage (échiquier/horloge/analyse), orientation automatique vers le
/// camp qui résout, bandeau objectif initial, et déclenchement de la barre
/// d'évaluation forcée (Multi-PV=1, jamais affichée en texte — voir gating
/// dans `app.slint`).
///
/// Affiche une boîte de dialogue et retourne `false` en cas d'échec (base
/// inaccessible, aucun puzzle en base, ligne corrompue) sans modifier l'état
/// courant — utile pour "Puzzle suivant" en cas d'erreur transitoire.
// Clippy (04/07/2026) : `#[allow(too_many_arguments)]` — regroupe tout l'état
// partagé nécessaire (fenêtre, contrôleur, bridges, horloge, historique,
// session de puzzle, langue) ; un découpage en struct changerait l'API de
// tous les call sites pour un lint mineur, hors périmètre de ce correctif.
#[allow(clippy::too_many_arguments)]
fn load_random_puzzle(
    win: &AppWindow,
    controller: &Rc<RefCell<GameController>>,
    analysis: &Rc<RefCell<DualAnalysisBridge>>,
    game_bridge: &Rc<RefCell<GameBridge>>,
    chess_clock: &Rc<RefCell<ChessClock>>,
    score_history: &Rc<RefCell<Vec<f32>>>,
    puzzle_session: &Rc<RefCell<Option<PuzzleSession>>>,
    lang: Lang,
) -> bool {
    let conn = match db::schema::open_and_migrate(&tournament_db_path()) {
        Ok(c) => c,
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.db_unavailable_title"))
                .set_description(format!("{e}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return false;
        }
    };

    let row = match db::repository::puzzle_repo::random_puzzle(&conn) {
        Ok(Some(row)) => row,
        Ok(None) => {
            rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.no_puzzle_available_title"))
                .set_description(i18n::translate("dialog.no_puzzle_available_desc"))
                .set_level(rfd::MessageLevel::Warning)
                .show();
            return false;
        }
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.db_error_title"))
                .set_description(format!("{e}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return false;
        }
    };

    let session = match PuzzleSession::new(&row) {
        Ok(s) => s,
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.invalid_puzzle_title"))
                .set_description(
                    i18n::translate("dialog.invalid_puzzle_desc")
                        .replace("{id}", &row.puzzle_id)
                        .replace("{err}", &e.to_string()),
                )
                .set_level(rfd::MessageLevel::Error)
                .show();
            return false;
        }
    };

    analysis.borrow_mut().stop();
    game_bridge.borrow_mut().init(&GameConfig::human_vs_human(), &win.as_weak());
    chess_clock.borrow_mut().stop();

    {
        let mut ctrl = controller.borrow_mut();
        ctrl.load_from_fen(&session.position().to_fen());
        // PHASE 68 — décision actée avec l'utilisateur : le mode Assistance
        // est désactivé automatiquement en mode Puzzle (résoudre un puzzle
        // suppose de calculer soi-même). Le bouton est déjà masqué côté
        // Slint (`current-game-mode != 4`), mais l'état Rust doit lui aussi
        // repasser à `false` pour ne pas laisser les badges actifs si
        // l'utilisateur avait activé le mode juste avant de lancer un puzzle.
        ctrl.set_assist_mode(false);
    }
    win.set_assist_mode_active(false);
    score_history.borrow_mut().clear();

    win.set_show_promotion_modal(false);
    win.set_is_game_over(false);
    win.set_game_over_result(slint::SharedString::default());
    win.set_game_over_reason(slint::SharedString::default());
    win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
    win.set_white_curve_path(slint::SharedString::default());
    win.set_black_curve_path(slint::SharedString::default());
    win.set_engine_depth("—".into());
    win.set_engine_score("—".into());
    win.set_engine_pv("—".into());
    win.set_game_paused(false);
    win.set_engine_thinking(false);
    win.set_engine_playing(false);
    win.set_eval_bar_visible(false);
    win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
    win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
    win.set_hint_arrow_path("".into());
    win.set_pv_selected_rank_white(0);
    win.set_pv_selected_rank_black(0);
    win.set_hint_computing(false);
    win.set_current_game_mode(4); // 4 = Puzzle

    // Pas d'horloge en mode Puzzle (résolution non chronométrée).
    win.set_show_clocks(false);
    win.set_white_clock_active(false);
    win.set_black_clock_active(false);
    win.set_white_clock_text("--:--".into());
    win.set_black_clock_text("--:--".into());

    // Orientation automatique : toujours du point de vue du camp qui résout
    // (décision de conception validée, PHASE 14).
    win.set_board_flipped(session.hero_color() == Color::Black);

    set_puzzle_banner_for_new_puzzle(win, &session);
    win.set_puzzle_mode_active(true);
    win.set_puzzle_feedback_text("".into());

    refresh_game_state(win, &controller.borrow(), lang);

    // Barre d'éval forcée dès le premier trait du joueur, indépendamment des
    // préférences Multi-PV normales (toujours 1 ligne, jamais affichée en
    // texte — voir gating Slint dans app.slint).
    if !controller.borrow().is_over() {
        let hero_is_white = session.hero_color() == Color::White;
        let fen = controller.borrow().current_fen();
        analysis.borrow_mut().start_for(hero_is_white, fen, win.as_weak(), hero_is_white, 1);
    }

    *puzzle_session.borrow_mut() = Some(session);
    true
}

/// Traite un coup humain venant d'être commis par `GameController` en mode
/// Puzzle : validation contre `PuzzleSession`, rejet visuel (undo) si
/// incorrect, enchaînement de la réponse adverse si correct, bandeau de
/// résultat + enregistrement des statistiques si résolu ou séquence
/// interrompue.
///
/// Appelée juste après que `ctrl.on_click`/`ctrl.complete_promotion` a déjà
/// commis le coup côté `GameController` — récupère ce coup via
/// `GameController::last_move_uci` plutôt que de dupliquer la sélection en
/// deux clics dans ce module (voir note de conception `puzzle_session.rs`).
fn handle_puzzle_move(
    win: &AppWindow,
    controller: &Rc<RefCell<GameController>>,
    puzzle_session: &Rc<RefCell<Option<PuzzleSession>>>,
    analysis: &Rc<RefCell<DualAnalysisBridge>>,
    lang: Lang,
) {
    let Some(uci) = controller.borrow().last_move_uci() else { return; };

    let mut session_slot = puzzle_session.borrow_mut();
    let Some(session) = session_slot.as_mut() else { return; };

    match session.try_move_uci(&uci) {
        MoveOutcome::Incorrect => {
            // Réessai illimité : le coup faux est annulé (position identique
            // à avant la tentative), notification rouge brève.
            controller.borrow_mut().undo_last_move();
            refresh_game_state(win, &controller.borrow(), lang);
            win.set_puzzle_feedback_text(i18n::translate("status.puzzle_feedback_incorrect").into());
            win.set_puzzle_feedback_positive(false);
            win.set_puzzle_feedback_seq(win.get_puzzle_feedback_seq() + 1);
        }
        MoveOutcome::Illegal => {
            // Ne devrait pas arriver : le coup vient d'être validé légal par
            // GameController lui-même. Purement défensif, aucune action.
        }
        MoveOutcome::CorrectContinue(opponent_reply) => {
            controller.borrow_mut().apply_uci_move(&opponent_reply.to_uci());
            refresh_game_state(win, &controller.borrow(), lang);
            win.set_puzzle_feedback_text(i18n::translate("status.puzzle_feedback_correct").into());
            win.set_puzzle_feedback_positive(true);
            win.set_puzzle_feedback_seq(win.get_puzzle_feedback_seq() + 1);

            if !controller.borrow().is_over() {
                let hero_is_white = session.hero_color() == Color::White;
                let fen = controller.borrow().current_fen();
                analysis.borrow_mut().start_for(hero_is_white, fen, win.as_weak(), hero_is_white, 1);
            }
        }
        MoveOutcome::Solved => {
            win.set_puzzle_feedback_text(i18n::translate("status.puzzle_feedback_solved").into());
            win.set_puzzle_feedback_positive(true);
            win.set_puzzle_feedback_seq(win.get_puzzle_feedback_seq() + 1);
            finish_puzzle_banner(win, session);
            record_puzzle_stats(win, session);
            analysis.borrow_mut().stop();
        }
        MoveOutcome::CorrectButSequenceBroken => {
            finish_puzzle_banner(win, session);
            record_puzzle_stats(win, session);
            analysis.borrow_mut().stop();
        }
    }
}

// ── Helpers tournoi ───────────────────────────────────────────────────────────

/// Met à jour le panneau classement dans la fenêtre Slint à partir de l'état
/// courant du `TournamentRunner`.
///
/// Pousse : standings, games-played, total-games, progress.
fn push_tournament_standings(win: &AppWindow, tr: &TournamentRunner) {
    let sorted = tr.state.standings();
    let rows: Vec<TournamentStanding> = sorted
        .iter()
        .enumerate()
        .map(|(i, s)| TournamentStanding {
            rank:   format!("#{}", i + 1).into(),
            name:   s.name.clone().into(),
            score:  format!("{:.1}", s.points).into(),
            wins:   s.wins.to_string().into(),
            draws:  s.draws.to_string().into(),
            losses: s.losses.to_string().into(),
        })
        .collect();

    win.set_tournament_standings(ModelRc::new(VecModel::from(rows)));

    let played   = tr.games_played();
    let total    = tr.total_games();
    let progress = if total > 0 { played as f32 / total as f32 } else { 0.0 };
    win.set_tournament_games_played(played as i32);
    win.set_tournament_total_games(total as i32);
    win.set_tournament_progress(progress);
}

// ── Phase 11.3 helpers — evaluation bar ───────────────────────────────────────

// ── Position editor (Phase 12) ────────────────────────────────────────────────

/// Converts a Slint piece identifier ("wK".."bP") into a FEN character.
fn piece_id_to_fen(id: &str) -> char {
    match id {
        "wK" => 'K', "wQ" => 'Q', "wR" => 'R',
        "wB" => 'B', "wN" => 'N', "wP" => 'P',
        "bK" => 'k', "bQ" => 'q', "bR" => 'r',
        "bB" => 'b', "bN" => 'n', "bP" => 'p',
        _    => '?',
    }
}

/// Builds a FEN from the position editor's current state.
///
/// The 64 squares are in row 0..7 × col 0..7 order (index = row*8+col).
/// Row 0 = rank 8 (top of the board), row 7 = rank 1 (bottom).
/// En passant is always "-" (not available in the visual editor).
// Clippy (04/07/2026): `#[allow(fn_params_excessive_bools, similar_names)]` —
// the 5 bools are the FIDE castling rights (white/black × kingside/queenside)
// plus the side to move, each individually meaningful to the Slint caller;
// grouping them into enums would add indirection with no clarity gain for
// a simple FEN generator. `castle_wk`/`castle_wq`/`castle_bk`/`castle_bq`
// follow standard FIDE notation (king/queen, white/black), not an
// accidental mix-up.
#[allow(clippy::fn_params_excessive_bools, clippy::similar_names)]
fn build_editor_fen(
    squares:       &[gui::SquareData],
    white_to_move: bool,
    castle_wk:     bool,
    castle_wq:     bool,
    castle_bk:     bool,
    castle_bq:     bool,
) -> String {
    // ── 1. Piece placement ─────────────────────────────────────────────────────
    let mut placement = String::new();
    for row in 0..8_i32 {
        let mut empty = 0_i32;
        for col in 0..8_i32 {
            let pc = squares
                .get((row * 8 + col) as usize)
                .map_or("", |sq| sq.piece_char.as_str());
            if pc.is_empty() {
                empty += 1;
            } else {
                if empty > 0 { placement.push_str(&empty.to_string()); empty = 0; }
                placement.push(piece_id_to_fen(pc));
            }
        }
        if empty > 0 { placement.push_str(&empty.to_string()); }
        if row < 7 { placement.push('/'); }
    }

    // ── 2. Side to move ─────────────────────────────────────────────────────────
    let color = if white_to_move { "w" } else { "b" };

    // ── 3. Castling ───────────────────────────────────────────────────────────
    let mut castling = String::new();
    if castle_wk { castling.push('K'); }
    if castle_wq { castling.push('Q'); }
    if castle_bk { castling.push('k'); }
    if castle_bq { castling.push('q'); }
    if castling.is_empty() { castling.push('-'); }

    format!("{placement} {color} {castling} - 0 1")
}

/// Synchronizes the analysis engine and returns its short name for display.
/// Returns "" if no engine is available.
///
/// Priority order (see also the module doc):
/// 1. Hint engine selected by the user in Preferences.
/// 2. First engine saved in the Preferences → Engines list.
/// 3. Auto-discovered engine (`vendetta_chess_motor` in PATH) — `AnalysisBridge::new()`'s default.
///
/// Must be called on every change: startup, engine added/removed, hint selection.
fn sync_analysis_engine(
    analysis: &mut DualAnalysisBridge,
    hint_path: Option<&String>,
    saved: &[prefs::SavedEngine],
) -> String {
    if let Some(p) = hint_path {
        // Priority 1: hint engine
        analysis.set_engine_path(p.clone());
        // Look up the name in the saved list, otherwise derive it from the path.
        // PHASE 74 — comparison via `Path` (not a raw `String`
        // equality): see the PHASE 71 fix (engine_scan.rs) for details
        // on the risk of mixed separators on Windows when
        // resolving a multi-component relative path.
        saved.iter()
            .find(|e| std::path::Path::new(&e.path) == std::path::Path::new(p))
            .map_or_else(|| {
                std::path::Path::new(p)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("moteur conseil")
                    .to_owned()
            }, |e| e.name.clone())
    } else if let Some(e) = saved.first() {
        // Priority 2: first saved engine
        analysis.set_engine_path(e.path.clone());
        e.name.clone()
    } else {
        // No engine available
        String::new()
    }
}

/// Name displayed in the "Hint engine" dropdown of the Preferences panel
/// (PHASE 52 — `HintEngineDropdown`, preferences.slint). "" if no hint
/// engine is selected (`path` = `None`).
///
/// Uses the same fallback as [`sync_analysis_engine`] when the selected
/// path no longer matches any engine in `saved` (engine removed from
/// the list since): a name derived from the path rather than an empty string, to
/// stay consistent with the name shown in the analysis bar in that same
/// case.
fn hint_engine_display_name(path: Option<&str>, saved: &[prefs::SavedEngine]) -> String {
    let Some(p) = path else { return String::new(); };
    // PHASE 74 — comparison via `Path`, see the equivalent comment in
    // `sync_analysis_engine`.
    saved.iter()
        .find(|e| std::path::Path::new(&e.path) == std::path::Path::new(p))
        .map_or_else(|| {
            std::path::Path::new(p)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("moteur conseil")
                .to_owned()
        }, |e| e.name.clone())
}

/// Returns the absolute path (as a `String`, the format expected by
/// `db::reference_schema::open_and_migrate`) of the `SQLite` database of
/// reference games **imported from a PGN** (PHASE 82). Same defensive
/// principle as `tournament_runner::db_path`: the `bases_parties/` folder
/// is normally already created at startup by `app_paths::ensure_app_dirs()`,
/// but `create_dir_all` is still called here defensively.
///
/// Decision made in discussion (11/07/2026): the PGN database and the SCID
/// database (see [`reference_scid_db_path`]) are two separate `SQLite`
/// files — importing one never touches the other.
fn reference_pgn_db_path() -> String {
    let path = app_paths::reference_pgn_db_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    path.to_string_lossy().into_owned()
}

/// Identical to [`reference_pgn_db_path`], for the reference games
/// database **imported from SCID** (`.si4`/`.si5`, see `crates/scid`).
fn reference_scid_db_path() -> String {
    let path = app_paths::reference_scid_db_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    path.to_string_lossy().into_owned()
}

/// Path of the reference games database **currently selected
/// in the browser** (`reference-browser-source`: 0 = PGN, 1 = SCID),
/// for everything related to BROWSING (search, opening tree,
/// game detail...) — see the "two databases" discussion from 11/07/2026.
/// Imports/clears, on the other hand, are always explicitly tied to a single
/// source (see `on_import_reference_base`/`on_import_reference_si4_base`/
/// `on_clear_reference_base`/`on_clear_reference_si4_base`), not to this
/// selector.
fn current_reference_db_path(win: &AppWindow) -> String {
    if win.get_reference_browser_source() == 1 {
        reference_scid_db_path()
    } else {
        reference_pgn_db_path()
    }
}

/// Result of replaying an opening-tree path: the resulting position, its
/// move list formatted for display, and the origin/destination squares of
/// the last move played (`None` for the starting position) — see
/// [`replay_opening_tree_path`] (`clippy::type_complexity`: factored out of
/// the function signature into a named alias).
type OpeningTreePathReplay = (
    chess_core::types::Position,
    String,
    Option<(chess_core::types::Square, chess_core::types::Square)>,
);

/// Replays `path` (moves in UCI format, e.g. `["e2e4", "e7e5"]`) from the
/// standard starting position — used by the opening tree of the exploration
/// screen (PHASE 82, step 8). Returns the reached position along with
/// a pre-formatted SAN breadcrumb ("1. e4 e5 2. Nf3"), or an empty
/// string if `path` is empty (the "Starting position" display is handled
/// on the Slint side in that case, see `reference_browser.slint`).
///
/// Does not go through `GameState`/`History` (unnecessary for a simple
/// breadcrumb outside a real game): reuses the "resolve then
/// apply" pattern already used by `GameController::apply_uci_move_impl` and
/// `puzzle_session::resolve_legal_move` — `Move::from_uci` alone does not
/// restore the correct `MoveKind` for castling/en passant, it is always
/// necessary to compare `from`/`to`/`promotion` against the position's actually
/// legal moves to find the applicable move.
///
/// Returns `None` if a move in the path does not match any legal move
/// (corrupted path — should never happen since every move comes from
/// an `OpeningMoveRow.uci` already validated by `opening_repo::next_moves`,
/// but stays defensive).
///
/// The tuple's 3rd element (ergonomics follow-up 10/07/2026 — the preview
/// board of the "Filtrer par ouverture" tab) is the origin/destination
/// pair of the LAST move of the path, `None` if `path` is empty
/// (starting position, no "last move" to highlight) — directly
/// reusable as the `last_move` argument of
/// `game_controller::build_static_squares`.
fn replay_opening_tree_path(path: &[String]) -> Option<OpeningTreePathReplay> {
    let mut pos = chess_core::types::Position::starting();
    let mut sans: Vec<String> = Vec::with_capacity(path.len());
    let mut last_move: Option<(chess_core::types::Square, chess_core::types::Square)> = None;
    for uci in path {
        let raw = chess_core::types::chess_move::Move::from_uci(uci)?;
        let mv = chess_core::movegen::generate_legal_moves(&pos)
            .into_iter()
            .find(|m| m.from == raw.from && m.to == raw.to && m.promotion == raw.promotion)?;
        sans.push(chess_core::notation::move_to_san(&pos, mv));
        last_move = Some((mv.from, mv.to));
        pos = chess_core::rules::make_move(&pos, mv).ok()?;
    }

    let mut display = String::new();
    for (i, san) in sans.iter().enumerate() {
        if i.is_multiple_of(2) {
            let _ = write!(display, "{}. ", i / 2 + 1);
        }
        display.push_str(san);
        display.push(' ');
    }
    Some((pos, display.trim_end().to_string(), last_move))
}

/// Re-reads `path` + the current Elo threshold (`tree-elo-min`), queries
/// `opening_repo::next_moves` from the reached position, and updates
/// `tree-path-display`/`tree-candidates`/`tree-total-games`/`tree-can-go-back`
/// on `win`. Used by the four `tree-*` callbacks (picking a move,
/// going back, resetting, changing the Elo threshold) — same query
/// every time, only the path or the threshold changes (PHASE 82, step 8).
///
/// `checked` (ergonomics follow-up 10/07/2026 — bridge to the "Parties" tab):
/// set of UCI moves currently checked in the table. Each rebuilt row
/// carries `checked = checked.contains(&uci)`, and
/// `tree-selected-count`/`tree-all-checked` are recomputed from this
/// set — never a simple local sum of checked rows, which would
/// easily get out of sync with the current Elo threshold: `checked.is_empty()`
/// means "everything selected" (default), otherwise a restricted
/// `opening_repo::games_for_path` query gives the exact
/// count. The caller decides whether to clear `checked` before calling this
/// function (path change = clear; Elo threshold change =
/// keep, see the callbacks further below).
fn refresh_opening_tree(win: &AppWindow, path: &[String], checked: &std::collections::HashSet<String>) {
    win.set_tree_can_go_back(!path.is_empty());

    let Some((pos, display, last_move)) = replay_opening_tree_path(path) else {
        // Corrupted path (should not happen, see the doc of
        // `replay_opening_tree_path`): displays an empty state rather
        // than an inconsistent one.
        win.set_tree_path_display("".into());
        win.set_tree_candidates(ModelRc::new(VecModel::from(Vec::<OpeningMoveRow>::new())));
        win.set_tree_total_games(0);
        win.set_tree_selected_count(0);
        win.set_tree_all_checked(false);
        win.set_tree_board_squares(ModelRc::new(VecModel::from(Vec::<gui::SquareData>::new())));
        win.set_tree_captured_by_white(ModelRc::new(VecModel::from(Vec::<gui::CapturedPieceData>::new())));
        win.set_tree_captured_by_black(ModelRc::new(VecModel::from(Vec::<gui::CapturedPieceData>::new())));
        win.set_tree_material_diff(0);
        return;
    };
    win.set_tree_path_display(display.into());

    // Ergonomics follow-up 10/07/2026 — preview board (user
    // feedback: "huge waste of space" from the table, freed-up space
    // reused to view the reached position). Independent of the
    // `opening_repo::next_moves` query below (which can fail without
    // preventing the board from being displayed): computed immediately after
    // replaying the path, before any DB connection.
    win.set_tree_board_squares(ModelRc::new(VecModel::from(game_controller::build_static_squares(&pos, last_move))));
    {
        let (captured_white, captured_black, diff) = game_controller::captured_summary_for_path(path);
        win.set_tree_captured_by_white(ModelRc::new(VecModel::from(captured_white)));
        win.set_tree_captured_by_black(ModelRc::new(VecModel::from(captured_black)));
        win.set_tree_material_diff(diff);
    }

    let min_elo = win.get_tree_elo_min().trim().parse::<i64>().ok();
    let hash = chess_core::polyglot::polyglot_hash(&pos);

    let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(win)) else {
        win.set_tree_candidates(ModelRc::new(VecModel::from(Vec::<OpeningMoveRow>::new())));
        win.set_tree_total_games(0);
        win.set_tree_selected_count(0);
        win.set_tree_all_checked(false);
        return;
    };

    let stats = db::repository::opening_repo::next_moves(&conn, hash, min_elo).unwrap_or_default();
    let total: i64 = stats.iter().map(|s| s.games).sum();
    let n_rows = stats.len();

    let rows: Vec<OpeningMoveRow> = stats
        .into_iter()
        .map(|s| {
            // The SAN and the hover arrow are obtained by resolving the
            // stored UCI move against the legal moves of the reached
            // position — same logic as `replay_opening_tree_path`.
            // `arrow_path` (ergonomics follow-up 10/07/2026): SVG
            // commands pre-computed once and for all here (same
            // `compute_hint_arrow` function as the engine's hint arrow),
            // displayed on the Slint side when hovering the row with no extra
            // Rust round trip (state purely reactive, local to
            // `OpeningTreeTab`, see `reference_browser.slint`).
            let resolved = chess_core::types::chess_move::Move::from_uci(&s.uci_move)
                .and_then(|raw| {
                    chess_core::movegen::generate_legal_moves(&pos)
                        .into_iter()
                        .find(|m| m.from == raw.from && m.to == raw.to && m.promotion == raw.promotion)
                });

            let san = resolved
                .map_or_else(|| s.uci_move.clone(), |mv| chess_core::notation::move_to_san(&pos, mv));

            let arrow_path = resolved.map_or_else(String::new, |mv| {
                let from_col = i32::from(mv.from.file());
                let from_row = 7 - i32::from(mv.from.rank());
                let to_col   = i32::from(mv.to.file());
                let to_row   = 7 - i32::from(mv.to.rank());
                compute_hint_arrow(from_row, from_col, to_row, to_col, false)
            });

            OpeningMoveRow {
                checked: checked.contains(&s.uci_move),
                san: san.into(),
                uci: s.uci_move.into(),
                games: i32::try_from(s.games).unwrap_or(i32::MAX),
                white_wins: i32::try_from(s.white_wins).unwrap_or(i32::MAX),
                draws: i32::try_from(s.draws).unwrap_or(i32::MAX),
                black_wins: i32::try_from(s.black_wins).unwrap_or(i32::MAX),
                arrow_path: arrow_path.into(),
            }
        })
        .collect();

    win.set_tree_candidates(ModelRc::new(VecModel::from(rows)));
    win.set_tree_total_games(i32::try_from(total).unwrap_or(i32::MAX));

    // Number of actually selected games: the total if nothing
    // is checked (default), otherwise a query restricted to the checked moves
    // (never a sum of the `games` of the checked rows on the Slint side — this
    // query remains the single source of truth, consistent with the
    // "IN" semantics combined with the Elo threshold used by `games_for_path`).
    let selected: i64 = if checked.is_empty() {
        total
    } else {
        let allowed: Vec<String> = checked.iter().cloned().collect();
        db::repository::opening_repo::games_for_path(&conn, hash, Some(&allowed), min_elo)
            .map_or(0, |ids| i64::try_from(ids.len()).unwrap_or(i64::MAX))
    };
    win.set_tree_selected_count(i32::try_from(selected).unwrap_or(i32::MAX));
    win.set_tree_all_checked(n_rows > 0 && checked.len() == n_rows);
}

/// Re-reads the classic filters (`reference-browser-filter-*`) and the
/// current page from `win`, runs the corresponding `SQL` search, and
/// fills `reference-browser-games`/`reference-browser-total-count`.
///
/// `game_ids` (ergonomics follow-up 10/07/2026 — bridge from the opening tree
/// to the game list): optional extra restriction, combined (logical AND,
/// decision made with the user) with the classic filters —
/// `None` exactly reproduces the old behavior (before this
/// filter was added). Extracted from the old body of `on_search_reference_games`
/// so it can also be called from `on_tree_list_games`/`on_tree_filter_clear`
/// without duplicating the `GameFilter` construction/row mapping.
fn run_reference_search(win: &AppWindow, game_ids: Option<&[i64]>) {
    // Must stay identical to `page-size` in `reference_browser.slint`
    // ("Page X / Y" computation on the GUI side) — two fixed constants, no
    // resynchronization needed between them.
    const PAGE_SIZE: i64 = 50;

    // Empty fields = filter not applied (`None`); a non-numeric Elo
    // range is silently ignored rather than blocking the
    // search — consistent with the choice not to validate input
    // live in `reference_browser.slint`.
    let player = win.get_reference_browser_filter_player().to_string();
    let player = (!player.trim().is_empty()).then_some(player);

    let min_elo = win.get_reference_browser_filter_elo_min().trim().parse::<i64>().ok();
    let max_elo = win.get_reference_browser_filter_elo_max().trim().parse::<i64>().ok();

    let date_from = win.get_reference_browser_filter_date_from().to_string();
    let date_from = (!date_from.trim().is_empty()).then_some(date_from);
    let date_to = win.get_reference_browser_filter_date_to().to_string();
    let date_to = (!date_to.trim().is_empty()).then_some(date_to);

    let eco = win.get_reference_browser_filter_eco().to_string();
    let eco = (!eco.trim().is_empty()).then_some(eco);

    let filter = db::repository::reference_game_repo::GameFilter {
        player: player.as_deref(),
        min_elo,
        max_elo,
        date_from: date_from.as_deref(),
        date_to: date_to.as_deref(),
        eco: eco.as_deref(),
        game_ids,
    };

    let page = i64::from(win.get_reference_browser_page().max(0));

    let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(win)) else {
        win.set_reference_browser_games(ModelRc::new(VecModel::from(Vec::<ReferenceGameListRow>::new())));
        win.set_reference_browser_total_count(0);
        return;
    };

    let total = db::repository::reference_game_repo::count_matching(&conn, &filter).unwrap_or(0);
    let rows = db::repository::reference_game_repo::search(&conn, &filter, PAGE_SIZE, page * PAGE_SIZE)
        .unwrap_or_default();

    let ui_rows: Vec<ReferenceGameListRow> = rows.into_iter().map(|r| {
        // Opening name resolved from the ECO code (PHASE 82, step 11) —
        // "" if no ECO code is set for this game (behavior
        // identical to the old `eco`-only field, no regression).
        let opening = r.eco.as_deref()
            .and_then(db::eco_names::opening_name)
            .unwrap_or_default()
            .to_string();
        ReferenceGameListRow {
            id:        i32::try_from(r.id).unwrap_or(0),
            white:     r.white.into(),
            black:     r.black.into(),
            result:    r.result.into(),
            date:      r.date.unwrap_or_else(|| "?".to_string()).into(),
            eco:       r.eco.unwrap_or_default().into(),
            opening:   opening.into(),
            white_elo: r.white_elo.map_or_else(|| "—".to_string(), |e| e.to_string()).into(),
            black_elo: r.black_elo.map_or_else(|| "—".to_string(), |e| e.to_string()).into(),
        }
    }).collect();

    win.set_reference_browser_games(ModelRc::new(VecModel::from(ui_rows)));
    win.set_reference_browser_total_count(i32::try_from(total).unwrap_or(i32::MAX));
}

/// Resolves the path of the analysis engine already configured by the user —
/// same priority order as `sync_analysis_engine` (hint engine, then
/// first saved engine, then none), but read directly from the
/// properties already exposed on `AppWindow` rather than via
/// `DualAnalysisBridge` (state internal to the current game's analysis,
/// unrelated to on-demand analysis of a reference-database
/// game, PHASE 82, step 9 — decision made: reuse the engine already
/// configured, not a separate setting).
///
/// # Ergonomics bugfix 09/07/2026
///
/// `win.get_hint_engine_path()` and the `path`s of `win.get_saved_engines()`
/// are **intentionally relative** on the Slint side (conversion done by
/// `hint_engine_path_for_window`/`update_engines_in_window`, only for
/// display and identity comparisons in the Preferences UI — see
/// their docs). Returning them as-is here passed them directly
/// to `UciEngine::connect_with_timeout`, which needs a path that is actually
/// launchable: the connection silently failed depending on the process's current
/// directory, producing the same "No engine configured" message as
/// an actual absence of engine — even though a hint engine was
/// selected. User feedback: "it tells me no engine is
/// configured even though the hint engine is selected and an engine is
/// present". Fixed by passing each path through
/// `app_paths::to_absolute_path` before returning it — the same resolution
/// already done at the very last moment in `GameBridge::init` for
/// an interactive game.
fn resolve_analysis_engine_path(win: &AppWindow) -> Option<String> {
    let hint = win.get_hint_engine_path().to_string();
    if !hint.trim().is_empty() {
        return Some(app_paths::to_absolute_path(&hint).to_string_lossy().into_owned());
    }
    win.get_saved_engines()
        .iter()
        .next()
        .map(|e| app_paths::to_absolute_path(&e.path).to_string_lossy().into_owned())
}

/// Loads the game `game_id` from the reference database and prepares
/// the display of the detail screen (header + moves text). Does NOT
/// start any analysis — that remains "on demand" (see
/// `on_analyze_game_detail` in `main()`), never automatic on
/// opening (decision made, PHASE 82, point 6).
fn build_game_detail(win: &AppWindow, game_id: i32) {
    let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(win)) else { return; };
    let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
    else {
        return;
    };
    let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };

    // One row per move pair (White/Black), with the 1-based index of
    // each move in the full game (PHASE 82, step 10 — "replay
    // from a move"): this index is exactly the one expected by
    // `core::game::GameState::position_at` (`index` = position AFTER the
    // `index`-th move), passed through as-is by `ply-selected` to
    // find the exact position without duplicating the numbering logic
    // on the Rust side and the Slint side.
    let records = game.history().records();
    // Comments (read-only): attached to `GameTree` nodes, not
    // to `MoveRecord` — retrieved via `node_id_at` (same indexing as
    // `records`, active line only) then `tree.node(id).comment`.
    // Classic PGN import (§ `chess_core::pgn::import_pgn`) or SCID
    // decoding (V2 Phase B, 12/07/2026) feed this field identically.
    let tree = game.history().tree();
    let move_rows: Vec<GameDetailMoveRow> = records
        .chunks(2)
        .enumerate()
        .map(|(pair_idx, chunk)| {
            let white_ply = i32::try_from(pair_idx * 2 + 1).unwrap_or(i32::MAX);
            let black_ply = if chunk.len() == 2 {
                i32::try_from(pair_idx * 2 + 2).unwrap_or(i32::MAX)
            } else {
                -1
            };
            let white_comment = game.history().node_id_at(pair_idx * 2)
                .and_then(|id| tree.node(id))
                .and_then(|n| n.comment.clone())
                .unwrap_or_default();
            let black_comment = if chunk.len() == 2 {
                game.history().node_id_at(pair_idx * 2 + 1)
                    .and_then(|id| tree.node(id))
                    .and_then(|n| n.comment.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            GameDetailMoveRow {
                number_str: format!("{}.", pair_idx + 1).into(),
                white_san: chunk[0].san.clone().into(),
                black_san: chunk.get(1).map_or_else(String::new, |r| r.san.clone()).into(),
                white_ply,
                black_ply,
                // Ergonomics follow-up 10/07/2026: same function as the main
                // history panel (`GameController::move_kind_code`),
                // for guaranteed identical syntax highlighting.
                white_move_kind: GameController::move_kind_code(&chunk[0]),
                black_move_kind: chunk.get(1).map_or(0, GameController::move_kind_code),
                white_comment: white_comment.into(),
                black_comment: black_comment.into(),
            }
        })
        .collect();

    let white_elo = row.white_elo.map_or_else(|| "—".to_string(), |e| e.to_string());
    let black_elo = row.black_elo.map_or_else(|| "—".to_string(), |e| e.to_string());
    // Opening name resolved from the ECO code (PHASE 82, step 11) — absent
    // from the header if no ECO code is set for this game.
    let opening_suffix = row.eco.as_deref()
        .and_then(db::eco_names::opening_name)
        .map(|name| format!(" — {name}"))
        .unwrap_or_default();
    let header = format!(
        "{} ({}) vs {} ({}) — {} — {}{}",
        row.white,
        white_elo,
        row.black,
        black_elo,
        row.result,
        row.date.unwrap_or_else(|| "?".to_string()),
        opening_suffix,
    );

    // Preview board (ergonomics bugfix 09/07/2026): initialized on the
    // END-of-game position (no selection — see the doc of
    // `GameDetailView`), `current_ply` set to the total half-move count.
    let total_plies = i32::try_from(records.len()).unwrap_or(i32::MAX);
    let final_last_move = records.last().map(|r| (r.mv.from, r.mv.to));
    let board_squares = game_controller::build_static_squares(game.position(), final_last_move);

    win.set_game_detail_id(game_id);
    win.set_game_detail_header(header.into());
    win.set_game_detail_moves(ModelRc::new(VecModel::from(move_rows)));
    win.set_game_detail_scores(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
    win.set_game_detail_white_curve(String::new().into());
    win.set_game_detail_black_curve(String::new().into());
    win.set_game_detail_can_deepen(false);
    win.set_game_detail_analysis_active(false);
    win.set_game_detail_analysis_status(String::new().into());
    win.set_game_detail_total_plies(total_plies);
    win.set_game_detail_current_ply(total_plies);
    win.set_game_detail_board_squares(ModelRc::new(VecModel::from(board_squares)));
    win.set_show_game_detail(true);
}

/// Generic "unexpected error" dialog shown by [`gui::run_guarded_thread`]'s
/// `on_panic` callbacks — the operation was aborted by an internal bug
/// (a panic) rather than a normal, already-translated `Err` path, so
/// there is no specific error message to relay to the user.
fn show_unexpected_error_dialog() {
    rfd::MessageDialog::new()
        .set_title(i18n::translate("dialog.unexpected_error_title"))
        .set_description(i18n::translate("dialog.unexpected_error_desc"))
        .set_level(rfd::MessageLevel::Error)
        .show();
}

// ── Entry point ────────────────────────────────────────────────────────────────

// Clippy (04/07/2026): `main()` assembles and wires up the entire set of Slint
// callbacks (board, clocks, engines, tournaments, puzzles, position editor,
// import/export...) — an orchestration function that stays more readable as
// a single sequential block than split into dozens of sub-functions
// that would have to capture the same shared variables anyway
// (Rc<RefCell<...>>) by closure. A larger refactor deemed out of scope for this
// clippy pass (confirmed with the user).
#[allow(clippy::too_many_lines)]
fn main() -> Result<(), slint::PlatformError> {
    // 0. Portable directory tree (PHASE 24, Step 1) — recreates the
    //    subfolders of VendettaChess/ (parametres/, parametres/parties/, base/, moteurs/,
    //    ouvertures/, logs/) if missing. Non-blocking: a failure here
    //    (e.g. a read-only delivery folder) must not prevent
    //    startup — the later PHASE 24 steps that will actually
    //    depend on these folders will then handle their own errors.
    if let Err(e) = app_paths::ensure_app_dirs() {
        eprintln!("Avertissement : impossible de créer l'arborescence portable ({e})");
    }

    // 1. Translations — load the saved language or use English by
    //    default (`Lang::default()` — change from 05/07/2026, explicit
    //    user request; previously Fr).
    let saved_lang_code = prefs::load_lang();
    let initial_lang = saved_lang_code.as_deref()
        .map_or_else(Lang::default, parse_lang_code);

    let lang_cell: Rc<RefCell<Lang>> = Rc::new(RefCell::new(initial_lang));
    i18n::init(initial_lang);

    // 2. Slint window
    let window = AppWindow::new()?;
    window.window().set_size(slint::WindowSize::Logical(slint::LogicalSize { width: 1200.0, height: 800.0 }));
    // Official software version: single source = `version` in the
    // workspace's Cargo.toml (inherited by crates/gui/Cargo.toml), read by the
    // compiler via the standard `env!` macro — never duplicated/hard-coded
    // on the Slint side (see the "About" window).
    window.set_app_version_number(env!("CARGO_PKG_VERSION").into());
    i18n_bridge::apply_translations(&window.global::<Tr>(), initial_lang);
    // Language dropdown in Preferences (ergonomics bugfix from
    // 03/07/2026): tell it the language already active at startup.
    window.set_current_lang_index(initial_lang.ui_index());

    // 2bis. Intercept the native close (X button) to ask for confirmation.
    let window_weak_close = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(win) = window_weak_close.upgrade() {
            win.set_show_close_confirm(true);
        }
        slint::CloseRequestResponse::KeepWindowShown
    });

    // 3. Game controller
    let controller = Rc::new(RefCell::new(GameController::new()));

    // 4. Engine analysis bridge (two independent bridges: white + black)
    let analysis = Rc::new(RefCell::new(DualAnalysisBridge::new()));

    // 4b. Engine-player bridge (H vs M / M vs M)
    let game_bridge = Rc::new(RefCell::new(GameBridge::new()));

    // 4c. List of remembered UCI engines (prefs) — loaded once at startup.
    //     Invalid paths are automatically filtered out by load_engines().
    let engine_list: Rc<RefCell<Vec<prefs::SavedEngine>>> =
        Rc::new(RefCell::new(prefs::load_engines()));

    // 4d. Hint engine: persisted path (None = none).
    let hint_engine_path: Rc<RefCell<Option<String>>> =
        Rc::new(RefCell::new(prefs::load_hint_engine()));

    // 4e. Polyglot opening books: loaded into memory once at
    // startup from their fixed location (PHASE 24, Step 6 —
    // `ouvertures/blancs.bin`/`noirs.bin`, no more separate persisted path:
    // the presence of a book is deduced directly from the
    // file's existence). `None` if the file is missing or became invalid in
    // the meantime — in that case the game continues normally (see `load_runtime_book`).
    let book_white: Rc<RefCell<Option<PolyglotBook>>> =
        Rc::new(RefCell::new(load_runtime_book(book_path_if_exists(&app_paths::book_blancs_path()))));
    let book_black: Rc<RefCell<Option<PolyglotBook>>> =
        Rc::new(RefCell::new(load_runtime_book(book_path_if_exists(&app_paths::book_noirs_path()))));

    // 4f. Active puzzle session (PHASE 14, Step 5): `None` as long as no
    // puzzle has been launched from the wizard. The full wiring of
    // solving (comparing moves, see solution/next/
    // quit buttons) is the subject of Step 6 — at this stage the session
    // only serves to display the position to solve (the opponent's
    // setup move having already been played by `PuzzleSession::new`).
    let puzzle_session: Rc<RefCell<Option<PuzzleSession>>> = Rc::new(RefCell::new(None));

    // 4d-bis. Synchronize the analysis engine (Phase 11.3 evaluation bar).
    // Priority: hint → first saved → auto-discovered.
    {
        let engine_name = sync_analysis_engine(
            &mut analysis.borrow_mut(),
            hint_engine_path.borrow().as_ref(),
            &engine_list.borrow(),
        );
        window.set_analysis_engine_name(engine_name.into());
    }

    // 4d-ter. Automatic scan of moteurs/ at startup (PHASE 24, Step 7).
    //
    //   Detects files in `moteurs/` absent from the saved list
    //   (including one level of subfolder — see `gui::engine_scan`), validates
    //   them via a silent UCI handshake (no dialog, no trace
    //   on failure: a `moteurs/` folder can legitimately contain
    //   non-executable companion files, e.g. NNUE weights, DLLs), then
    //   adds them under their UCI `id name` (falls back to the file name).
    //
    //   Runs entirely on a background thread so as to never
    //   delay the window's display: only owned data
    //   (`Vec<String>`, `PathBuf`) crosses the thread boundary — never
    //   `engine_list` itself (`Rc<RefCell<…>>` is not `Send`). The
    //   result comes back via an `mpsc` channel, polled by a Slint `Timer`
    //   (same principle as `clock_timer`/`startup_timer` further below): it is this
    //   `Timer`, running on the UI thread, that touches `engine_list` and the
    //   window. Once the message is received (or never, if no new
    //   file), the channel is closed and subsequent polls do nothing
    //   more — no explicit stopping of the `Timer` (a known Slint bug makes
    //   `Timer::stop()` unreliable when called from its own
    //   callback).
    //
    //   IMPORTANT: `scan_timer` is declared at the `main()` level (like
    //   `clock_timer`/`startup_timer` further below), never in a nested `{ }`
    //   block — its scope must cover all of `main()` up to
    //   `window.run()`, otherwise it would be destroyed (and disabled) as soon as
    //   the block that created it ends.
    let engine_scan_known_paths: Vec<String> =
        engine_list.borrow().iter().map(|e| e.path.clone()).collect();
    let engine_scan_moteurs_dir = app_paths::moteurs_dir();
    let (engine_scan_tx, engine_scan_rx) =
        std::sync::mpsc::channel::<Vec<prefs::SavedEngine>>();

    std::thread::spawn(move || {
        let candidates = engine_scan::list_scan_candidates(
            &engine_scan_moteurs_dir,
            &engine_scan_known_paths,
        );
        let found = engine_scan::validate_candidates(candidates);
        let _ = engine_scan_tx.send(found);
    });

    let engine_list_scan  = engine_list.clone();
    let window_weak_scan  = window.as_weak();
    let analysis_for_scan = analysis.clone();
    let hint_path_scan    = hint_engine_path.clone();

    let scan_timer = slint::Timer::default();
    scan_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(300),
        move || {
            let Ok(found) = engine_scan_rx.try_recv() else { return; };
            if found.is_empty() { return; }

            // PHASE 81 — user request: an engine manually dropped
            // into the moteurs/ folder (without going through the "Add" button)
            // and detected by this automatic scan at startup must benefit
            // from the same rule as an addition via the software (PHASE 80): if the
            // list was empty and no hint engine is currently
            // defined, the first newly detected engine becomes the
            // hint engine (persisted). If several engines are detected
            // in a single pass while the list was empty, only the
            // first one (order of `engine_scan::list_scan_candidates`) is
            // kept — consistent with the "first in the list" convention
            // already used by `sync_analysis_engine`.
            let mut auto_hint_path: Option<String> = None;
            {
                let mut engines = engine_list_scan.borrow_mut();
                let was_empty = engines.is_empty();
                let had_no_hint = hint_path_scan.borrow().is_none();
                // PHASE 74 — comparison via `Path` (safeguard redundant with
                // the filtering already done in `engine_scan::list_scan_candidates`,
                // but against the same class of potential bug: `engine.path`
                // comes from `std::fs::read_dir`, native separators, while
                // `e.path` may contain a mixed separator after
                // resolving a multi-component relative path).
                for engine in found {
                    if !engines.iter().any(|e| std::path::Path::new(&e.path) == std::path::Path::new(&engine.path)) {
                        if was_empty && had_no_hint && auto_hint_path.is_none() {
                            auto_hint_path = Some(engine.path.clone());
                        }
                        engines.push(engine);
                    }
                }
                prefs::save_engines(&engines);
            }
            if let Some(path) = auto_hint_path {
                prefs::save_hint_engine(Some(path.as_str()));
                *hint_path_scan.borrow_mut() = Some(path);
            }
            let engine_name = sync_analysis_engine(
                &mut analysis_for_scan.borrow_mut(),
                hint_path_scan.borrow().as_ref(),
                &engine_list_scan.borrow(),
            );
            if let Some(win) = window_weak_scan.upgrade() {
                update_engines_in_window(&win, &engine_list_scan.borrow());
                win.set_analysis_engine_name(engine_name.into());
                win.set_hint_engine_path(
                    hint_engine_path_for_window(hint_path_scan.borrow().as_deref()).into()
                );
                win.set_hint_engine_name(
                    hint_engine_display_name(hint_path_scan.borrow().as_deref(), &engine_list_scan.borrow()).into()
                );
            }
        },
    );

    // 5. Score history
    let score_history: Rc<RefCell<Vec<f32>>> = Rc::new(RefCell::new(Vec::new()));

    // 5a-bis. Current opening-tree path (PHASE 82, step 8) —
    // UCI moves chosen from the standard starting position, shared
    // between the `tree-*` callbacks wired further below. Empty = starting
    // position (initial state and after `tree-reset`).
    let opening_tree_path: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    // 5a-ter. Ergonomics follow-up 10/07/2026 — bridge from the opening tree
    // to the game list (user feedback: the tree was a dead end).
    // `tree_checked_moves`: UCI moves checked in the current table, cleared
    // on every PATH change (new move chosen, back,
    // reset) but kept on a simple Elo threshold
    // change (the checked boxes stay relevant, only the displayed
    // statistics change). `tree_applied_game_ids`: filter actually
    // applied to the "Parties" tab once "Lister les parties" is clicked —
    // `None` = no active filter, distinct from `tree_checked_moves` which
    // only describes the (temporary, not yet applied) state of the tree's
    // table.
    let tree_checked_moves: Rc<RefCell<std::collections::HashSet<String>>> =
        Rc::new(RefCell::new(std::collections::HashSet::new()));
    let tree_applied_game_ids: Rc<RefCell<Option<Vec<i64>>>> = Rc::new(RefCell::new(None));

    // 5b. Chess clock (pure logic, ticks every 100 ms via the Slint timer)
    let chess_clock: Rc<RefCell<ChessClock>> =
        Rc::new(RefCell::new(ChessClock::new(&TimeControl::Infinite)));

    // 6. Initial display
    {
        let ctrl = controller.borrow();
        window.set_squares(ModelRc::new(VecModel::from(ctrl.build_squares())));
        window.set_moves(ModelRc::new(VecModel::from(ctrl.build_move_rows())));
        window.set_viewed_ply(ctrl.viewed_ply_slint());
        window.set_status_text(i18n::translate(ctrl.status_key()).into());
        window.set_is_white_turn(ctrl.is_white_turn());
    }

    // 6b. Inject the engine list into the window
    update_engines_in_window(&window, &engine_list.borrow());

    // 6c. Inject the hint engine's path ("" if none)
    {
        let path = hint_engine_path.borrow();
        window.set_hint_engine_path(hint_engine_path_for_window(path.as_deref()).into());
        window.set_hint_engine_name(
            hint_engine_display_name(path.as_deref(), &engine_list.borrow()).into()
        );
    }

    // 6c-bis. Inject the name of the already-configured Polyglot books ("" if none),
    // based on the simple presence of ouvertures/blancs.bin/noirs.bin
    // (PHASE 24, Step 6 — fixed names, no more separate persisted path).
    if app_paths::book_blancs_path().exists() {
        window.set_book_name_white("blancs.bin".into());
    }
    if app_paths::book_noirs_path().exists() {
        window.set_book_name_black("noirs.bin".into());
    }

    // 6d. Load the persisted Multi-PV choices (default = 0 for each side)
    window.set_analysis_multipv_white(prefs::load_multipv_white());
    window.set_analysis_multipv_black(prefs::load_multipv_black());

    // 6e. Number of puzzles already in the database (PHASE 14, Step 3) and
    // global solving statistics (PHASE 14, Step 8 — displayed only in
    // Preferences, then updated live after each attempt via
    // `record_puzzle_stats`). Silent failure (DB unreachable): the
    // counter and stats stay at their default values, non-blocking
    // for the rest of the application — consistent with the handling of
    // other DB errors.
    if let Ok(conn) = db::schema::open_and_migrate(&tournament_db_path()) {
        if let Ok(n) = db::repository::puzzle_repo::count(&conn) {
            window.set_puzzle_count(i32::try_from(n).unwrap_or(i32::MAX));
        }
        if let Ok(stats) = db::repository::puzzle_repo::global_stats(&conn) {
            push_puzzle_stats(&window, stats);
        }
    }

    // 6e-bis. Number of games already in the reference database (PHASE 82), PGN and
    // SCID separately (two distinct files since 11/07/2026, see
    // `app_paths::reference_pgn_db_path`/`reference_scid_db_path`) — each
    // if it does not exist yet (no import done), `open_and_migrate`
    // creates it empty, so the count stays at 0 with no error. Silent failure
    // (DB unreachable), same handling as the puzzles above.
    if let Ok(conn) = db::reference_schema::open_and_migrate(&reference_pgn_db_path()) {
        if let Ok(n) = db::repository::reference_game_repo::count_matching(
            &conn,
            &db::repository::reference_game_repo::GameFilter::default(),
        ) {
            window.set_reference_pgn_game_count(i32::try_from(n).unwrap_or(i32::MAX));
        }
    }
    if let Ok(conn) = db::reference_schema::open_and_migrate(&reference_scid_db_path()) {
        if let Ok(n) = db::repository::reference_game_repo::count_matching(
            &conn,
            &db::repository::reference_game_repo::GameFilter::default(),
        ) {
            window.set_reference_scid_game_count(i32::try_from(n).unwrap_or(i32::MAX));
        }
    }

    // 6f. Remembered Puzzle-mode choices (PHASE 14, Step 5): pre-filling
    // the wizard from the prefs. These properties are held by
    // `AppWindow` (not destroyed/recreated with the wizard), so this
    // startup pre-fill is enough for the whole application session —
    // only an explicit save ("Démarrer" click) changes them.
    window.set_puzzle_hint_theme(prefs::load_puzzle_hint_theme());
    window.set_puzzle_hint_button(prefs::load_puzzle_hint_button());

    // 6g. Debug mode (PHASE 26sexies, Preferences → Misc): persisted
    // preference applied immediately at startup — never enabled by
    // default (user feedback: never risk shipping the software
    // stuck in debug mode).
    let debug_mode_enabled = prefs::load_debug_mode_enabled();
    window.set_debug_mode_enabled(debug_mode_enabled);
    gui::debug_log::set_debug_enabled(debug_mode_enabled);

    {
        let window_weak_dbg = window.as_weak();
        window.on_toggle_debug_mode(move |enabled| {
            prefs::save_debug_mode_enabled(enabled);
            gui::debug_log::set_debug_enabled(enabled);
            if let Some(win) = window_weak_dbg.upgrade() {
                win.set_debug_mode_enabled(enabled);
            }
        });
    }

    // 7. Callback: click on a square → model update + analysis / engine
    let window_weak_board     = window.as_weak();
    let controller_for_board  = controller.clone();
    let analysis_for_board    = analysis.clone();
    let game_bridge_for_board = game_bridge.clone();
    let lang_for_board        = lang_cell.clone();
    let chess_clock_board     = chess_clock.clone();
    let book_white_board      = book_white.clone();
    let book_black_board      = book_black.clone();
    let puzzle_session_board  = puzzle_session.clone();

    window.on_square_clicked(move |row, col| {
        if let Some(win) = window_weak_board.upgrade() {
            gui::debug_log::log_event("on_square_clicked", &serde_json::json!({
                "row": row,
                "col": col,
                "game_mode": win.get_current_game_mode(),
                "engine_playing": win.get_engine_playing(),
                "game_paused": win.get_game_paused(),
                "viewed_ply": win.get_viewed_ply(),
                "variation_editing": win.get_variation_editing(),
            }));
        }
        // Guard: a click on the board must never be able to play in
        // place of an engine. Blocks ONLY clicks on the squares — the
        // other controls (buttons, Multi-PV, preferences…) have their
        // own callbacks and are not affected by this guard.
        if let Some(win) = window_weak_board.upgrade() {
            // M vs M: neither side is human, no click on
            // the board should have any effect.
            if win.get_current_game_mode() == 2 {
                return;
            }
            // H vs M: the engine is currently thinking → a click during
            // that time must not be able to move its pieces (bug reported
            // on 02/07/2026: possible by clicking faster than the engine).
            if win.get_engine_playing() {
                gui::debug_log::log_event("on_square_clicked_blocked_engine_playing", &serde_json::json!({}));
                return;
            }
            // Puzzle (PHASE 14, Step 6): no more moves accepted once the
            // sequence is finished (solved, revealed or interrupted) — the user
            // must go through "Next puzzle"/"Quit" rather than continuing
            // to play on an already-solved position.
            if win.get_current_game_mode() == 4
                && puzzle_session_board.borrow().as_ref().is_none_or(PuzzleSession::is_finished)
            {
                return;
            }
        }

        // PHASE 16, Step 5, decision 3 — update PHASE 26, Step 3: variation
        // creation now depends solely on
        // `is_variation_editing()`, explicitly toggled by the
        // "Create a variation"/"End the variation" buttons of the banner
        // (`on_enter_variation_editing`/`on_exit_variation_editing`
        // below), no more implicit computation here.
        let mut ctrl     = controller_for_board.borrow_mut();
        let moves_before = ctrl.move_count();
        let lang         = *lang_for_board.borrow();

        if ctrl.on_click(row, col) {
            if let Some(win) = window_weak_board.upgrade() {

                // Promotion pending → pause the clock + show the modal
                if ctrl.has_pending_promotion() {
                    chess_clock_board.borrow_mut().stop();
                    win.set_white_clock_active(false);
                    win.set_black_clock_active(false);
                    win.set_show_promotion_modal(true);
                    win.set_promotion_is_white(ctrl.pending_promo_is_white());
                    win.set_squares(ModelRc::new(VecModel::from(ctrl.build_squares())));
                    return;
                }

                refresh_game_state(&win, &ctrl, lang);

                // After a move: advance the clock + trigger engine + analysis
                if ctrl.move_count() > moves_before {
                    // Puzzle mode (PHASE 14, Step 6): validation against
                    // PuzzleSession rather than the normal engine/book/clock
                    // logic below (no engine nor clock in Puzzle
                    // mode — see `load_random_puzzle`).
                    if win.get_current_game_mode() == 4 {
                        drop(ctrl);
                        handle_puzzle_move(&win, &controller_for_board, &puzzle_session_board, &analysis_for_board, lang);
                        return;
                    }

                    let just_moved_white = !ctrl.is_white_turn();

                    if ctrl.is_over() {
                        chess_clock_board.borrow_mut().stop();
                        win.set_white_clock_active(false);
                        win.set_black_clock_active(false);
                    } else {
                        chess_clock_board.borrow_mut().apply_move_bonus(just_moved_white);

                        // Release the `ctrl` borrow before `try_play_book_moves`
                        // (which internally re-borrows `controller_for_board`).
                        drop(ctrl);

                        // PHASE 15: does the side to move (and the following
                        // ones, if both sides have a book) have a book move for
                        // the current position? Played directly if so, without
                        // consulting the engine.
                        if try_play_book_moves(&win, &controller_for_board, &chess_clock_board, &game_bridge_for_board, &book_white_board, &book_black_board) {
                            refresh_game_state(&win, &controller_for_board.borrow(), lang);
                        }

                        let (next_is_white, is_over, fen) = {
                            let ctrl = controller_for_board.borrow();
                            (ctrl.is_white_turn(), ctrl.is_over(), ctrl.current_fen())
                        };

                        if is_over {
                            chess_clock_board.borrow_mut().stop();
                            win.set_white_clock_active(false);
                            win.set_black_clock_active(false);
                        } else {
                            chess_clock_board.borrow_mut().start(next_is_white);
                            win.set_white_clock_active(next_is_white);
                            win.set_black_clock_active(!next_is_white);

                            let limits_opt = build_go_limits(&chess_clock_board.borrow());
                            // Set engine_playing synchronously (without
                            // waiting for the engine thread to do it after
                            // receiving the FEN): closes the micro-window between "the
                            // human move was just played" and "the engine
                            // thread has actually started thinking", during
                            // which a click could have slipped past the guard
                            // above.
                            //
                            // PHASE 26, Step 2: never trigger the engine
                            // during variation editing — this is exactly the
                            // path taken after the first move played in a
                            // variation (`viewed_ply` just went back to `None`,
                            // but `is_variation_editing()` stays true).
                            let engine_triggered = !controller_for_board.borrow().is_variation_editing()
                                && game_bridge_for_board.borrow()
                                    .trigger_if_engine_turn(next_is_white, fen.clone(), limits_opt);
                            if engine_triggered {
                                win.set_engine_playing(true);
                            }
                            let mpv_n = if next_is_white { win.get_analysis_multipv_white() } else { win.get_analysis_multipv_black() };
                            if mpv_n > 0 {
                                analysis_for_board.borrow_mut().start_for(next_is_white, fen, win.as_weak(), next_is_white, mpv_n as u32);
                            }
                        }
                    }
                }
            }
        }
    });

    // 8. Callback: promotion chosen
    let window_weak_promo     = window.as_weak();
    let controller_for_promo  = controller.clone();
    let analysis_for_promo    = analysis.clone();
    let game_bridge_for_promo = game_bridge.clone();
    let lang_for_promo        = lang_cell.clone();
    let chess_clock_promo     = chess_clock.clone();
    let book_white_promo      = book_white.clone();
    let book_black_promo      = book_black.clone();
    let puzzle_session_promo  = puzzle_session.clone();

    window.on_promote_chosen(move |piece_code| {
        // Same guard as on the board click (defense in depth):
        // the promotion modal is only supposed to open after a human click
        // already validated by on_square_clicked's guard, but it is re-checked
        // here to never depend solely on the state at the
        // time of the first click.
        if let Some(win) = window_weak_promo.upgrade() {
            if win.get_current_game_mode() == 2 || win.get_engine_playing() {
                return;
            }
            if win.get_current_game_mode() == 4
                && puzzle_session_promo.borrow().as_ref().is_none_or(PuzzleSession::is_finished)
            {
                return;
            }
        }

        let mut ctrl = controller_for_promo.borrow_mut();
        let lang     = *lang_for_promo.borrow();

        if ctrl.complete_promotion(piece_code) {
            if let Some(win) = window_weak_promo.upgrade() {
                win.set_show_promotion_modal(false);
                refresh_game_state(&win, &ctrl, lang);

                // Puzzle mode (PHASE 14, Step 6): a promotion can be the
                // human move expected by the sequence — same routing as
                // in on_square_clicked, before any engine/clock logic.
                if win.get_current_game_mode() == 4 {
                    drop(ctrl);
                    handle_puzzle_move(&win, &controller_for_promo, &puzzle_session_promo, &analysis_for_promo, lang);
                    return;
                }

                let just_moved_white = !ctrl.is_white_turn();

                if ctrl.is_over() {
                    chess_clock_promo.borrow_mut().stop();
                    win.set_white_clock_active(false);
                    win.set_black_clock_active(false);
                } else {
                    chess_clock_promo.borrow_mut().apply_move_bonus(just_moved_white);

                    drop(ctrl); // release the borrow before try_play_book_moves

                    // PHASE 15: a book move for the side to move?
                    if try_play_book_moves(&win, &controller_for_promo, &chess_clock_promo, &game_bridge_for_promo, &book_white_promo, &book_black_promo) {
                        refresh_game_state(&win, &controller_for_promo.borrow(), lang);
                    }

                    let (is_white_to_move, is_over, fen) = {
                        let ctrl = controller_for_promo.borrow();
                        (ctrl.is_white_turn(), ctrl.is_over(), ctrl.current_fen())
                    };

                    if is_over {
                        chess_clock_promo.borrow_mut().stop();
                        win.set_white_clock_active(false);
                        win.set_black_clock_active(false);
                    } else {
                        chess_clock_promo.borrow_mut().start(is_white_to_move);
                        win.set_white_clock_active(is_white_to_move);
                        win.set_black_clock_active(!is_white_to_move);

                        let limits_opt       = build_go_limits(&chess_clock_promo.borrow());
                        // PHASE 26, Step 2: same guard as in on_square_clicked.
                        let engine_triggered = !controller_for_promo.borrow().is_variation_editing()
                            && game_bridge_for_promo.borrow()
                                .trigger_if_engine_turn(is_white_to_move, fen.clone(), limits_opt);
                        if engine_triggered {
                            win.set_engine_playing(true);
                        }
                        let mpv_n = if is_white_to_move { win.get_analysis_multipv_white() } else { win.get_analysis_multipv_black() };
                        if mpv_n > 0 {
                            analysis_for_promo.borrow_mut().start_for(is_white_to_move, fen, win.as_weak(), is_white_to_move, mpv_n as u32);
                        }
                    }
                }
            }
        }
    });

    // 9. Callback: click on a move in the list → history navigation
    let window_weak_moves    = window.as_weak();
    let controller_for_moves = controller.clone();

    window.on_move_clicked(move |ply| {
        let mut ctrl = controller_for_moves.borrow_mut();
        if ctrl.go_to_ply(ply) {
            if let Some(win) = window_weak_moves.upgrade() {
                win.set_squares(ModelRc::new(VecModel::from(ctrl.build_squares())));
                win.set_viewed_ply(ctrl.viewed_ply_slint());
                push_captured_pieces(&win, &ctrl);
            }
        }
    });

    // 9bis. Callback: NAG chosen in the context menu (right-click on a
    // history move, including the starting move of a collapsed
    // variation) — PHASE 16, Step 6.1. The targeted node is read from
    // `context-menu-node-id`, still valid at this stage (reset to -1 by
    // the .slint right after this callback is called).
    let window_weak_nag    = window.as_weak();
    let controller_for_nag = controller.clone();

    window.on_context_menu_nag_picked(move |code| {
        let mut ctrl = controller_for_nag.borrow_mut();
        if let Some(win) = window_weak_nag.upgrade() {
            let node_id = win.get_context_menu_node_id();
            if node_id >= 0 && ctrl.toggle_move_nag(node_id as usize, code) {
                win.set_moves(ModelRc::new(VecModel::from(ctrl.build_move_rows())));
            }
        }
    });

    // 9ter. Callback: "Promote to main line" (context menu,
    // right-click on a variation) — PHASE 16, Step 6.2. Also moves the
    // active line up to the tip of the promoted variation (see the doc of
    // `GameController::promote_variation_to_mainline`), hence a
    // full refresh (position, history, status) rather than the
    // simple `set_moves` used for the NAG.
    let window_weak_promote    = window.as_weak();
    let controller_for_promote = controller.clone();
    let lang_for_promote       = lang_cell.clone();

    window.on_context_menu_promote_picked(move || {
        let mut ctrl = controller_for_promote.borrow_mut();
        if let Some(win) = window_weak_promote.upgrade() {
            let node_id = win.get_context_menu_node_id();
            if node_id >= 0 && ctrl.promote_variation_to_mainline(node_id as usize) {
                let lang = *lang_for_promote.borrow();
                refresh_game_state(&win, &ctrl, lang);
            }
        }
    });

    // 9quater. Callback: "Delete this variation" (context menu, right
    // click) — PHASE 16, Step 6.2. Never affects the active line (see the
    // safeguard in `History::remove_variation`): only the displayed
    // history (the variation blocks) needs to be refreshed.
    let window_weak_delete    = window.as_weak();
    let controller_for_delete = controller.clone();

    window.on_context_menu_delete_picked(move || {
        let mut ctrl = controller_for_delete.borrow_mut();
        if let Some(win) = window_weak_delete.upgrade() {
            let node_id = win.get_context_menu_node_id();
            if node_id >= 0 && ctrl.remove_variation(node_id as usize) {
                win.set_moves(ModelRc::new(VecModel::from(ctrl.build_move_rows())));
            }
        }
    });

    // 9quinquies. Callback: comment validated (inline editing below the
    // move, context menu → "Add a comment") — PHASE 16, Step
    // 6.3, limited to the main line. The targeted node is provided
    // directly as a callback parameter (unlike NAG/promote/
    // delete): `context-menu-node-id` has already been reset to -1 by
    // `comment-picked` in app.slint at the moment the inline
    // editor was opened, well before this save happens.
    let window_weak_comment    = window.as_weak();
    let controller_for_comment = controller.clone();

    window.on_move_comment_saved(move |node_id, text| {
        let mut ctrl = controller_for_comment.borrow_mut();
        if let Some(win) = window_weak_comment.upgrade() {
            if node_id >= 0 && ctrl.set_move_comment(node_id as usize, text.as_str()) {
                win.set_moves(ModelRc::new(VecModel::from(ctrl.build_move_rows())));
            }
        }
    });

    // 10. Callback: analysis finished → update history + SVG curves
    let window_weak_score  = window.as_weak();
    let score_history_ref  = score_history.clone();

    window.on_analysis_completed(move |score_f32| {
        let mut hist = score_history_ref.borrow_mut();
        hist.push(score_f32);

        if let Some(win) = window_weak_score.upgrade() {
            // Ergonomics follow-up 10/07/2026: the 4 new `ScoreBar`
            // fields (depth/best-move-san/from-deep-pass/move-quality) are
            // only used by the "Détail de la partie" info block — default
            // values here, never read on the Slint side for the main board.
            let bars: Vec<ScoreBar> = hist.iter()
                .map(|&s| ScoreBar {
                    score: s,
                    score_display: String::new().into(),
                    depth: 0,
                    best_move_san: String::new().into(),
                    from_deep_pass: false,
                    move_quality: -1,
                })
                .collect();
            win.set_score_history(ModelRc::new(VecModel::from(bars)));

            let (white_path, black_path) = compute_score_paths(&hist);
            win.set_white_curve_path(white_path.into());
            win.set_black_curve_path(black_path.into());
        }
    });

    // 11. Callback: new game
    let window_weak_new       = window.as_weak();
    let controller_for_new    = controller.clone();
    let analysis_for_new      = analysis.clone();
    let game_bridge_for_new   = game_bridge.clone();
    let score_history_new     = score_history.clone();
    let lang_for_new          = lang_cell.clone();
    let chess_clock_new       = chess_clock.clone();

    // ── Board flip ──────────────────────────────────────────────────────────
    {
        let window_flip = window.as_weak();
        window.on_flip_board(move || {
            if let Some(w) = window_flip.upgrade() {
                let flipped = w.get_board_flipped();
                w.set_board_flipped(!flipped);
            }
        });
    }

    // ── Assist mode (PHASE 68) ────────────────────────────────────────────────
    // Toggles the state AND immediately rebuilds `squares`: if a piece
    // is already selected at the moment the 💡 button is clicked, the badges
    // must appear/disappear right away, without waiting for a
    // new selection.
    {
        let window_assist     = window.as_weak();
        let controller_assist = controller.clone();
        window.on_toggle_assist_mode(move || {
            let Some(w) = window_assist.upgrade() else { return };
            let new_active = !w.get_assist_mode_active();
            w.set_assist_mode_active(new_active);
            let mut ctrl = controller_assist.borrow_mut();
            ctrl.set_assist_mode(new_active);
            w.set_squares(ModelRc::new(VecModel::from(ctrl.build_squares())));
        });
    }

    // ── Pause / resume the game ──────────────────────────────────────────────
    {
        let chess_clock_pause = chess_clock.clone();
        let game_bridge_pause = game_bridge.clone();
        let analysis_pause    = analysis.clone();
        let controller_pause  = controller.clone();
        let window_weak_pause = window.as_weak();
        let lang_for_pause    = lang_cell.clone();
        let book_white_pause  = book_white.clone();
        let book_black_pause  = book_black.clone();

        window.on_toggle_pause(move || {
            let Some(win) = window_weak_pause.upgrade() else { return };

            if win.get_game_paused() {
                // ── Resume ───────────────────────────────────────────────────
                win.set_game_paused(false);

                let is_white = win.get_is_white_turn();
                chess_clock_pause.borrow_mut().start(is_white);
                win.set_white_clock_active(is_white);
                win.set_black_clock_active(!is_white);

                let mut fen      = controller_pause.borrow().current_fen();
                let mut is_white = is_white;

                // If it's the engine player's turn, restart it — unless
                // a computation was already in progress BEFORE the pause: pausing
                // does not interrupt the engine-player thread (`analyze()`
                // keeps running in the background), so its `bestmove` will eventually
                // arrive and will be applied normally now that
                // `game_paused` is back to `false`. Firing an extra request
                // here would create a duplicate second request on the
                // same position (risk of an extra move being applied / wasted
                // duplicate engine computation).
                // PHASE 26, Step 2: never restart the engine while
                // variation editing is active — resuming the game must
                // not interrupt an ongoing exploration (the engine
                // stays silent until `exit_variation_editing()`, wired in
                // Step 3).
                if !win.get_engine_playing() && !controller_pause.borrow().is_variation_editing() {
                    // PHASE 15: book move(s) before consulting the engine.
                    if try_play_book_moves(&win, &controller_pause, &chess_clock_pause, &game_bridge_pause, &book_white_pause, &book_black_pause) {
                        let lang = *lang_for_pause.borrow();
                        refresh_game_state(&win, &controller_pause.borrow(), lang);
                        let ctrl = controller_pause.borrow();
                        is_white = ctrl.is_white_turn();
                        fen      = ctrl.current_fen();
                        drop(ctrl);
                        chess_clock_pause.borrow_mut().start(is_white);
                        win.set_white_clock_active(is_white);
                        win.set_black_clock_active(!is_white);
                    }

                    let limits_opt = build_go_limits(&chess_clock_pause.borrow());
                    let _ = game_bridge_pause.borrow().trigger_if_engine_turn(
                        is_white, fen.clone(), limits_opt,
                    );
                }

                // Resume Multi-PV analysis if configured
                let mpv_w = win.get_analysis_multipv_white();
                let mpv_b = win.get_analysis_multipv_black();
                if mpv_w > 0 {
                    analysis_pause.borrow_mut().start_for(
                        true, fen.clone(), win.as_weak(), is_white, mpv_w as u32,
                    );
                }
                if mpv_b > 0 {
                    analysis_pause.borrow_mut().start_for(
                        false, fen, win.as_weak(), is_white, mpv_b as u32,
                    );
                }
            } else {
                // ── Pause ────────────────────────────────────────────────────
                win.set_game_paused(true);

                chess_clock_pause.borrow_mut().stop();
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);

                analysis_pause.borrow_mut().stop();
                win.set_engine_thinking(false);
            }
        });
    }

    // ── PHASE 26, Step 3: entering / exiting variation-editing mode ──────────
    {
        let controller_var  = controller.clone();
        let window_weak_var = window.as_weak();
        let lang_for_var    = lang_cell.clone();

        window.on_enter_variation_editing(move || {
            gui::debug_log::log_event("callback_enter_variation_editing", &serde_json::json!({}));
            let Some(win) = window_weak_var.upgrade() else { return };
            let mut ctrl = controller_var.borrow_mut();
            ctrl.enter_variation_editing();
            drop(ctrl);
            let lang = *lang_for_var.borrow();
            refresh_game_state(&win, &controller_var.borrow(), lang);
        });
    }

    {
        let controller_var2  = controller.clone();
        let chess_clock_var  = chess_clock.clone();
        let game_bridge_var  = game_bridge.clone();
        let analysis_var     = analysis.clone();
        let window_weak_var2 = window.as_weak();
        let lang_for_var2    = lang_cell.clone();
        let book_white_var   = book_white.clone();
        let book_black_var   = book_black.clone();

        window.on_exit_variation_editing(move || {
            gui::debug_log::log_event("callback_exit_variation_editing", &serde_json::json!({}));
            let Some(win) = window_weak_var2.upgrade() else { return };

            {
                let mut ctrl = controller_var2.borrow_mut();
                ctrl.exit_variation_editing();
            }

            let lang = *lang_for_var2.borrow();
            refresh_game_state(&win, &controller_var2.borrow(), lang);

            // Ending a variation "resumes" the game exactly like exiting
            // pause: clock, book, engine (H vs Engine included,
            // PHASE 26 having opened the feature to this mode) and Multi-PV
            // analysis all restart, unless the game is over.
            if controller_var2.borrow().is_over() {
                chess_clock_var.borrow_mut().stop();
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);
                return;
            }

            let mut is_white = win.get_is_white_turn();
            chess_clock_var.borrow_mut().start(is_white);
            win.set_white_clock_active(is_white);
            win.set_black_clock_active(!is_white);

            let mut fen = controller_var2.borrow().current_fen();

            if !win.get_engine_playing() {
                if try_play_book_moves(&win, &controller_var2, &chess_clock_var, &game_bridge_var, &book_white_var, &book_black_var) {
                    let lang = *lang_for_var2.borrow();
                    refresh_game_state(&win, &controller_var2.borrow(), lang);
                    let ctrl = controller_var2.borrow();
                    is_white = ctrl.is_white_turn();
                    fen      = ctrl.current_fen();
                    drop(ctrl);
                    chess_clock_var.borrow_mut().start(is_white);
                    win.set_white_clock_active(is_white);
                    win.set_black_clock_active(!is_white);
                }

                let limits_opt = build_go_limits(&chess_clock_var.borrow());
                let _ = game_bridge_var.borrow().trigger_if_engine_turn(
                    is_white, fen.clone(), limits_opt,
                );
            }

            let mpv_w = win.get_analysis_multipv_white();
            let mpv_b = win.get_analysis_multipv_black();
            if mpv_w > 0 {
                analysis_var.borrow_mut().start_for(
                    true, fen.clone(), win.as_weak(), is_white, mpv_w as u32,
                );
            }
            if mpv_b > 0 {
                analysis_var.borrow_mut().start_for(
                    false, fen, win.as_weak(), is_white, mpv_b as u32,
                );
            }
        });
    }

    // ── Hint engine: selection ────────────────────────────────────────────────
    {
        let window_hint_set   = window.as_weak();
        let engine_list_hint  = engine_list.clone();
        let hint_path_set     = hint_engine_path.clone();
        let analysis_hint_set = analysis.clone();

        window.on_set_hint_engine(move |idx| {
            let engines = engine_list_hint.borrow();
            let new_path: Option<String> = if idx < 0 {
                None
            } else {
                engines.get(idx as usize).map(|e| e.path.clone())
            };
            // Persist the choice
            prefs::save_hint_engine(new_path.as_deref());
            // Update the Rust state
            hint_path_set.borrow_mut().clone_from(&new_path);
            // Synchronize the analysis engine (hint takes priority)
            let engine_name = sync_analysis_engine(
                &mut analysis_hint_set.borrow_mut(),
                new_path.as_ref(),
                &engines,
            );
            // Update the window
            if let Some(w) = window_hint_set.upgrade() {
                w.set_hint_engine_path(hint_engine_path_for_window(new_path.as_deref()).into());
                w.set_hint_engine_name(hint_engine_display_name(new_path.as_deref(), &engines).into());
                w.set_analysis_engine_name(engine_name.into());
            }
        });
    }

    // ── Polyglot opening books (PHASE 15, Steps 4 and 6) ──────────────────────
    // Loading + validation via a native file dialog (Step 4). The
    // validation covers the Polyglot format (size a multiple of 16, sorted
    // by hash, via PolyglotBook::open). Since Step 6, the loaded book
    // is also placed into the runtime state (`book_white`/`book_black`) to
    // be used immediately in a game, with no restart.
    {
        let window_book_w = window.as_weak();
        let book_white_set = book_white.clone();
        window.on_browse_book_white(move || {
            let Some(win) = window_book_w.upgrade() else { return; };
            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.load_book_white_title"))
                .add_filter(i18n::translate("dialog.polyglot_filter_label"), &["bin"])
                .pick_file()
            else { return; };

            match chess_core::polyglot::PolyglotBook::open(&path) {
                Ok(book) => {
                    // PHASE 24, Step 6: copies into ouvertures/blancs.bin,
                    // regardless of the file's original name — replaces the
                    // previous White book if there was one.
                    let dest = app_paths::book_blancs_path();
                    if let Err(e) = app_paths::copy_overwrite(&path, &dest) {
                        rfd::MessageDialog::new()
                            .set_title(i18n::translate("dialog.import_impossible_title"))
                            .set_description(
                                i18n::translate("dialog.book_import_failed_desc")
                                    .replace("{err}", &e.to_string()),
                            )
                            .set_level(rfd::MessageLevel::Warning)
                            .show();
                        return;
                    }
                    let entry_count = book.len();
                    win.set_book_name_white("blancs.bin".into());
                    *book_white_set.borrow_mut() = Some(book);
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.book_loaded_title"))
                        .set_description(
                            i18n::translate("dialog.book_loaded_desc")
                                .replace("{name}", "blancs.bin")
                                .replace("{count}", &entry_count.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Info)
                        .show();
                }
                Err(e) => {
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.invalid_file_title"))
                        .set_description(
                            i18n::translate("dialog.invalid_book_file_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }
    {
        let window_book_b = window.as_weak();
        let book_black_set = book_black.clone();
        window.on_browse_book_black(move || {
            let Some(win) = window_book_b.upgrade() else { return; };
            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.load_book_black_title"))
                .add_filter(i18n::translate("dialog.polyglot_filter_label"), &["bin"])
                .pick_file()
            else { return; };

            match chess_core::polyglot::PolyglotBook::open(&path) {
                Ok(book) => {
                    // PHASE 24, Step 6: copies into ouvertures/noirs.bin,
                    // regardless of the file's original name — replaces the
                    // previous Black book if there was one.
                    let dest = app_paths::book_noirs_path();
                    if let Err(e) = app_paths::copy_overwrite(&path, &dest) {
                        rfd::MessageDialog::new()
                            .set_title(i18n::translate("dialog.import_impossible_title"))
                            .set_description(
                                i18n::translate("dialog.book_import_failed_desc")
                                    .replace("{err}", &e.to_string()),
                            )
                            .set_level(rfd::MessageLevel::Warning)
                            .show();
                        return;
                    }
                    let entry_count = book.len();
                    win.set_book_name_black("noirs.bin".into());
                    *book_black_set.borrow_mut() = Some(book);
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.book_loaded_title"))
                        .set_description(
                            i18n::translate("dialog.book_loaded_desc")
                                .replace("{name}", "noirs.bin")
                                .replace("{count}", &entry_count.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Info)
                        .show();
                }
                Err(e) => {
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.invalid_file_title"))
                        .set_description(
                            i18n::translate("dialog.invalid_book_file_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }
    {
        let window_book_w_clear = window.as_weak();
        let book_white_clear    = book_white.clone();
        window.on_clear_book_white(move || {
            // PHASE 24, Step 6: no more separate registry — the presence of a
            // book is deduced solely from the file's existence, so
            // "clearing" means deleting the file itself.
            let _ = std::fs::remove_file(app_paths::book_blancs_path());
            *book_white_clear.borrow_mut() = None;
            if let Some(win) = window_book_w_clear.upgrade() {
                win.set_book_name_white("".into());
            }
        });
    }
    {
        let window_book_b_clear = window.as_weak();
        let book_black_clear    = book_black.clone();
        window.on_clear_book_black(move || {
            let _ = std::fs::remove_file(app_paths::book_noirs_path());
            *book_black_clear.borrow_mut() = None;
            if let Some(win) = window_book_b_clear.upgrade() {
                win.set_book_name_black("".into());
            }
        });
    }

    // ── Tactical puzzles (PHASE 14, Step 3; background import with a
    // progress indicator — bugfix from 03/07/2026) ───────────────────────────
    // Importing a CSV file (e.g. a Lichess Puzzles export) supplied by
    // the user — no database ships with the software, same
    // principle as the Polyglot opening books above. Format validation
    // (header + rows) is delegated to `puzzle_repo::import_csv_with_progress`.
    //
    // A large file (the full Lichess Puzzles export is ~300 MB)
    // can take long enough to look like a crash with no
    // visual feedback: the import therefore runs on a dedicated thread (same
    // principle as the hint engine below), with a live status pushed via
    // `puzzle-import-status` every `PROGRESS_EVERY` rows processed.
    {
        let window_puzzles = window.as_weak();
        window.on_import_puzzles(move || {
            let Some(win) = window_puzzles.upgrade() else { return; };
            if win.get_puzzle_import_active() { return; } // import already in progress

            // Accepts the raw CSV or the official Zstandard-compressed
            // Lichess file (`lichess_db_puzzle.csv.zst`) — decompression
            // is streamed by `puzzle_repo::import_csv_with_progress`,
            // no manual step is required from the user.
            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.import_puzzles_title"))
                .add_filter(i18n::translate("dialog.puzzles_filter_label"), &["csv", "zst"])
                .pick_file()
            else { return; };

            win.set_puzzle_import_active(true);
            win.set_puzzle_import_status(i18n::translate("status.puzzle_import_in_progress").into());

            let window_back = window_puzzles.clone();

            std::thread::spawn(move || {
            let window_panic = window_back.clone();
            run_guarded_thread(std::panic::AssertUnwindSafe(move || {
                let conn = match db::schema::open_and_migrate(&tournament_db_path()) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_puzzle_import_active(false);
                                w.set_puzzle_import_status("".into());
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.db_unavailable_title"))
                                .set_description(format!("{e}"))
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                        return;
                    }
                };

                let window_progress = window_back.clone();
                let result = db::repository::puzzle_repo::import_csv_with_progress(
                    &conn,
                    &path,
                    move |done| {
                        let window_progress = window_progress.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_progress.upgrade() {
                                w.set_puzzle_import_status(
                                    i18n::translate("status.puzzle_import_lines_processed")
                                        .replace("{done}", &done.to_string())
                                        .into(),
                                );
                            }
                        });
                    },
                );

                match result {
                    Ok(summary) => {
                        let count_res = db::repository::puzzle_repo::count(&conn);
                        let stats_res = db::repository::puzzle_repo::global_stats(&conn);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_puzzle_import_active(false);
                                w.set_puzzle_import_status("".into());
                                if let Ok(n) = count_res {
                                    w.set_puzzle_count(i32::try_from(n).unwrap_or(i32::MAX));
                                }
                                if let Ok(stats) = stats_res {
                                    push_puzzle_stats(&w, stats);
                                }
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.import_finished_title"))
                                .set_description(
                                    i18n::translate("dialog.import_finished_desc")
                                        .replace("{imported}", &summary.imported.to_string())
                                        .replace("{skipped}", &summary.skipped.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Info)
                                .show();
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_puzzle_import_active(false);
                                w.set_puzzle_import_status("".into());
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.invalid_file_title"))
                                .set_description(
                                    i18n::translate("dialog.invalid_puzzle_file_desc")
                                        .replace("{err}", &e.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                    }
                }
            }), window_panic, |w| {
                w.set_puzzle_import_active(false);
                w.set_puzzle_import_status(String::new().into());
                show_unexpected_error_dialog();
            });
            });
        });
    }

    // ── Unload the puzzle database (bugfix from 03/07/2026) ───────────────────
    // Clears puzzles + progress statistics without going through a
    // full preferences reset — native Yes/No confirmation,
    // same mechanism as the project's other destructive actions.
    {
        let window_unload = window.as_weak();
        window.on_unload_puzzles(move || {
            let Some(win) = window_unload.upgrade() else { return; };
            if win.get_puzzle_import_active() { return; } // not during an import

            let confirmed = rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.unload_puzzles_title"))
                .set_description(i18n::translate("dialog.unload_puzzles_desc"))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes;
            if !confirmed { return; }

            match db::schema::open_and_migrate(&tournament_db_path()) {
                Ok(conn) => {
                    if let Err(e) = db::repository::puzzle_repo::clear_all(&conn) {
                        rfd::MessageDialog::new()
                            .set_title(i18n::translate("dialog.generic_error_title"))
                            .set_description(format!("{e}"))
                            .set_level(rfd::MessageLevel::Error)
                            .show();
                        return;
                    }
                    win.set_puzzle_count(0);
                    if let Ok(stats) = db::repository::puzzle_repo::global_stats(&conn) {
                        push_puzzle_stats(&win, stats);
                    }
                }
                Err(e) => {
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.db_unavailable_title"))
                        .set_description(format!("{e}"))
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── Import of the reference games database (PHASE 82) ────────────────────
    // Same architecture as the puzzle import above: dedicated thread,
    // `reference-import-active` flag preventing a double import, live status
    // pushed via `slint::invoke_from_event_loop`, final
    // imported/skipped summary. Assumed difference (decision made in discussion):
    // a new import **entirely replaces** the existing database — the
    // `SQLite` file is deleted then recreated empty before the import, rather
    // than adding to the games already present (no duplicate detection
    // between two versions of the same PGN file for now).
    {
        let window_ref = window.as_weak();
        window.on_import_reference_base(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            if win.get_reference_import_active() { return; } // import already in progress

            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.import_reference_base_title"))
                .add_filter(i18n::translate("dialog.reference_pgn_filter_label"), &["pgn"])
                .pick_file()
            else { return; };

            win.set_reference_import_active(true);
            win.set_reference_import_status(i18n::translate("status.reference_import_in_progress").into());
            win.set_reference_import_progress(0.0);

            let window_back = window_ref.clone();
            std::thread::spawn(move || {
            let window_panic = window_back.clone();
            run_guarded_thread(std::panic::AssertUnwindSafe(move || {
                let db_path = reference_pgn_db_path();
                // Full reimport: starts from a blank file rather than
                // adding to the games already in the database (decision made PHASE 82).
                // Dedicated PGN file (11/07/2026): never affects the SCID database.
                let _ = std::fs::remove_file(&db_path);

                let conn = match db::reference_schema::open_and_migrate(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.db_unavailable_title"))
                                .set_description(format!("{e}"))
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                        return;
                    }
                };

                let window_progress = window_back.clone();
                let result = db::reference_import::import_pgn_file_with_progress(
                    &conn,
                    &path,
                    move |done, total| {
                        let window_progress = window_progress.clone();
                        // Ratio [0.0, 1.0] for the progress bar
                        // (perf bugfix 09/07/2026, user request) —
                        // `total` known from the first call (`split_pgn_games`
                        // materializes the whole file before the loop), guards
                        // `total == 0` for an empty PGN file (0/0 → 0.0,
                        // no division by zero).
                        #[allow(clippy::cast_precision_loss)]
                        let progress = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_progress.upgrade() {
                                w.set_reference_import_status(
                                    i18n::translate("status.reference_import_games_processed")
                                        .replace("{done}", &done.to_string())
                                        .replace("{total}", &total.to_string())
                                        .into(),
                                );
                                w.set_reference_import_progress(progress);
                            }
                        });
                    },
                );

                match result {
                    Ok(summary) => {
                        // Real recount in the database rather than blind trust in
                        // `summary.imported` (bugfix 09/07/2026, user feedback:
                        // dialog announcing a successful import while the database
                        // stayed empty) — same principle as the puzzle import
                        // above, which already recounts via `puzzle_repo::count`
                        // rather than trusting the import summary.
                        // Single source of truth: if a discrepancy ever appears between
                        // "games successfully processed" and "games actually
                        // present in the database", it is this count that drives the state
                        // of the interface (wizard card, Preferences counter).
                        let real_count = db::repository::reference_game_repo::count_matching(
                            &conn,
                            &db::repository::reference_game_repo::GameFilter::default(),
                        ).unwrap_or(0);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                                w.set_reference_pgn_game_count(
                                    i32::try_from(real_count).unwrap_or(i32::MAX),
                                );
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.import_finished_title"))
                                .set_description(
                                    i18n::translate("dialog.import_reference_base_finished_desc")
                                        .replace("{imported}", &summary.imported.to_string())
                                        .replace("{skipped}", &summary.skipped.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Info)
                                .show();
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.invalid_file_title"))
                                .set_description(
                                    i18n::translate("dialog.invalid_reference_pgn_file_desc")
                                        .replace("{err}", &e.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                    }
                }
            }), window_panic, |w| {
                w.set_reference_import_active(false);
                w.set_reference_import_status(String::new().into());
                w.set_reference_import_progress(0.0);
                show_unexpected_error_dialog();
            });
            });
        });
    }

    // ── Importing a SCID database (.si4 or .si5) into the reference games
    //    database (follow-up 11/07/2026; si5 added 12/07/2026, V2 Phase C1,
    //    task #21) ─────────────────────────────────────────────────────────
    // Same architecture as `on_import_reference_base` just above (dedicated
    // thread, shared `reference-import-active` flag, full replacement of
    // the database rather than incremental import) — the only difference: the
    // chosen file is a `.si4` OR a `.si5` (the associated files — `.sn4`/`.sg4`
    // or `.sn5`/`.sg5` — are derived from the same base name by
    // `scid::Si4Paths::from_index_path`/`scid::Si5Paths::from_index_path`),
    // and the binary decoding goes through `db::scid_import::import_si4_file_
    // with_progress`/`import_si5_file_with_progress` (crate `scid`) rather
    // than a simple text PGN parser. The format is determined by
    // the chosen file's extension (see `is_si5` further below) — a single
    // "Importer SCID" button for both formats, consistent with the decision made
    // in discussion: si4 and si5 share the same SCID reference database.
    {
        let window_ref = window.as_weak();
        window.on_import_reference_si4_base(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            if win.get_reference_import_active() { return; } // import already in progress

            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.import_si4_base_title"))
                .add_filter(i18n::translate("dialog.si4_filter_label"), &["si4", "si5"])
                .pick_file()
            else { return; };

            let is_si5 = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("si5"));

            win.set_reference_import_active(true);
            win.set_reference_import_status(i18n::translate("status.reference_import_in_progress").into());
            win.set_reference_import_progress(0.0);

            let window_back = window_ref.clone();
            std::thread::spawn(move || {
            let window_panic = window_back.clone();
            run_guarded_thread(std::panic::AssertUnwindSafe(move || {
                let db_path = reference_scid_db_path();
                // Full reimport: same policy as the PGN import
                // (`on_import_reference_base`) — no incremental addition.
                // Dedicated SCID file (11/07/2026): never affects the PGN database.
                let _ = std::fs::remove_file(&db_path);

                let conn = match db::reference_schema::open_and_migrate(&db_path) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.db_unavailable_title"))
                                .set_description(format!("{e}"))
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                        return;
                    }
                };

                let window_progress = window_back.clone();
                let progress_cb = move |done: usize, total: usize| {
                        let window_progress = window_progress.clone();
                        #[allow(clippy::cast_precision_loss)]
                        let progress = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_progress.upgrade() {
                                w.set_reference_import_status(
                                    i18n::translate("status.reference_import_games_processed")
                                        .replace("{done}", &done.to_string())
                                        .replace("{total}", &total.to_string())
                                        .into(),
                                );
                                w.set_reference_import_progress(progress);
                            }
                        });
                    };

                let result = if is_si5 {
                    db::scid_import::import_si5_file_with_progress(&conn, &path, progress_cb)
                } else {
                    db::scid_import::import_si4_file_with_progress(&conn, &path, progress_cb)
                };

                match result {
                    Ok(summary) => {
                        // Real recount in the database rather than blind trust
                        // in `summary.imported` — same principle as
                        // `on_import_reference_base` above.
                        let real_count = db::repository::reference_game_repo::count_matching(
                            &conn,
                            &db::repository::reference_game_repo::GameFilter::default(),
                        ).unwrap_or(0);
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                                w.set_reference_scid_game_count(
                                    i32::try_from(real_count).unwrap_or(i32::MAX),
                                );
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.import_finished_title"))
                                .set_description(
                                    i18n::translate("dialog.import_reference_base_finished_desc")
                                        .replace("{imported}", &summary.imported.to_string())
                                        .replace("{skipped}", &summary.skipped.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Info)
                                .show();
                        });
                    }
                    Err(e) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(w) = window_back.upgrade() {
                                w.set_reference_import_active(false);
                                w.set_reference_import_status("".into());
                                w.set_reference_import_progress(0.0);
                            }
                            rfd::MessageDialog::new()
                                .set_title(i18n::translate("dialog.invalid_file_title"))
                                .set_description(
                                    i18n::translate("dialog.invalid_si4_file_desc")
                                        .replace("{err}", &e.to_string()),
                                )
                                .set_level(rfd::MessageLevel::Error)
                                .show();
                        });
                    }
                }
            }), window_panic, |w| {
                w.set_reference_import_active(false);
                w.set_reference_import_status(String::new().into());
                w.set_reference_import_progress(0.0);
                show_unexpected_error_dialog();
            });
            });
        });
    }

    // ── Unload the PGN reference games database (bugfix 09/07/2026) ─────────
    // Was missing from the initial implementation (user feedback from 09/07/2026,
    // Preferences → Base de parties) — same Yes/No confirmation mechanism
    // as `on_unload_puzzles`. Simply deletes then recreates (empty) the
    // dedicated `SQLite` file, as a reimport already does (see
    // `on_import_reference_base` above): the reference database lives in
    // its own file, no need for a table-by-table `DELETE FROM`.
    // PGN file only (11/07/2026): never affects the SCID database.
    {
        let window_clear_ref = window.as_weak();
        window.on_clear_reference_base(move || {
            let Some(win) = window_clear_ref.upgrade() else { return; };
            if win.get_reference_import_active() { return; } // not during an import

            let confirmed = rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.clear_reference_base_title"))
                .set_description(i18n::translate("dialog.clear_reference_base_desc"))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes;
            if !confirmed { return; }

            let db_path = reference_pgn_db_path();
            let _ = std::fs::remove_file(&db_path);
            match db::reference_schema::open_and_migrate(&db_path) {
                Ok(_conn) => {
                    win.set_reference_pgn_game_count(0);
                }
                Err(e) => {
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.db_unavailable_title"))
                        .set_description(format!("{e}"))
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── Unload the SCID reference games database (follow-up 11/07/2026) ─────
    // Symmetric to the PGN block above, SCID file only (see the
    // "two databases" discussion from 11/07/2026: never affects the PGN database).
    {
        let window_clear_scid = window.as_weak();
        window.on_clear_reference_si4_base(move || {
            let Some(win) = window_clear_scid.upgrade() else { return; };
            if win.get_reference_import_active() { return; } // not during an import

            let confirmed = rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.clear_reference_base_title"))
                .set_description(i18n::translate("dialog.clear_reference_base_desc"))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes;
            if !confirmed { return; }

            let db_path = reference_scid_db_path();
            let _ = std::fs::remove_file(&db_path);
            match db::reference_schema::open_and_migrate(&db_path) {
                Ok(_conn) => {
                    win.set_reference_scid_game_count(0);
                }
                Err(e) => {
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.db_unavailable_title"))
                        .set_description(format!("{e}"))
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── Search in the reference games database (PHASE 82, step 7) ────────────
    // Called when the exploration screen is opened (unfiltered search,
    // page 0 — see `open-navigation-base` in `app.slint`) and on every
    // filter/page change ("Rechercher", "Réinitialiser", pagination —
    // see the `ReferenceBrowser` callbacks routed to this same entry
    // point). Unlike the full PGN import (potentially
    // several minutes on a large Gigabase), a paginated search
    // (`LIMIT` 50, indexed columns — see `reference_game_repo.rs`) stays
    // on the order of a millisecond: no dedicated thread nor
    // loading indicator needed.
    {
        let window_ref = window.as_weak();
        let applied_ref = tree_applied_game_ids.clone();
        window.on_search_reference_games(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            // The "Arbre d'ouverture" filter that may be active (chip in
            // the "Parties" tab) keeps applying ON TOP of a
            // new classic search — decision made with
            // the user: only an explicit click on the chip (see
            // `on_tree_filter_clear`) removes it, never a "Rechercher".
            run_reference_search(&win, applied_ref.borrow().as_deref());
        });
    }

    // ── Opening tree (PHASE 82, step 8) ───────────────────────────────────────
    // Four callbacks sharing the same path (`opening_tree_path`) and the
    // same refresh logic (`refresh_opening_tree`, above,
    // outside `main()`): picking a move extends it, "Retour" removes the
    // last element, "Recommencer" clears it, changing the Elo threshold leaves it
    // unchanged (only the query is rerun).
    {
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_move_chosen(move |uci| {
            let Some(win) = window_ref.upgrade() else { return; };
            path_ref.borrow_mut().push(uci.to_string());
            // New path: the checked moves from the previous table no
            // longer correspond to anything (the displayed rows are those of
            // the newly reached position).
            checked_ref.borrow_mut().clear();
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_go_back(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            path_ref.borrow_mut().pop();
            checked_ref.borrow_mut().clear();
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_reset(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            path_ref.borrow_mut().clear();
            checked_ref.borrow_mut().clear();
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_filter_changed(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            // Elo threshold change only: the path (hence the
            // displayed moves) does not change, the checked boxes stay
            // valid and are kept (see the doc of
            // `refresh_opening_tree`).
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        // Ergonomics follow-up 10/07/2026 — checkbox of a table row.
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_toggle_move(move |uci| {
            let Some(win) = window_ref.upgrade() else { return; };
            let uci = uci.to_string();
            {
                let mut checked = checked_ref.borrow_mut();
                if !checked.remove(&uci) {
                    checked.insert(uci);
                }
            }
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        // "Check all" / "Uncheck all" — toggles based on the already-displayed
        // `tree-all-checked` (computed by `refresh_opening_tree`, never
        // independently recomputed here to avoid any divergence).
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        window.on_tree_toggle_all(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            if win.get_tree_all_checked() {
                checked_ref.borrow_mut().clear();
            } else {
                let uci_list: Vec<String> = win
                    .get_tree_candidates()
                    .iter()
                    .map(|m| m.uci.to_string())
                    .collect();
                *checked_ref.borrow_mut() = uci_list.into_iter().collect();
            }
            refresh_opening_tree(&win, &path_ref.borrow(), &checked_ref.borrow());
        });
    }
    {
        // "Lister les parties (N)" — applies the current selection as the
        // `game-ids` filter of the "Parties" tab (decision made with
        // the user: combines with the classic filters, with a logical
        // AND) then switches to it.
        let window_ref = window.as_weak();
        let path_ref = opening_tree_path.clone();
        let checked_ref = tree_checked_moves.clone();
        let applied_ref = tree_applied_game_ids.clone();
        window.on_tree_list_games(move || {
            let Some(win) = window_ref.upgrade() else { return; };

            let Some((pos, display, _last_move)) = replay_opening_tree_path(&path_ref.borrow()) else { return; };
            let hash = chess_core::polyglot::polyglot_hash(&pos);
            let min_elo = win.get_tree_elo_min().trim().parse::<i64>().ok();

            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };

            let checked = checked_ref.borrow();
            let allowed: Option<Vec<String>> =
                (!checked.is_empty()).then(|| checked.iter().cloned().collect());
            let ids = db::repository::opening_repo::games_for_path(
                &conn, hash, allowed.as_deref(), min_elo,
            )
            .unwrap_or_default();
            drop(checked);

            *applied_ref.borrow_mut() = Some(ids);

            // The number of matching games is already shown via
            // `reference-browser-total-count` once the search is rerun
            // below — no need to duplicate it in the chip's label.
            // Reuses the `tree_start_position` key (already translated into
            // 40 languages, the tree's breadcrumb) rather than duplicating
            // its content into a new key.
            let position_desc = if display.is_empty() {
                i18n::translate("refbrowser.tree_start_position")
            } else {
                display
            };
            let label = format!("{} {}", i18n::translate("refbrowser.tree_filter_chip_prefix"), position_desc);
            win.set_tree_filter_active(true);
            win.set_tree_filter_label(label.into());

            win.set_reference_browser_active_tab(0);
            win.set_reference_browser_page(0);
            run_reference_search(&win, applied_ref.borrow().as_deref());
        });
    }
    {
        // "X" chip of the "Parties" tab — clears the `game-ids` filter coming
        // from the tree and reruns a search with only the classic filters.
        let window_ref = window.as_weak();
        let applied_ref = tree_applied_game_ids.clone();
        window.on_tree_filter_clear(move || {
            let Some(win) = window_ref.upgrade() else { return; };
            *applied_ref.borrow_mut() = None;
            win.set_tree_filter_active(false);
            win.set_tree_filter_label("".into());
            win.set_reference_browser_page(0);
            run_reference_search(&win, None);
        });
    }

    // ── Game detail: on-demand evaluation curve (PHASE 82,
    // step 9) ─────────────────────────────────────────────────────────────
    {
        let window_ref = window.as_weak();
        window.on_open_game_detail(move |id| {
            let Some(win) = window_ref.upgrade() else { return; };
            build_game_detail(&win, id);
        });
    }
    {
        let window_ref = window.as_weak();
        window.on_analyze_game_detail(move |movetime_ms| {
            let Some(win) = window_ref.upgrade() else { return; };
            if win.get_game_detail_analysis_active() {
                return; // analysis already in progress
            }

            let Some(engine_path) = resolve_analysis_engine_path(&win) else {
                win.set_game_detail_analysis_status(
                    i18n::translate("status.game_analysis_no_engine").into(),
                );
                return;
            };

            let game_id = win.get_game_detail_id();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else {
                return;
            };
            let Ok(Some(row)) =
                db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };
            let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };

            // FEN of each position ply by ply, plus the final position —
            // see the earlier lookup (Step 9): linear via
            // `history().records()`, not `position_at()` (which replays from
            // the start on every call, an unnecessary quadratic cost here).
            let mut fens: Vec<String> =
                game.history().records().iter().map(|r| r.fen_before.clone()).collect();
            fens.push(game.position().to_fen());
            let total = fens.len();

            win.set_game_detail_analysis_active(true);
            win.set_game_detail_analysis_status(
                i18n::translate("status.game_analysis_in_progress").into(),
            );

            let window_thread = window_ref.clone();
            std::thread::spawn(move || {
            let window_panic = window_thread.clone();
            run_guarded_thread(std::panic::AssertUnwindSafe(move || {
                use uci::engine::{EnginePosition, UciEngine};

                let Ok(mut engine) =
                    UciEngine::connect_with_timeout(&engine_path, std::time::Duration::from_secs(3))
                else {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = window_thread.upgrade() {
                            w.set_game_detail_analysis_active(false);
                            w.set_game_detail_analysis_status(
                                i18n::translate("status.game_analysis_no_engine").into(),
                            );
                        }
                    });
                    return;
                };
                // Always force MultiPV to 1: only the main line's score
                // matters here (same principle as
                // `AnalysisBridge::start`).
                let _ = engine.set_option("MultiPV", Some("1"));

                let mut scores: Vec<f32> = Vec::with_capacity(total);
                // Ergonomics follow-up 10/07/2026: depth reached and best
                // move (raw UCI, converted to SAN after the loop once
                // all positions are known) — computed by the engine at
                // each position but discarded until now, keeping only the
                // score. Parallel to `scores` (same index `i`).
                let mut depths: Vec<u32> = Vec::with_capacity(total);
                let mut best_move_ucis: Vec<String> = Vec::with_capacity(total);
                for (i, fen) in fens.iter().enumerate() {
                    // fens[0] = starting position (White to move),
                    // alternates on every ply — same convention as
                    // `replay_opening_tree_path`/`refresh_opening_tree`.
                    let is_white_to_move = i.is_multiple_of(2);
                    let position = EnginePosition::from_fen(fen.clone());
                    // `movetime_ms` comes from a fixed literal on the Slint side (400 or
                    // 1500, see `app.slint`): always positive in practice,
                    // `unwrap_or` stays defensive without ever actually triggering.
                    let movetime = u64::try_from(movetime_ms.max(50)).unwrap_or(400);
                    let limits = GoLimits { movetime: Some(movetime), ..GoLimits::default() };

                    let result = engine.analyze(&position, &limits).ok();
                    let pv_info = result.as_ref().and_then(|r| r.principal_variation());
                    let score = pv_info
                        .map_or(0.0, |pv| gui::analysis_bridge::score_to_f32(pv.score.as_ref(), is_white_to_move));
                    let depth = pv_info.and_then(|info| info.depth).unwrap_or(0);
                    let best_move_uci = pv_info.and_then(|info| info.pv.first().cloned()).unwrap_or_default();
                    scores.push(score);
                    depths.push(depth);
                    best_move_ucis.push(best_move_uci);

                    let done = i + 1;
                    let w = window_thread.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(win) = w.upgrade() {
                            win.set_game_detail_analysis_status(
                                i18n::translate("status.game_analysis_progress")
                                    .replace("{done}", &done.to_string())
                                    .replace("{total}", &total.to_string())
                                    .into(),
                            );
                        }
                    });
                }
                engine.quit();

                let w = window_thread.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(win) = w.upgrade() {
                        win.set_game_detail_analysis_active(false);
                        win.set_game_detail_analysis_status(String::new().into());
                        win.set_game_detail_can_deepen(true);

                        // Ergonomics follow-up 10/07/2026: selected-move
                        // info block — best move in SAN (resolved against
                        // the real position for correct castling/en
                        // passant, see `game_controller::best_move_san`),
                        // the pass's origin (fast/deep, decided
                        // once for the whole pass: same `movetime_ms`
                        // for every position analyzed here), and the
                        // quality of the move played (comparison with the
                        // previous position's score, from the perspective of the player who
                        // just moved — lichess thresholds: Inaccuracy ≥ 0.5
                        // pawn lost · Mistake ≥ 1.0 · Blunder ≥ 2.0; below
                        // that, an Excellent/Good sub-threshold at 0.2 pawn, not
                        // explicitly requested but consistent with the 5 categories
                        // chosen).
                        let from_deep_pass = movetime_ms > 400;
                        let best_move_sans: Vec<String> = fens.iter().zip(best_move_ucis.iter())
                            .map(|(fen, uci)| {
                                chess_core::types::Position::from_fen(fen)
                                    .ok()
                                    .and_then(|pos| game_controller::best_move_san(&pos, uci))
                                    .unwrap_or_default()
                            })
                            .collect();
                        let mut move_quality: Vec<i32> = vec![-1; total];
                        for i in 1..total {
                            let delta_white = scores[i] - scores[i - 1];
                            // The move leading to position i was played by
                            // White if i is odd (position 1 = after
                            // the 1st move, played by White, etc.).
                            let mover_is_white = i % 2 == 1;
                            let delta_for_mover =
                                if mover_is_white { delta_white } else { -delta_white };
                            let loss = (-delta_for_mover).max(0.0);
                            move_quality[i] = if loss < 0.2 {
                                0 // Excellent
                            } else if loss < 0.5 {
                                1 // Good
                            } else if loss < 1.0 {
                                2 // Inaccuracy
                            } else if loss < 2.0 {
                                3 // Mistake
                            } else {
                                4 // Blunder
                            };
                        }

                        let bars: Vec<ScoreBar> = (0..total)
                            .map(|i| ScoreBar {
                                score: scores[i],
                                score_display: format_score_display(scores[i]).into(),
                                depth: i32::try_from(depths[i]).unwrap_or(i32::MAX),
                                best_move_san: best_move_sans[i].clone().into(),
                                from_deep_pass,
                                move_quality: move_quality[i],
                            })
                            .collect();
                        win.set_game_detail_scores(ModelRc::new(VecModel::from(bars)));
                        let (white, black) = compute_score_paths(&scores);
                        win.set_game_detail_white_curve(white.into());
                        win.set_game_detail_black_curve(black.into());
                    }
                });
            }), window_panic, |w| {
                w.set_game_detail_analysis_active(false);
                w.set_game_detail_analysis_status(String::new().into());
                show_unexpected_error_dialog();
            });
            });
        });
    }

    // ── Hint engine: computing a hint ─────────────────────────────────────────
    {
        let window_hint_req  = window.as_weak();
        let hint_path_req    = hint_engine_path.clone();
        let controller_hint  = controller.clone();

        window.on_request_hint(move || {
            let Some(win) = window_hint_req.upgrade() else { return; };

            // Guard checks
            if win.get_hint_computing() { return; }
            if win.get_is_game_over()   { return; }

            let path = hint_path_req.borrow().clone();
            let Some(path) = path else { return }; // no hint engine configured

            let fen     = controller_hint.borrow().current_fen();
            let flipped = win.get_board_flipped();

            win.set_hint_computing(true);
            win.set_hint_arrow_path("".into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);

            let window_back = window_hint_req.clone();

            std::thread::spawn(move || {
            let window_panic = window_back.clone();
            run_guarded_thread(std::panic::AssertUnwindSafe(move || {
                use uci::engine::{UciEngine, EnginePosition};
                use uci::protocol::GoLimits;
                use std::time::Duration;

                // Connect the hint engine and compute the best move
                let result = (|| -> Option<String> {
                    let mut engine = UciEngine::connect_with_timeout(&path, Duration::from_secs(5)).ok()?;
                    let position = EnginePosition::from_fen(&fen);
                    let limits   = GoLimits { movetime: Some(800), ..GoLimits::default() };
                    let analysis = engine.analyze(&position, &limits).ok()?;
                    engine.quit();
                    Some(analysis.best_move)
                })();

                // Compute the SVG arrow if we have a move
                let arrow_path = result.and_then(|mv| {
                    // UCI format: "e2e4" or "e7e8q"
                    let bytes = mv.as_bytes();
                    if bytes.len() < 4 { return None; }
                    let fc = i32::from(bytes[0]) - i32::from(b'a');  // source col (0..7)
                    let fr = 8 - (i32::from(bytes[1]) - i32::from(b'0'));  // source row
                    let tc = i32::from(bytes[2]) - i32::from(b'a');  // dest col
                    let tr = 8 - (i32::from(bytes[3]) - i32::from(b'0'));  // dest row
                    if !(0..8).contains(&fc) || !(0..8).contains(&fr)
                    || !(0..8).contains(&tc) || !(0..8).contains(&tr) {
                        return None;
                    }
                    let s = compute_hint_arrow(fr, fc, tr, tc, flipped);
                    if s.is_empty() { None } else { Some(s) }
                }).unwrap_or_default();

                // Pass back to the UI thread
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = window_back.upgrade() {
                        w.set_hint_arrow_path(arrow_path.into());
                        w.set_hint_computing(false);
                    }
                });
            }), window_panic, |w| {
                w.set_hint_computing(false);
                show_unexpected_error_dialog();
            });
            });
        });
    }

    // Note (PHASE 16, Step 5): the `resume-from-here` callback and the
    // "✂ Reprendre depuis cette position" bar (app.slint) were removed — playing
    // a move from the history now always creates a variation
    // (decision 1) rather than silently truncating the existing line.

    // ── Undo (undo the last move) — H vs H only ───────────────────────────────
    {
        let window_undo     = window.as_weak();
        let controller_undo = controller.clone();
        let analysis_undo   = analysis.clone();
        let game_bridge_undo = game_bridge.clone();
        let lang_undo       = lang_cell.clone();

        window.on_undo_move(move || {
            let lang = *lang_undo.borrow();
            let undone = controller_undo.borrow_mut().undo_last_move();
            if !undone { return; }

            let Some(win) = window_undo.upgrade() else { return };

            // Stop any engine player currently thinking: without this reset,
            // a `bestmove` computed on the old position could be
            // applied after the fact onto the position truncated by the undo.
            // Button normally hidden outside H vs H, but this stays safe
            // regardless of mode / any future UI change.
            game_bridge_undo.borrow_mut().reset();
            win.set_engine_playing(false);

            // Stop the ongoing analysis
            analysis_undo.borrow_mut().stop();
            win.set_engine_thinking(false);
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv(String::new().into());
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);

            // Refresh the game state
            refresh_game_state(&win, &controller_undo.borrow(), lang);
        });
    }

    // ── FEN: copy / paste ───────────────────────────────────────────────────────
    {
        let controller_copy_fen = controller.clone();

        window.on_copy_fen(move || {
            let fen = controller_copy_fen.borrow().current_fen();
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(&fen) {
                        eprintln!("[FEN copy] Erreur presse-papier : {e}");
                    }
                }
                Err(e) => eprintln!("[FEN copy] Impossible d'accéder au presse-papier : {e}"),
            }
        });
    }

    {
        let window_paste_fen  = window.as_weak();
        let controller_paste  = controller.clone();
        let analysis_paste    = analysis.clone();
        let game_bridge_paste = game_bridge.clone();
        let clock_paste       = chess_clock.clone();
        let lang_paste        = lang_cell.clone();

        window.on_paste_fen(move || {
            let lang = *lang_paste.borrow();

            // Read the clipboard
            let fen_text = match arboard::Clipboard::new()
                .and_then(|mut cb| cb.get_text())
            {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("[FEN paste] Erreur lecture presse-papier : {e}");
                    return;
                }
            };

            // 1. Validate the FEN without modifying the state
            if !GameController::is_valid_fen(&fen_text) {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.invalid_fen_title"))
                    .set_description(
                        i18n::translate("dialog.invalid_fen_desc").replace(
                            "{fen}",
                            &fen_text.chars().take(120).collect::<String>(),
                        ),
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                return;
            }

            // 2. Ask for confirmation before overwriting the current game
            let confirmed = rfd::MessageDialog::new()
                .set_title(i18n::translate("dialog.load_fen_confirm_title"))
                .set_description(i18n::translate("dialog.load_fen_confirm_desc"))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show() == rfd::MessageDialogResult::Yes;

            if !confirmed { return; }

            // 3. Load the FEN (valid, confirmed)
            controller_paste.borrow_mut().load_from_fen(&fen_text);

            let Some(win) = window_paste_fen.upgrade() else { return };

            // Stop the analysis and the engine player
            analysis_paste.borrow_mut().stop();
            game_bridge_paste.borrow_mut().reset();
            *clock_paste.borrow_mut() = ChessClock::new(&TimeControl::Infinite);
            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv(String::new().into());
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_is_game_over(false);
            win.set_hint_arrow_path(String::new().into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            // Implicit H vs H mode on a FEN paste (no engine)
            win.set_current_game_mode(0);

            refresh_game_state(&win, &controller_paste.borrow(), lang);
        });
    }

    // ── Position editor (Phase 12) ────────────────────────────────────────────
    // 12-A. Open the editor ────────────────────────────────────────────────────
    {
        let window_ed_open  = window.as_weak();
        let controller_open = controller.clone();

        window.on_open_position_editor(move || {
            let Some(win) = window_ed_open.upgrade() else { return };

            // Build the squares from the current position, without game overlays
            let mut squares = controller_open.borrow().build_squares();
            for sq in &mut squares {
                sq.is_selected     = false;
                sq.is_legal_target = false;
                sq.is_last_from    = false;
                sq.is_last_to      = false;
            }
            win.set_editor_squares(ModelRc::new(VecModel::from(squares)));

            // Extract the options from the current FEN
            let fen = controller_open.borrow().current_fen();
            let parts: Vec<&str> = fen.split_whitespace().collect();
            let side     = parts.get(1).copied().unwrap_or("w");
            let castling = parts.get(2).copied().unwrap_or("-");

            win.set_editor_white_to_move(side == "w");
            win.set_editor_castle_wk(castling.contains('K'));
            win.set_editor_castle_wq(castling.contains('Q'));
            win.set_editor_castle_bk(castling.contains('k'));
            win.set_editor_castle_bq(castling.contains('q'));

            win.set_editor_piece_id("".into());
            win.set_show_position_editor(true);
        });
    }

    // 12-B. Click on an editor square ─────────────────────────────────────────
    {
        let window_ed_sq = window.as_weak();

        window.on_editor_square_clicked(move |row, col| {
            let Some(win) = window_ed_sq.upgrade() else { return };
            let piece_id = win.get_editor_piece_id().to_string();
            if piece_id.is_empty() { return; }  // no piece selected

            // ── Chess rules (real-time checks) ────────────────────────────────

            // Pawns cannot be placed on ranks 1 and 8
            // (row 0 = rank 8, row 7 = rank 1 in the Slint convention)
            if (piece_id == "wP" || piece_id == "bP") && (row == 0 || row == 7) {
                return;
            }

            // Read the current model
            let model = win.get_editor_squares();
            let mut squares: Vec<gui::SquareData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();

            // Only one king per color: placing a king removes the existing one
            if piece_id == "wK" || piece_id == "bK" {
                for sq in &mut squares {
                    if sq.piece_char.as_str() == piece_id.as_str() {
                        sq.piece_char = slint::SharedString::from("");
                        sq.piece_side = 0;
                    }
                }
            }

            // Two kings cannot be adjacent (Chebyshev distance ≤ 1)
            if piece_id == "wK" || piece_id == "bK" {
                let enemy_king = if piece_id == "wK" { "bK" } else { "wK" };
                let adjacent = squares.iter().any(|sq| {
                    sq.piece_char.as_str() == enemy_king
                        && (sq.row - row).abs() <= 1
                        && (sq.col - col).abs() <= 1
                });
                if adjacent { return; }
            }

            // ── Piece limits (exact promotion constraint) ─────────────────────
            // Checked after any same-color king has been removed.
            //
            // Mathematical chess constraint:
            //   promotions_used + pawns_remaining ≤ 8
            // where promotions_used = Σ max(0, count(type) − count_initial(type))
            // with: Q_init=1, R_init=2, B_init=2, N_init=2.
            //
            // This formula correctly covers all cases (10 legal bishops if
            // 0 pawns remain, but 10 bishops + 5 rooks impossible: 8+3 > 8).
            if piece_id != "eraser" && piece_id != "wK" && piece_id != "bK" {
                let color_prefix = if piece_id.starts_with('w') { "w" } else { "b" };
                let idx_target   = (row * 8 + col) as usize;
                let pawn_id      = format!("{color_prefix}P");

                // Piece currently on the target square (empty → "")
                let target_piece: String = squares
                    .get(idx_target)
                    .map(|sq| sq.piece_char.as_str().to_string())
                    .unwrap_or_default();
                let target_same_color = !target_piece.is_empty()
                    && target_piece.starts_with(color_prefix);

                // Fast count of a piece identifier
                let count_id = |pid: &str| -> i32 {
                    squares.iter()
                        .filter(|sq| sq.piece_char.as_str() == pid)
                        .count() as i32
                };

                if piece_id == pawn_id.as_str() {
                    // ── Pawns: max 8 per color ────────────────────────────────
                    // Same-color pawn→pawn replacement: count unchanged → OK
                    let replacing_same_pawn =
                        target_same_color && target_piece == pawn_id;
                    if !replacing_same_pawn && count_id(&pawn_id) >= 8 { return; }
                } else {
                    // ── Q / R / B / N: promotion budget ───────────────────────
                    // Initial count of each type (excluding king and pawns)
                    let initial = |kind: &str| -> i32 {
                        match kind { "Q" => 1, "R" | "B" | "N" => 2, _ => 0 }
                    };
                    let new_kind = &piece_id[1..]; // "Q", "R", "B" or "N"

                    // Simulate the placement: remove the existing piece if same color,
                    // then add the new piece — then compute the excess.
                    let simulated_excess: i32 = ["Q", "R", "B", "N"].iter().map(|&k| {
                        let fid = format!("{color_prefix}{k}");
                        let mut n = count_id(&fid);
                        // Remove the piece on the target square (if same color)
                        if target_same_color && target_piece.len() > 1
                            && &target_piece[1..] == k { n -= 1; }
                        // Add the new piece
                        if k == new_kind { n += 1; }
                        (n - initial(k)).max(0)
                    }).sum();

                    let pawns_on_board = count_id(&pawn_id);
                    if simulated_excess + pawns_on_board > 8 { return; }
                }
            }

            // Modify the clicked square
            let idx = (row * 8 + col) as usize;
            if let Some(sq) = squares.get_mut(idx) {
                if piece_id == "eraser" {
                    sq.piece_char = slint::SharedString::from("");
                    sq.piece_side = 0;
                } else {
                    sq.piece_char = slint::SharedString::from(piece_id.as_str());
                    sq.piece_side = if piece_id.starts_with('w') { 1 } else { 2 };
                }
            }
            win.set_editor_squares(ModelRc::new(VecModel::from(squares)));
        });
    }

    // 12-B2. Clear the whole board ────────────────────────────────────────────
    {
        let window_ed_clr = window.as_weak();

        window.on_editor_clear_board(move || {
            let Some(win) = window_ed_clr.upgrade() else { return };
            let model = win.get_editor_squares();
            let mut squares: Vec<gui::SquareData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            for sq in &mut squares {
                sq.piece_char = slint::SharedString::from("");
                sq.piece_side = 0;
            }
            win.set_editor_squares(ModelRc::new(VecModel::from(squares)));
        });
    }

    // 12-C. Reset to the starting position ─────────────────────────────────────
    {
        let window_ed_rst = window.as_weak();

        window.on_editor_reset_position(move || {
            let Some(win) = window_ed_rst.upgrade() else { return };

            // Starting position: create a temporary controller
            let squares = GameController::new().build_squares();
            win.set_editor_squares(ModelRc::new(VecModel::from(squares)));
            win.set_editor_white_to_move(true);
            win.set_editor_castle_wk(true);
            win.set_editor_castle_wq(true);
            win.set_editor_castle_bk(true);
            win.set_editor_castle_bq(true);
        });
    }

    // 12-C2. Export the editor's board as a PNG image (ergonomics follow-up
    // 10/07/2026 — 3 buttons "identical (same principle)" to those of the
    // main window, applied to the position currently being built
    // in the editor, not to any game currently in progress) ─────────────────
    {
        let window_ed_png = window.as_weak();

        window.on_editor_export_board_png(move || {
            let Some(win) = window_ed_png.upgrade() else { return };

            let model = win.get_editor_squares();
            let squares: Vec<gui::SquareData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            let fen = build_editor_fen(
                &squares,
                win.get_editor_white_to_move(),
                win.get_editor_castle_wk(),
                win.get_editor_castle_wq(),
                win.get_editor_castle_bk(),
                win.get_editor_castle_bq(),
            );

            // No captured pieces at this stage (a simple position built
            // by hand, not a game that was played) — empty strips, as for
            // any position outside a game's context.
            let png_bytes =
                png_export::build_board_png_bytes(&fen, win.get_board_flipped(), &[], &[]);

            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_png_title"))
                .add_filter(i18n::translate("dialog.png_filter_label"), &["png"])
                .set_file_name("echiquier.png")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, &png_bytes) {
                    eprintln!("[PNG export - éditeur] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_png_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // 12-C3. Copy the editor's FEN ─────────────────────────────────────────────
    {
        let window_ed_copy = window.as_weak();

        window.on_editor_copy_fen(move || {
            let Some(win) = window_ed_copy.upgrade() else { return };

            let model = win.get_editor_squares();
            let squares: Vec<gui::SquareData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            let fen = build_editor_fen(
                &squares,
                win.get_editor_white_to_move(),
                win.get_editor_castle_wk(),
                win.get_editor_castle_wq(),
                win.get_editor_castle_bk(),
                win.get_editor_castle_bq(),
            );

            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(&fen) {
                        eprintln!("[FEN copy - éditeur] Erreur presse-papier : {e}");
                    }
                }
                Err(e) => eprintln!("[FEN copy - éditeur] Impossible d'accéder au presse-papier : {e}"),
            }
        });
    }

    // 12-C4. Paste a FEN into the editor ───────────────────────────────────────
    // Unlike the main board's "paste FEN" (`on_paste_fen`),
    // no confirmation is requested before overwriting the position being
    // built: the editor has no game/history to lose, only an
    // arrangement of pieces under construction — replacing it
    // via paste is exactly the button's expected behavior, like
    // "Effacer tout" or "Position initiale" just above, which also don't
    // ask for one.
    {
        let window_ed_paste = window.as_weak();

        window.on_editor_paste_fen(move || {
            let Some(win) = window_ed_paste.upgrade() else { return };

            let fen_text = match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
                Ok(text) => text,
                Err(e) => {
                    eprintln!("[FEN paste - éditeur] Erreur lecture presse-papier : {e}");
                    return;
                }
            };

            if !GameController::is_valid_fen(&fen_text) {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.invalid_fen_title"))
                    .set_description(
                        i18n::translate("dialog.invalid_fen_desc").replace(
                            "{fen}",
                            &fen_text.chars().take(120).collect::<String>(),
                        ),
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                return;
            }

            let Ok(pos) = chess_core::types::Position::from_fen(&fen_text) else { return; };
            let squares = game_controller::build_static_squares(&pos, None);

            win.set_editor_squares(ModelRc::new(VecModel::from(squares)));
            win.set_editor_white_to_move(pos.side_to_move == Color::White);
            win.set_editor_castle_wk(pos.castling.white_kingside);
            win.set_editor_castle_wq(pos.castling.white_queenside);
            win.set_editor_castle_bk(pos.castling.black_kingside);
            win.set_editor_castle_bq(pos.castling.black_queenside);
        });
    }

    // 12-D. Validate the position and start the game ──────────────────────────
    {
        let window_ed_val    = window.as_weak();
        let controller_val   = controller.clone();
        let analysis_val     = analysis.clone();
        let game_bridge_val  = game_bridge.clone();
        let clock_val        = chess_clock.clone();
        let score_hist_val   = score_history.clone();
        let lang_val         = lang_cell.clone();
        let book_white_val   = book_white.clone();
        let book_black_val   = book_black.clone();

        window.on_editor_validate(move || {
            let Some(win) = window_ed_val.upgrade() else { return };

            // ── 1. Build the FEN from the editor's state ──────────────────────
            let model = win.get_editor_squares();
            let squares: Vec<gui::SquareData> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();

            let fen = build_editor_fen(
                &squares,
                win.get_editor_white_to_move(),
                win.get_editor_castle_wk(),
                win.get_editor_castle_wq(),
                win.get_editor_castle_bk(),
                win.get_editor_castle_bq(),
            );

            // ── 2. Validate the FEN (chess_core) ──────────────────────────────

            // Pre-check: count the kings to give a precise message
            let wk = squares.iter().filter(|sq| sq.piece_char.as_str() == "wK").count();
            let bk = squares.iter().filter(|sq| sq.piece_char.as_str() == "bK").count();
            if wk != 1 {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.invalid_position_title"))
                    .set_description(
                        i18n::translate("dialog.need_one_white_king_desc")
                            .replace("{n}", &wk.to_string()),
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                return;
            }
            if bk != 1 {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.invalid_position_title"))
                    .set_description(
                        i18n::translate("dialog.need_one_black_king_desc")
                            .replace("{n}", &bk.to_string()),
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                return;
            }

            // General validation (illegal check, inconsistent castling rights, etc.)
            if !GameController::is_valid_fen(&fen) {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.invalid_position_title"))
                    .set_description(i18n::translate("dialog.illegal_position_desc"))
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
                return;
            }

            // ── 3. Wizard mode: return the FEN to the wizard ──────────────────
            // If the editor was opened from the wizard, the FEN is stored in
            // wizard-start-fen and control returns to the wizard without starting a game.
            if win.get_editor_from_wizard() {
                win.set_wizard_start_fen(fen.clone().into());
                // Ergonomics bugfix 09/07/2026: the position comes from the editor,
                // not the game database — disables the attribution to the other
                // source so the correct button shows as active at step 1.
                win.set_wizard_start_fen_from_base(false);
                win.set_editor_from_wizard(false);
                win.set_show_position_editor(false);
                win.set_editor_piece_id("".into());
                win.set_wizard_step(1);   // direct return to the config step (not step 0 mode)
                win.set_show_setup_wizard(true);
                return;
            }

            // ── 4. Load the FEN into the controller ───────────────────────────
            controller_val.borrow_mut().load_from_fen(&fen);

            // ── 5. Reset the game state ────────────────────────────────────────
            // Keep the active mode (H vs M / M vs M) — do NOT force H vs H.
            let prev_mode = win.get_current_game_mode();

            analysis_val.borrow_mut().stop();
            game_bridge_val.borrow_mut().reset();   // Stops the engine-player threads
            *clock_val.borrow_mut() = ChessClock::new(&TimeControl::Infinite);

            score_hist_val.borrow_mut().clear();

            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv(String::new().into());
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_is_game_over(false);
            win.set_hint_arrow_path(String::new().into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            // set_current_game_mode intentionally absent → prev_mode is kept
            win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
            win.set_white_curve_path(String::new().into());
            win.set_black_curve_path(String::new().into());

            let lang = *lang_val.borrow();
            refresh_game_state(&win, &controller_val.borrow(), lang);

            // ── 6. Restart the engine player if needed ────────────────────────
            // If we were in H vs M or M vs M, reset the bridge from the
            // persisted config and trigger the engine if it's its turn.
            if prev_mode != 0 {
                let saved_cfg = gc_persist::load_last_mode()
                    .and_then(gc_persist::load_last_config);
                if let Some(ref cfg) = saved_cfg {
                    game_bridge_val.borrow_mut().init(cfg, &win.as_weak());
                    if !controller_val.borrow().is_over() {
                        // PHASE 15: book move(s) before consulting the engine.
                        if try_play_book_moves(&win, &controller_val, &clock_val, &game_bridge_val, &book_white_val, &book_black_val) {
                            refresh_game_state(&win, &controller_val.borrow(), lang);
                        }
                        if !controller_val.borrow().is_over() {
                            let is_white_turn = controller_val.borrow().is_white_turn();
                            let fen_play      = controller_val.borrow().current_fen();
                            let _ = game_bridge_val.borrow()
                                .trigger_if_engine_turn(is_white_turn, fen_play, None);
                        }
                    }
                }
            }

            // ── 7. Close the editor ─────────────────────────────────────────────
            win.set_show_position_editor(false);
            win.set_editor_piece_id("".into());
        });
    }

    // ── PGN: export ─────────────────────────────────────────────────────────────
    {
        let window_pgn_exp     = window.as_weak();
        let controller_pgn_exp = controller.clone();

        window.on_export_pgn(move || {
            let Some(win) = window_pgn_exp.upgrade() else { return; };
            let white = win.get_white_player_name().to_string();
            let black = win.get_black_player_name().to_string();
            let pgn   = controller_pgn_exp.borrow().export_pgn(&white, &black);
            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_pgn_title"))
                .add_filter(i18n::translate("dialog.pgn_filter_label"), &["pgn"])
                .set_file_name("partie.pgn")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, pgn.as_bytes()) {
                    eprintln!("[PGN export] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_pgn_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── PDF: printing the game (PHASE 25) ────────────────────────────────────────
    {
        let window_print     = window.as_weak();
        let controller_print = controller.clone();

        window.on_print_game(move || {
            let Some(win) = window_print.upgrade() else { return; };
            let ctrl = controller_print.borrow();

            // Always the game's final position (independent of
            // history navigation on screen) and the same data
            // source as the displayed moves panel — see the decisions
            // made, PHASE 25.
            let final_fen = ctrl.current_fen();
            let moves     = ctrl.build_move_rows();

            // Time control: reloaded from the current mode's persisted config,
            // same pattern as resuming a game (see the "New
            // Game" section above) — no new source of truth.
            let time_control_label = gc_persist::load_last_mode()
                .and_then(gc_persist::load_last_config)
                .map_or_else(|| "—".to_owned(), |cfg| cfg.time_control.label());

            let info = pdf_export::PrintGameInfo {
                white_name: win.get_white_player_name().to_string(),
                black_name: win.get_black_player_name().to_string(),
                date: pdf_export::today_date_string(),
                time_control_label,
                result: ctrl.result_pgn().map(str::to_owned),
            };
            drop(ctrl);

            let pdf_bytes = pdf_export::build_pdf_bytes(&final_fen, &moves, &info);

            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_pdf_title"))
                .add_filter(i18n::translate("dialog.pdf_filter_label"), &["pdf"])
                .set_file_name("partie.pdf")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, &pdf_bytes) {
                    eprintln!("[PDF export] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_pdf_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── PNG: exporting the displayed board (PHASE 76) ─────────────────────────────
    {
        let window_png_exp     = window.as_weak();
        let controller_png_exp = controller.clone();

        window.on_export_board_png(move || {
            let Some(win) = window_png_exp.upgrade() else { return; };
            let ctrl = controller_png_exp.borrow();

            // Unlike the PDF export: position AND orientation are a "snapshot"
            // of what is currently displayed on screen (respects
            // `viewed_ply` via `displayed_fen`, and the "Flip" button via
            // `board-flipped`) — see the decisions made, PHASE 76.
            let fen = ctrl.displayed_fen();
            let (captured_by_white, captured_by_black, _diff) = ctrl.captured_summary();
            drop(ctrl);

            let flipped = win.get_board_flipped();
            let png_bytes = png_export::build_board_png_bytes(
                &fen,
                flipped,
                &captured_by_white,
                &captured_by_black,
            );

            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_png_title"))
                .add_filter(i18n::translate("dialog.png_filter_label"), &["png"])
                .set_file_name("echiquier.png")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, &png_bytes) {
                    eprintln!("[PNG export] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_png_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── PGN: import ─────────────────────────────────────────────────────────────
    {
        let window_pgn_imp     = window.as_weak();
        let controller_pgn_imp = controller.clone();
        let analysis_pgn_imp   = analysis.clone();
        let game_bridge_pgn    = game_bridge.clone();
        let clock_pgn_imp      = chess_clock.clone();
        let score_hist_pgn     = score_history.clone();
        let lang_pgn_imp       = lang_cell.clone();

        window.on_import_pgn(move || {
            // 1. Pick the file
            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.open_pgn_title"))
                .add_filter(i18n::translate("dialog.pgn_filter_label"), &["pgn"])
                .pick_file()
            else { return; };

            // 2. Read the content
            let Ok(content) = std::fs::read_to_string(&path) else { return; };

            // 3. Load into the controller (silently abandoned if the PGN is invalid)
            {
                let mut ctrl = controller_pgn_imp.borrow_mut();
                if ctrl.load_from_pgn(&content).is_err() { return; }
            }

            // 4. Stop the analysis and the engine player
            analysis_pgn_imp.borrow_mut().stop();
            game_bridge_pgn.borrow_mut().reset();

            // 5. Clock: untimed mode (replay)
            *clock_pgn_imp.borrow_mut() = ChessClock::new(&TimeControl::Infinite);

            // 6. Clear the score history
            score_hist_pgn.borrow_mut().clear();

            let lang = *lang_pgn_imp.borrow();

            let Some(win) = window_pgn_imp.upgrade() else { return; };
            win.set_show_promotion_modal(false);
            win.set_is_game_over(false);
            win.set_game_over_result(String::new().into());
            win.set_game_over_reason(String::new().into());
            win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
            win.set_white_curve_path(String::new().into());
            win.set_black_curve_path(String::new().into());
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv("—".into());
            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_show_clocks(false);
            win.set_white_clock_active(false);
            win.set_black_clock_active(false);
            win.set_white_clock_text("--:--".into());
            win.set_black_clock_text("--:--".into());
            win.set_hint_arrow_path("".into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            win.set_hint_computing(false);
            // H vs H mode: history navigation available
            win.set_current_game_mode(0);

            // 7. Refresh the board, the move list and the status
            let ctrl = controller_pgn_imp.borrow();
            refresh_game_state(&win, &ctrl, lang);
        });
    }

    window.on_new_game(move || {
        let lang = *lang_for_new.borrow();

        // Stop the analysis and the engine player
        analysis_for_new.borrow_mut().stop();
        game_bridge_for_new.borrow_mut().reset();

        // Reset the clock (no game time control)
        *chess_clock_new.borrow_mut() = ChessClock::new(&TimeControl::Infinite);

        // Reset the controller
        let mut ctrl = controller_for_new.borrow_mut();
        ctrl.reset();

        // Clear the score history and the curves
        {
            let mut hist = score_history_new.borrow_mut();
            hist.clear();
        }

        if let Some(win) = window_weak_new.upgrade() {
            // Close the modal and the banner if open
            win.set_show_promotion_modal(false);
            win.set_is_game_over(false);
            win.set_game_over_result(String::new().into());
            win.set_game_over_reason(String::new().into());
            win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
            win.set_white_curve_path(String::new().into());
            win.set_black_curve_path(String::new().into());
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv("—".into());
            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            // Clock: hide and reset the display
            win.set_show_clocks(false);
            win.set_white_clock_active(false);
            win.set_black_clock_active(false);
            win.set_white_clock_text("--:--".into());
            win.set_black_clock_text("--:--".into());
            win.set_hint_arrow_path("".into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            win.set_hint_computing(false);

            refresh_game_state(&win, &ctrl, lang);
        }
    });

    // 12. Callback: language chosen on first launch
    //     → save, apply, close the screen, open the wizard or recap
    let window_weak_lang_setup    = window.as_weak();
    let controller_for_lang_setup = controller.clone();
    let lang_for_lang_setup       = lang_cell.clone();

    window.on_language_chosen(move |lang_code| {
        let lang = parse_lang_code(lang_code.as_str());
        *lang_for_lang_setup.borrow_mut() = lang;
        i18n::init(lang);
        prefs::save_lang(lang_code.as_str());

        let Some(win) = window_weak_lang_setup.upgrade() else { return; };
        i18n_bridge::apply_translations(&win.global::<Tr>(), lang);
        win.set_current_lang_index(lang.ui_index());
        {
            let ctrl = controller_for_lang_setup.borrow();
            win.set_status_text(i18n::translate(ctrl.status_key()).into());
        }
        win.set_show_language_setup(false);
        win.invoke_open_new_game();
    });

    // 13. Callback: language changed from the Preferences panel
    //     → save + apply (the overlay closes via Slint)
    let window_weak_pref_lang    = window.as_weak();
    let controller_for_pref_lang = controller.clone();
    let lang_for_pref_lang       = lang_cell.clone();

    window.on_pref_language_chosen(move |lang_code| {
        let lang = parse_lang_code(lang_code.as_str());
        *lang_for_pref_lang.borrow_mut() = lang;
        i18n::init(lang);
        prefs::save_lang(lang_code.as_str());

        let Some(win) = window_weak_pref_lang.upgrade() else { return; };
        i18n_bridge::apply_translations(&win.global::<Tr>(), lang);
        win.set_current_lang_index(lang.ui_index());
        let ctrl = controller_for_pref_lang.borrow();
        win.set_status_text(i18n::translate(ctrl.status_key()).into());
        if ctrl.is_over() {
            win.set_game_over_result(i18n::translate_in(lang, ctrl.status_key()).into());
            win.set_game_over_reason(i18n::translate_in(lang, ctrl.end_reason_key()).into());
        }
    });

    // 14. Callback: add a UCI engine via the file dialog (Préférences → Moteurs)
    //     rfd::FileDialog blocks the current (main) thread until a selection is made.
    //     On macOS, NSOpenPanel creates its own modal run loop — this is safe.
    let engine_list_add  = engine_list.clone();
    let window_weak_add  = window.as_weak();
    let analysis_for_add = analysis.clone();
    let hint_path_add    = hint_engine_path.clone();

    window.on_browse_add_engine(move || {
        let result = rfd::FileDialog::new()
            .set_title(i18n::translate("dialog.select_engine_title"))
            .pick_file();
        if let Some(path) = result {
            let path_str = path.to_string_lossy().into_owned();

            // ── UCI validation ──────────────────────────────────────────────────
            // Attempts a "uci / uciok" handshake with a 5 s timeout.
            // Briefly blocks the main thread — acceptable since the user
            // just closed a native dialog and expects a check to happen.
            {
                use uci::engine::UciEngine;
                use std::time::Duration;
                match UciEngine::connect_with_timeout(&path_str, Duration::from_secs(5)) {
                    Ok(engine) => engine.quit(), // handshake OK → releases the process
                    Err(e) => {
                        rfd::MessageDialog::new()
                            .set_title(i18n::translate("dialog.invalid_executable_title"))
                            .set_description(
                                i18n::translate("dialog.invalid_executable_desc")
                                    .replace("{err}", &e.to_string()),
                            )
                            .set_level(rfd::MessageLevel::Warning)
                            .show();
                        return;
                    }
                }
            }

            // ── Import into moteurs/ (PHASE 24, Step 5) ──────────────────────────
            // Copies the validated file into the application's moteurs/ folder
            // (unless it's already there — re-adding an already-imported engine),
            // with automatic renaming on a name collision. The Unix
            // execute bit is reapplied on every launch anyway
            // by `EngineProcess::spawn` (see crates/uci/src/process.rs), so
            // no need to redo it here — an immediate chmod after copying would be
            // redundant.
            let moteurs_dir = app_paths::moteurs_dir();
            let final_path: String = if path.parent() == Some(moteurs_dir.as_path()) {
                path_str.clone()
            } else {
                match app_paths::copy_with_auto_rename(&path, &moteurs_dir) {
                    Ok(copied) => copied.to_string_lossy().into_owned(),
                    Err(e) => {
                        rfd::MessageDialog::new()
                            .set_title(i18n::translate("dialog.import_impossible_title"))
                            .set_description(
                                i18n::translate("dialog.engine_import_failed_desc")
                                    .replace("{err}", &e.to_string()),
                            )
                            .set_level(rfd::MessageLevel::Warning)
                            .show();
                        return;
                    }
                }
            };

            // Display name = file name without extension
            let name = std::path::Path::new(&final_path)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("moteur")
                .to_owned();
            let engine = prefs::SavedEngine {
                name,
                path: final_path,
                options: std::collections::HashMap::new(),
            };
            // PHASE 80 — user request: if the list is empty and
            // no hint engine is currently defined, this first
            // added engine automatically becomes the hint engine — without
            // this, Assist mode stays silently disabled until
            // the user goes and explicitly chooses one from the
            // Preferences dropdown (the same discoverability blind spot
            // fixed in PHASE 79). The choice is persisted as a real
            // explicit setting (`prefs::save_hint_engine`), not a silent
            // fallback — see the PHASE 80 discussion in SUIVI_PLAN_ACTION.md.
            let mut auto_hint_path: Option<String> = None;
            {
                let mut engines = engine_list_add.borrow_mut();
                // Avoid duplicates (same path). PHASE 74 — comparison
                // via `Path`: `engine.path` (final_path) is built by a
                // native join (`copy_with_auto_rename`/native dialog),
                // `e.path` may contain a mixed separator after
                // resolving a multi-component relative path —
                // the same class of bug as the PHASE 71 fix.
                if !engines.iter().any(|e| std::path::Path::new(&e.path) == std::path::Path::new(&engine.path)) {
                    let was_empty = engines.is_empty();
                    if was_empty && hint_path_add.borrow().is_none() {
                        auto_hint_path = Some(engine.path.clone());
                    }
                    engines.push(engine);
                    prefs::save_engines(&engines);
                }
            }
            if let Some(path) = auto_hint_path {
                prefs::save_hint_engine(Some(path.as_str()));
                *hint_path_add.borrow_mut() = Some(path);
            }
            // Resynchronize the analysis engine after adding (Phase 11.3).
            let engine_name = sync_analysis_engine(
                &mut analysis_for_add.borrow_mut(),
                hint_path_add.borrow().as_ref(),
                &engine_list_add.borrow(),
            );
            if let Some(win) = window_weak_add.upgrade() {
                update_engines_in_window(&win, &engine_list_add.borrow());
                win.set_analysis_engine_name(engine_name.into());
                win.set_hint_engine_path(
                    hint_engine_path_for_window(hint_path_add.borrow().as_deref()).into()
                );
                win.set_hint_engine_name(
                    hint_engine_display_name(hint_path_add.borrow().as_deref(), &engine_list_add.borrow()).into()
                );
            }
        }
    });

    // 15. Callback: remove an engine from the list (Préférences → Moteurs)
    let engine_list_rm   = engine_list.clone();
    let window_weak_rm   = window.as_weak();
    let analysis_for_rm  = analysis.clone();
    let hint_path_rm     = hint_engine_path.clone();

    window.on_remove_engine(move |idx| {
        let idx = idx as usize;

        // Name + path read BEFORE any removal (read-only).
        let engine_info: Option<(String, String)> = {
            let engines = engine_list_rm.borrow();
            engines.get(idx).map(|e| (e.name.clone(), e.path.clone()))
        };
        let Some((engine_name_disp, engine_path)) = engine_info else { return; };

        // PHASE 64 — reported by the user: removing an engine from the
        // LIST alone does not prevent it from reappearing on the next startup,
        // since the automatic scan (`scan_timer` above) redetects any
        // executable present in `moteurs/` that is no longer in the
        // saved list. So the user is explicitly asked whether the file itself
        // should be deleted from disk; otherwise they are warned it will be
        // redetected. Either way, the engine is removed from the list
        // (already-existing behavior, never called into question by this choice).
        let delete_from_disk = rfd::MessageDialog::new()
            .set_title(i18n::translate("dialog.remove_engine_confirm_title"))
            .set_description(
                i18n::translate("dialog.remove_engine_confirm_desc")
                    .replace("{name}", &engine_name_disp),
            )
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes;

        if delete_from_disk {
            if let Err(e) = std::fs::remove_file(&engine_path) {
                rfd::MessageDialog::new()
                    .set_title(i18n::translate("dialog.generic_error_title"))
                    .set_description(
                        i18n::translate("dialog.engine_delete_failed_desc")
                            .replace("{err}", &e.to_string()),
                    )
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                // Disk deletion failed (permissions, file already
                // missing...): the engine is still removed from the list,
                // to stay consistent with what the interface already shows.
            }
        }

        {
            let mut engines = engine_list_rm.borrow_mut();
            if idx < engines.len() {
                engines.remove(idx);
                prefs::save_engines(&engines);
            }
        }

        // PHASE 65 — reported by the user: if the removed engine was
        // precisely the selected "hint engine", it stayed
        // referenced by its old (now nonexistent) path. This is compared
        // BEFORE anything else against the path that was just removed (read
        // above, before the removal).
        //
        // PHASE 80 — user request: rather than always falling back to
        // "None", fall back to the first remaining engine if one
        // exists (the same "first saved engine" priority as
        // `sync_analysis_engine`/PHASE 80, applied here to the hint engine
        // itself); only if the list is now empty does it fall back
        // to "None". Either way, the new choice is persisted
        // explicitly (`prefs::save_hint_engine`), not a silent fallback
        // — see the PHASE 80 discussion in SUIVI_PLAN_ACTION.md.
        let hint_was_cleared = hint_path_rm.borrow().as_deref() == Some(engine_path.as_str());
        if hint_was_cleared {
            let fallback_path = engine_list_rm.borrow().first().map(|e| e.path.clone());
            prefs::save_hint_engine(fallback_path.as_deref());
            *hint_path_rm.borrow_mut() = fallback_path;
        }

        // Resynchronize the analysis engine after removal (Phase 11.3).
        let engine_name = sync_analysis_engine(
            &mut analysis_for_rm.borrow_mut(),
            hint_path_rm.borrow().as_ref(),
            &engine_list_rm.borrow(),
        );
        if let Some(win) = window_weak_rm.upgrade() {
            update_engines_in_window(&win, &engine_list_rm.borrow());
            win.set_analysis_engine_name(engine_name.into());
            if hint_was_cleared {
                win.set_hint_engine_path(
                    hint_engine_path_for_window(hint_path_rm.borrow().as_deref()).into()
                );
            }
            win.set_hint_engine_name(
                hint_engine_display_name(hint_path_rm.borrow().as_deref(), &engine_list_rm.borrow()).into()
            );
        }
    });

    // PGN content pending for the wizard's next "Démarrer" ("" = starting position).
    let pending_pgn: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // 16. Callback: "Nouvelle partie" (header / panel / banner) → wizard or recap
    let window_weak_open  = window.as_weak();
    let engine_list_open  = engine_list.clone();
    let pending_pgn_open  = pending_pgn.clone();

    window.on_open_new_game(move || {
        let Some(win) = window_weak_open.upgrade() else { return; };

        // Look up the last saved config
        if let Some(mode) = gc_persist::load_last_mode() {
            if let Some(cfg) = gc_persist::load_last_config(mode) {
                // Recap lines (i18n: PHASE 34, i18n audit batch 2)
                let mode_prefix = i18n::translate("wizard.recap_mode_prefix");
                let line1 = match cfg.mode {
                    GameMode::HumanVsHuman   => format!("{mode_prefix}{}", i18n::translate("wizard.mode_human_vs_human")),
                    GameMode::HumanVsEngine  => format!("{mode_prefix}{}", i18n::translate("wizard.mode_human_vs_engine")),
                    GameMode::EngineVsEngine => format!("{mode_prefix}{}", i18n::translate("wizard.mode_engine_vs_engine")),
                };
                let line2 = match cfg.mode {
                    GameMode::HumanVsHuman   => i18n::translate("wizard.recap_no_engine"),
                    GameMode::HumanVsEngine  => {
                        let c = match cfg.human_color {
                            HumanColor::White  => i18n::translate("board.white"),
                            HumanColor::Black  => i18n::translate("board.black"),
                            HumanColor::Random => i18n::translate("wizard.color_random"),
                        };
                        format!("{}{c}", i18n::translate("wizard.recap_human_prefix"))
                    }
                    GameMode::EngineVsEngine => i18n::translate("wizard.recap_two_engines"),
                };
                let line3 = {
                    let engine = cfg.black_engine.as_ref().or(cfg.white_engine.as_ref());
                    let tc_str = cfg.time_control.label();
                    if let Some(e) = engine {
                        let engine_lvl = e.time_control.label();
                        let name = std::path::Path::new(&e.path)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(e.path.as_str());
                        format!("{name}  ({engine_lvl})  ·  {tc_str}")
                    } else {
                        tc_str  // H vs H: only show the time control
                    }
                };
                win.set_recap_line1(line1.into());
                win.set_recap_line2(line2.into());
                win.set_recap_line3(line3.into());
                win.set_show_setup_recap(true);
                return;
            }
        }
        // No config → open the wizard.
        // Pre-select the first remembered engine (if available).
        let first_path = engine_list_open.borrow()
            .first()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        win.set_wizard_engine_path_w(first_path.clone().into());
        win.set_wizard_engine_path_b(first_path.into());
        // Always start over with no PGN pre-loaded when the wizard opens
        *pending_pgn_open.borrow_mut() = String::new();
        win.set_wizard_pgn_filename("".into());
        win.set_wizard_start_fen("".into());
        win.set_wizard_start_fen_from_base(false);
        win.set_wizard_step(0);
        win.set_show_setup_wizard(true);
    });

    // 16bis. Callbacks of the "Détail de la partie" screen (ergonomics bugfix
    // 09/07/2026, user feedback) — replaces the old
    // `on_replay_from_detail_move` (PHASE 82, step 10), which used to start
    // a game directly on clicking a move: a source of confusion
    // explicitly reported by the user. Split into two callbacks
    // with distinct responsibilities:
    //
    // - `game-detail-ply-selected`: clicking a move (or navigating via the
    //   "<< < > >>" buttons) ONLY updates the preview (static
    //   board, see `game_controller::build_static_squares`) — never a
    //   game started, regardless of the screen's entry point.
    // - `game-detail-start-from-here`: wired/visible only when
    //   the screen was opened from the 3rd button of step 1 of the wizard
    //   ("Depuis la base de parties…", `reference-browser-from-wizard`,
    //   see `app.slint::open-reference-browser-wizard`) — the wizard is
    //   still open UNDERNEATH (never closed): this just
    //   fills in `wizard-start-fen`/`wizard-start-fen-from-base` and
    //   closes both overlays (browser + detail), WITHOUT touching
    //   `wizard-step` nor recreating the wizard (which would lose the mode
    //   already chosen — `mode` is a private property of `GameSetupWizard`,
    //   never bound to `AppWindow`). From the pure browsing entry point
    //   ("Navigation Base"), this button simply doesn't exist on the Slint side —
    //   no way to start a game from this screen in that case.
    {
        let window_weak_ply = window.as_weak();
        window.on_game_detail_ply_selected(move |ply| {
            let Some(win) = window_weak_ply.upgrade() else { return; };

            let game_id = win.get_game_detail_id();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };
            let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };
            let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };
            let Some(pos) = game.position_at(ply as usize) else { return; };

            let last_move = if ply > 0 {
                game.history().records().get(ply as usize - 1).map(|r| (r.mv.from, r.mv.to))
            } else {
                None
            };
            let squares = game_controller::build_static_squares(&pos, last_move);

            win.set_game_detail_current_ply(ply);
            win.set_game_detail_board_squares(ModelRc::new(VecModel::from(squares)));
        });
    }
    {
        let window_weak_start = window.as_weak();
        window.on_game_detail_start_from_here(move |ply| {
            let Some(win) = window_weak_start.upgrade() else { return; };

            let game_id = win.get_game_detail_id();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };
            let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };
            let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };
            let Some(pos) = game.position_at(ply as usize) else { return; };
            let fen = pos.to_fen();

            // The wizard is already open underneath (never closed, see
            // `open-reference-browser-wizard`): neither `wizard_step`
            // nor `show_setup_wizard` is touched, to preserve the mode
            // already chosen.
            win.set_wizard_pgn_filename("".into());
            win.set_wizard_start_fen(fen.into());
            win.set_wizard_start_fen_from_base(true);
            win.set_show_game_detail(false);
            win.set_show_reference_browser(false);
            win.set_reference_browser_from_wizard(false);
        });
    }

    // ── Game detail: 4 buttons "identical (same principle)" to those ────────────
    //    of the main window (ergonomics follow-up 10/07/2026) — each acts on
    //    the PREVIEWED position (`game-detail-current-ply`), never on any
    //    game in progress. Same DB fetch + `position_at` pattern already
    //    established above (`on_game_detail_ply_selected`/`_start_from_here`).

    // Copy the FEN of the previewed position.
    {
        let window_weak_detail_fen = window.as_weak();
        window.on_game_detail_copy_fen(move || {
            let Some(win) = window_weak_detail_fen.upgrade() else { return; };

            let game_id = win.get_game_detail_id();
            let ply     = win.get_game_detail_current_ply();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };
            let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };
            let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };
            let Some(pos) = game.position_at(ply as usize) else { return; };
            let fen = pos.to_fen();

            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(&fen) {
                        eprintln!("[FEN copy - détail] Erreur presse-papier : {e}");
                    }
                }
                Err(e) => eprintln!("[FEN copy - détail] Impossible d'accéder au presse-papier : {e}"),
            }
        });
    }

    // Save the PGN — text already stored as-is in the database, no
    // reconstruction needed (unlike the main board's `on_export_pgn`,
    // which has to rebuild the PGN from the controller).
    {
        let window_weak_detail_pgn = window.as_weak();
        window.on_game_detail_export_pgn(move || {
            let Some(win) = window_weak_detail_pgn.upgrade() else { return; };

            let game_id = win.get_game_detail_id();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };
            let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };

            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_pgn_title"))
                .add_filter(i18n::translate("dialog.pgn_filter_label"), &["pgn"])
                .set_file_name("partie.pgn")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, row.pgn.as_bytes()) {
                    eprintln!("[PGN export - détail] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_pgn_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // Save the previewed board as a PNG image (position + captured
    // pieces up to this half-move + orientation passed as an argument by
    // Slint, see `game_detail.slint::board-flipped`).
    {
        let window_weak_detail_png = window.as_weak();
        window.on_game_detail_export_board_png(move |flipped| {
            let Some(win) = window_weak_detail_png.upgrade() else { return; };

            let game_id = win.get_game_detail_id();
            let ply     = win.get_game_detail_current_ply();
            let Ok(conn) = db::reference_schema::open_and_migrate(&current_reference_db_path(&win)) else { return; };
            let Ok(Some(row)) = db::repository::reference_game_repo::find_by_id(&conn, i64::from(game_id))
            else {
                return;
            };
            let Ok(game) = chess_core::pgn::import_pgn(&row.pgn) else { return; };
            let Some(pos) = game.position_at(ply as usize) else { return; };
            let fen = pos.to_fen();
            let (captured_by_white, captured_by_black, _diff) =
                game_controller::captured_summary_at_ply(&game, ply as usize);

            let png_bytes = png_export::build_board_png_bytes(
                &fen,
                flipped,
                &captured_by_white,
                &captured_by_black,
            );

            if let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.save_png_title"))
                .add_filter(i18n::translate("dialog.png_filter_label"), &["png"])
                .set_file_name("echiquier.png")
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, &png_bytes) {
                    eprintln!("[PNG export - détail] Erreur d'écriture : {e}");
                    rfd::MessageDialog::new()
                        .set_title(i18n::translate("dialog.save_error_title"))
                        .set_description(
                            i18n::translate("dialog.save_png_error_desc")
                                .replace("{err}", &e.to_string()),
                        )
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
            }
        });
    }

    // ── PGN wizard: file dialog to load a starting position ──────────────────────
    {
        let window_pgn_wiz   = window.as_weak();
        let pending_pgn_wiz  = pending_pgn.clone();

        window.on_wizard_pick_pgn(move || {
            let Some(win) = window_pgn_wiz.upgrade() else { return; };

            let Some(path) = rfd::FileDialog::new()
                .set_title(i18n::translate("dialog.wizard_open_pgn_title"))
                .add_filter(i18n::translate("dialog.pgn_filter_label"), &["pgn"])
                .pick_file()
            else { return; };

            let Ok(content) = std::fs::read_to_string(&path) else { return; };

            // Quick check: the PGN must be parsable
            if chess_core::pgn::import_pgn(&content).is_err() { return; }

            // Store the content and display the short name in the wizard
            *pending_pgn_wiz.borrow_mut() = content;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("partie.pgn")
                .to_owned();
            win.set_wizard_pgn_filename(filename.into());
        });
    }

    // 16b. Callbacks: MultiPV line-count change for ♔ and ♚
    {
        // Shared helper: applies an N change for a given side.
        // `for_white` = true → White side, false → Black side.
        let make_mpv_callback = |for_white: bool| {
            let window_mpv     = window.as_weak();
            let analysis_mpv   = analysis.clone();
            let controller_mpv = controller.clone();
            move |n: i32| {
                let Some(win) = window_mpv.upgrade() else { return; };
                if for_white { prefs::save_multipv_white(n); }
                else         { prefs::save_multipv_black(n); }

                // Active N = the one for the side that must move right now
                let is_white_turn = controller_mpv.borrow().is_white_turn();
                let active_n = if is_white_turn {
                    win.get_analysis_multipv_white()
                } else {
                    win.get_analysis_multipv_black()
                };

                if n == 0 {
                    // This side is disabled: clear its lines immediately
                    // (regardless of who is moving — the panel must be clean)
                    let empty = ModelRc::new(VecModel::<gui::PvLine>::from(vec![]));
                    if for_white {
                        analysis_mpv.borrow_mut().white.stop();
                        win.set_engine_pv_lines_white(empty);
                        win.set_pv_selected_rank_white(0);
                    } else {
                        analysis_mpv.borrow_mut().black.stop();
                        win.set_engine_pv_lines_black(empty);
                        win.set_pv_selected_rank_black(0);
                    }
                    // If it's the turn of the side we just disabled, also turn off the UI
                    if for_white == is_white_turn {
                        win.set_engine_thinking(false);
                        win.set_eval_bar_visible(false);
                        win.set_hint_arrow_path("".into());
                    }
                } else {
                    // Restart the analysis if the change concerns the active side
                    let camp_changed_is_active = for_white == is_white_turn;
                    if camp_changed_is_active && analysis_mpv.borrow().has_engine() {
                        let fen = controller_mpv.borrow().current_fen();
                        analysis_mpv.borrow_mut().start_for(is_white_turn, fen, win.as_weak(), is_white_turn, active_n as u32);
                    }
                }
            }
        };

        window.on_multipv_white_changed(make_mpv_callback(true));
        window.on_multipv_black_changed(make_mpv_callback(false));
    }

    // 16c. Callbacks: click on a PV line → arrow of the first move
    {
        let make_pv_click = |for_white: bool| {
            let window_pv = window.as_weak();
            move |rank: i32| {
                let Some(win) = window_pv.upgrade() else { return; };
                let flipped = win.get_board_flipped();

                // Retrieve the PV line from the right model
                let pv_lines = if for_white { win.get_engine_pv_lines_white() }
                               else         { win.get_engine_pv_lines_black() };
                let line = (0..pv_lines.row_count())
                    .filter_map(|i| pv_lines.row_data(i))
                    .find(|l| l.rank == rank);

                let arrow = line.and_then(|l| {
                    let first_move = l.pv.as_str().split_whitespace().next()?.to_owned();
                    let b = first_move.as_bytes();
                    if b.len() < 4 { return None; }
                    let fc = i32::from(b[0]) - i32::from(b'a');
                    let fr = 8 - (i32::from(b[1]) - i32::from(b'0'));
                    let tc = i32::from(b[2]) - i32::from(b'a');
                    let tr = 8 - (i32::from(b[3]) - i32::from(b'0'));
                    if !(0..8).contains(&fc) || !(0..8).contains(&fr)
                    || !(0..8).contains(&tc) || !(0..8).contains(&tr) {
                        return None;
                    }
                    let s = compute_hint_arrow(fr, fc, tr, tc, flipped);
                    if s.is_empty() { None } else { Some(s) }
                }).unwrap_or_default();

                win.set_hint_arrow_path(arrow.into());
            }
        };

        window.on_pv_line_white_clicked(make_pv_click(true));
        window.on_pv_line_black_clicked(make_pv_click(false));
    }

    // 17. Callback: wizard "Démarrer" → build a GameConfig + reset the game
    let window_weak_setup    = window.as_weak();
    let controller_for_setup = controller.clone();
    let analysis_for_setup   = analysis.clone();
    let game_bridge_for_setup = game_bridge.clone();
    let score_history_setup  = score_history.clone();
    let lang_for_setup       = lang_cell.clone();
    let engine_list_setup    = engine_list.clone();   // to inject the UCI options
    let chess_clock_setup    = chess_clock.clone();
    let pending_pgn_setup    = pending_pgn.clone();
    let book_white_setup     = book_white.clone();
    let book_black_setup     = book_black.clone();

    window.on_setup_start(move |mode, hcolor, path_w, path_b, level_w, level_b, game_tc_idx, white_time_secs, black_time_secs| {
        let lang = *lang_for_setup.borrow();

        // PHASE 66 — reported by the user: "Aléatoire" (hcolor == 2)
        // ALWAYS gave White to the human. Cause: the config
        // construction below only distinguished hcolor == 1 (explicit
        // Black); hcolor == 2 fell into the default branch
        // (`_ => human_vs_engine`, human White) exactly like hcolor ==
        // 0. `config.human_color = HumanColor::Random` was only set
        // AFTERWARD, as a simple metadata field never read back to decide
        // anything — the actual draw (`game_config::roll_random_is_white`)
        // was never called anywhere in the whole GUI. So the
        // color is now drawn AT THE VERY START, only once, and is
        // used both to build the config AND (further below) to
        // orient the board — both MUST use exactly the
        // same value, never two separate draws.
        let human_is_white = match hcolor {
            1 => false,
            2 => game_config::roll_random_is_white(),
            _ => true,
        };

        // Build the GameConfig from the wizard's parameters
        let mut config = match mode {
            0 => GameConfig::human_vs_human(),
            2 => GameConfig::engine_vs_engine(path_w.as_str(), path_b.as_str()),
            // H vs M: path_b is always the opposing engine's path
            _ if human_is_white => GameConfig::human_vs_engine(path_b.as_str()),
            _ => GameConfig::human_vs_engine_as_black(path_b.as_str()),
        };
        config.human_color = match hcolor {
            1 => HumanColor::Black,
            2 => HumanColor::Random,
            _ => HumanColor::White,
        };
        if let Some(e) = config.white_engine.as_mut() {
            e.time_control = TimeControl::Level(level_w.clamp(1, 12) as u8);
        }
        if let Some(e) = config.black_engine.as_mut() {
            e.time_control = TimeControl::Level(level_b.clamp(1, 12) as u8);
        }

        // Inject the saved UCI options (Préférences → Moteurs) into the config.
        // Each engine is looked up by path in engine_list to copy its options.
        {
            let engines = engine_list_setup.borrow();
            // PHASE 74 — bug introduced by PHASE 73: `settings.path`
            // now comes from the RELATIVE path chosen in the wizard
            // (`SavedEngine.path` on the Slint side, PHASE 73), while `e.path`
            // (`engine_list`, on the Rust side) stays absolute (resolved by
            // `prefs::load_engines`). A raw `String` equality between
            // the two therefore never matched anymore, silently: the
            // user's custom UCI options were no longer
            // injected into any game launched from the wizard.
            // Fixed by resolving `settings.path` to absolute before
            // comparing (`Path`, not `String`, to also tolerate a possible
            // mixed separator — the same precaution as PHASE 71).
            let inject = |settings: &mut game_config::EngineSettings| {
                let settings_abs = app_paths::to_absolute_path(&settings.path);
                if let Some(saved) = engines.iter().find(|e| std::path::Path::new(&e.path) == settings_abs) {
                    settings.options.clone_from(&saved.options);
                }
            };
            if let Some(e) = config.white_engine.as_mut() { inject(e); }
            if let Some(e) = config.black_engine.as_mut() { inject(e); }
        }

        // Time control chosen in the wizard (idx → TimeControl)
        config.time_control = match game_tc_idx {
            1 => TimeControl::BULLET_1_0,
            2 => TimeControl::BULLET_2_1,
            3 => TimeControl::BLITZ_3_2,
            4 => TimeControl::BLITZ_5_0,
            5 => TimeControl::RAPID_10_5,
            6 => TimeControl::RAPID_15_10,
            7 => TimeControl::CLASSICAL_90_30,
            _ => TimeControl::Infinite,
        };

        // Handicap overrides: asymmetric times if requested (0 = use the preset)
        if white_time_secs > 0 || black_time_secs > 0 {
            config.white_time_secs_override = if white_time_secs > 0 {
                Some(white_time_secs as u64)
            } else {
                None
            };
            config.black_time_secs_override = if black_time_secs > 0 {
                Some(black_time_secs as u64)
            } else {
                None
            };
        }

        // Debug mode (PHASE 26sexies): new game → new game
        // GUID, and a context record (full config, time control)
        // to be able to replay/understand a log session after the fact.
        gui::debug_log::new_game();
        gui::debug_log::log_event("game_started", &serde_json::json!({
            "source": "wizard",
            "config": format!("{config:?}"),
            "time_control": format!("{:?}", config.time_control),
        }));

        // Persistence (including the UCI options)
        let _ = gc_persist::save_last_config(&config);
        let _ = gc_persist::save_last_mode(config.mode);

        // Initialize the clock from the configured time control
        let tc = config.time_control;
        let mut new_clock = ChessClock::new(&tc);
        // Apply the handicap if configured
        if let (Some(w), Some(b)) = (config.white_time_secs_override, config.black_time_secs_override) {
            new_clock.set_initial_times(w as i64 * 1_000, b as i64 * 1_000);
        } else if let Some(w) = config.white_time_secs_override {
            let b_ms = new_clock.black_ms();
            new_clock.set_initial_times(w as i64 * 1_000, b_ms);
        } else if let Some(b) = config.black_time_secs_override {
            let w_ms = new_clock.white_ms();
            new_clock.set_initial_times(w_ms, b as i64 * 1_000);
        }
        *chess_clock_setup.borrow_mut() = new_clock;
        let show_clocks = tc.use_player_clock();

        // Reset the board + initialize the engine players
        if let Some(win) = window_weak_setup.upgrade() {
            analysis_for_setup.borrow_mut().stop();

            // Init the engine-player bridge BEFORE the reset (to avoid a premature trigger)
            game_bridge_for_setup.borrow_mut().init(&config, &win.as_weak());

            let mut ctrl = controller_for_setup.borrow_mut();
            // Priority: editor/database FEN > PGN > starting position
            let start_fen = win.get_wizard_start_fen().to_string();
            let pgn_content = pending_pgn_setup.borrow().clone();
            if !start_fen.is_empty() {
                // Position set in the position editor or chosen from
                // the game database (ergonomics bugfix 09/07/2026)
                ctrl.load_from_fen(&start_fen);
                win.set_wizard_start_fen("".into());
                win.set_wizard_start_fen_from_base(false);
            } else if pgn_content.is_empty() || ctrl.load_from_pgn(&pgn_content).is_err() {
                ctrl.reset();
            }
            // Consume the PGN (applies only once)
            *pending_pgn_setup.borrow_mut() = String::new();
            win.set_wizard_pgn_filename("".into());
            score_history_setup.borrow_mut().clear();

            win.set_show_promotion_modal(false);
            win.set_is_game_over(false);
            win.set_game_over_result(slint::SharedString::default());
            win.set_game_over_reason(slint::SharedString::default());
            win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
            win.set_white_curve_path(slint::SharedString::default());
            win.set_black_curve_path(slint::SharedString::default());
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv("—".into());
            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_engine_playing(false);
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_hint_arrow_path("".into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            win.set_hint_computing(false);
            win.set_current_game_mode(mode);  // 0=HvH 1=HvM 2=MvM
            // PHASE 66 — reported by the user: the board would
            // never orient itself automatically from Black's point of view,
            // whether after an explicit "Black" choice or after a "Random"
            // draw that landed on Black — only the manual flip
            // button changed `board-flipped`, which therefore stayed frozen
            // from the previous game. In H vs M, the orientation is now
            // aligned with `human_is_white` (the same value used just
            // above to build the config, never a second draw); in H vs H
            // / M vs M (no dedicated "human side"), the board now
            // always starts from White-at-the-bottom orientation, so as not
            // to inherit a state left over from a previous game.
            win.set_board_flipped(mode == 1 && !human_is_white);
            refresh_game_state(&win, &ctrl, lang);

            // Clock: initial display + start (White moves first)
            win.set_show_clocks(show_clocks);
            if show_clocks {
                let clk = chess_clock_setup.borrow();
                push_clock_to_window(&win, &clk);
                drop(clk);
                chess_clock_setup.borrow_mut().start(true);
                win.set_white_clock_active(true);
                win.set_black_clock_active(false);
            } else {
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);
                win.set_white_clock_text("--:--".into());
                win.set_black_clock_text("--:--".into());
            }

            // Trigger the engine if it moves first (White in M vs M or H vs M with the human as Black)
            if !ctrl.is_over() {
                drop(ctrl); // release the borrow before try_play_book_moves

                // PHASE 15: book move(s) before consulting the engine.
                if try_play_book_moves(&win, &controller_for_setup, &chess_clock_setup, &game_bridge_for_setup, &book_white_setup, &book_black_setup) {
                    refresh_game_state(&win, &controller_for_setup.borrow(), lang);
                }

                let (is_white_turn, is_over, fen) = {
                    let ctrl = controller_for_setup.borrow();
                    (ctrl.is_white_turn(), ctrl.is_over(), ctrl.current_fen())
                };
                if !is_over {
                    win.set_white_clock_active(is_white_turn);
                    win.set_black_clock_active(!is_white_turn);
                    if show_clocks {
                        chess_clock_setup.borrow_mut().start(is_white_turn);
                    }
                    let limits_opt = build_go_limits(&chess_clock_setup.borrow());
                    let _ = game_bridge_for_setup.borrow().trigger_if_engine_turn(is_white_turn, fen, limits_opt);
                }
            }
        }
    });

    // 17bis. Callback: wizard "Démarrer" in Puzzle mode — persists both
    // wizard choices then delegates the draw/display to `load_random_puzzle`
    // (shared with "Next puzzle", Step 6).
    let window_weak_puzzle     = window.as_weak();
    let controller_for_puzzle  = controller.clone();
    let analysis_for_puzzle    = analysis.clone();
    let game_bridge_for_puzzle = game_bridge.clone();
    let chess_clock_for_puzzle = chess_clock.clone();
    let score_history_puzzle   = score_history.clone();
    let lang_for_puzzle        = lang_cell.clone();
    let puzzle_session_start   = puzzle_session.clone();

    window.on_setup_start_puzzle(move |hint_theme, hint_button| {
        let Some(win) = window_weak_puzzle.upgrade() else { return; };
        let lang = *lang_for_puzzle.borrow();

        // Persist both choices, regardless of whether the draw below succeeds.
        prefs::save_puzzle_hint_theme(hint_theme);
        prefs::save_puzzle_hint_button(hint_button);

        gui::debug_log::new_game();
        gui::debug_log::log_event("game_started", &serde_json::json!({
            "source": "puzzle",
            "hint_theme": hint_theme,
            "hint_button": hint_button,
        }));

        load_random_puzzle(
            &win,
            &controller_for_puzzle,
            &analysis_for_puzzle,
            &game_bridge_for_puzzle,
            &chess_clock_for_puzzle,
            &score_history_puzzle,
            &puzzle_session_start,
            lang,
        );
    });

    // 17ter. Callback: "Voir la solution" during a puzzle (PHASE 14, Step 6).
    // Automatically plays all remaining moves (opponent + human) via
    // `PuzzleSession::reveal_solution`, mirrors them on the real board, and
    // shows the result banner. Never counts as a solve
    // (see `PuzzleSession::outcome_for_stats`).
    let window_weak_puzzle_reveal    = window.as_weak();
    let controller_for_puzzle_reveal = controller.clone();
    let analysis_for_puzzle_reveal   = analysis.clone();
    let puzzle_session_reveal        = puzzle_session.clone();
    let lang_for_puzzle_reveal       = lang_cell.clone();

    window.on_puzzle_show_solution(move || {
        let Some(win) = window_weak_puzzle_reveal.upgrade() else { return; };
        let lang = *lang_for_puzzle_reveal.borrow();

        let mut session_slot = puzzle_session_reveal.borrow_mut();
        let Some(session) = session_slot.as_mut() else { return; };
        if session.is_finished() { return; }

        let played = session.reveal_solution();
        {
            let mut ctrl = controller_for_puzzle_reveal.borrow_mut();
            for mv in &played {
                ctrl.apply_uci_move(&mv.to_uci());
            }
        }
        refresh_game_state(&win, &controller_for_puzzle_reveal.borrow(), lang);
        analysis_for_puzzle_reveal.borrow_mut().stop();
        win.set_eval_bar_visible(false);

        finish_puzzle_banner(&win, session);
        record_puzzle_stats(&win, session);
    });

    // 17quater. Callback: "Puzzle suivant" (PHASE 14, Step 6). First
    // records the current attempt if it isn't already finished (otherwise
    // already counted by `handle_puzzle_move`/"Voir la solution" — avoids a
    // double count), then immediately draws a new puzzle.
    let window_weak_puzzle_next     = window.as_weak();
    let controller_for_puzzle_next  = controller.clone();
    let analysis_for_puzzle_next    = analysis.clone();
    let game_bridge_for_puzzle_next = game_bridge.clone();
    let chess_clock_for_puzzle_next = chess_clock.clone();
    let score_history_puzzle_next   = score_history.clone();
    let puzzle_session_next         = puzzle_session.clone();
    let lang_for_puzzle_next        = lang_cell.clone();

    window.on_puzzle_next(move || {
        let Some(win) = window_weak_puzzle_next.upgrade() else { return; };
        let lang = *lang_for_puzzle_next.borrow();

        // Abandoned after at least one mistake (not a "neutral" abandonment): to
        // be flagged with a toast, but only AFTER the new puzzle is
        // loaded below — `load_random_puzzle` resets
        // `puzzle-feedback-text` to "", and Slint only renders the final state of
        // the properties at the end of this callback (not intermediate values);
        // setting the toast text before the call would therefore be
        // immediately overwritten and never visible (PHASE 14, Step 7).
        let mut show_abandon_toast = false;
        if let Some(session) = puzzle_session_next.borrow().as_ref() {
            if !session.is_finished() {
                record_puzzle_stats(&win, session);
                show_abandon_toast = session.wrong_attempts_count() > 0;
            }
        }

        load_random_puzzle(
            &win,
            &controller_for_puzzle_next,
            &analysis_for_puzzle_next,
            &game_bridge_for_puzzle_next,
            &chess_clock_for_puzzle_next,
            &score_history_puzzle_next,
            &puzzle_session_next,
            lang,
        );

        if show_abandon_toast {
            win.set_puzzle_feedback_text(i18n::translate("status.puzzle_feedback_abandoned").into());
            win.set_puzzle_feedback_positive(false);
            win.set_puzzle_feedback_seq(win.get_puzzle_feedback_seq() + 1);
        }
    });

    // 17quinquies. Callback: "Quitter les puzzles" (PHASE 14, Step 6).
    // Records the current attempt if needed (same rule as "Next
    // puzzle"), clears the puzzle state, then reuses exactly the existing
    // "New Game" flow (recap if a previous config exists,
    // otherwise a blank wizard) rather than inventing a dedicated exit path.
    let window_weak_puzzle_quit  = window.as_weak();
    let analysis_for_puzzle_quit = analysis.clone();
    let puzzle_session_quit      = puzzle_session.clone();

    window.on_puzzle_quit(move || {
        let Some(win) = window_weak_puzzle_quit.upgrade() else { return; };

        if let Some(session) = puzzle_session_quit.borrow().as_ref() {
            if !session.is_finished() {
                record_puzzle_stats(&win, session);
            }
        }
        *puzzle_session_quit.borrow_mut() = None;

        analysis_for_puzzle_quit.borrow_mut().stop();
        win.set_puzzle_mode_active(false);
        win.set_eval_bar_visible(false);

        win.invoke_open_new_game();
    });

    // 18. Callback: recap "Rejouer" → reload the config + reset the game
    let window_weak_replay    = window.as_weak();
    let controller_for_replay = controller.clone();
    let analysis_for_replay   = analysis.clone();
    let game_bridge_for_replay = game_bridge.clone();
    let score_history_replay  = score_history.clone();
    let lang_for_replay       = lang_cell.clone();
    let chess_clock_replay    = chess_clock.clone();
    let book_white_replay     = book_white.clone();
    let book_black_replay     = book_black.clone();

    window.on_setup_replay(move || {
        let lang = *lang_for_replay.borrow();

        // Reload the persisted config to reset the engine players
        let saved_config = gc_persist::load_last_mode()
            .and_then(gc_persist::load_last_config);

        // Initialize the clock from the saved config (with handicap if persisted)
        let tc = saved_config.as_ref().map_or(TimeControl::Infinite, |c| c.time_control);
        let mut replay_clock = ChessClock::new(&tc);
        if let Some(ref cfg) = saved_config {
            if let (Some(w), Some(b)) = (cfg.white_time_secs_override, cfg.black_time_secs_override) {
                replay_clock.set_initial_times(w as i64 * 1_000, b as i64 * 1_000);
            } else if let Some(w) = cfg.white_time_secs_override {
                let b_ms = replay_clock.black_ms();
                replay_clock.set_initial_times(w as i64 * 1_000, b_ms);
            } else if let Some(b) = cfg.black_time_secs_override {
                let w_ms = replay_clock.white_ms();
                replay_clock.set_initial_times(w_ms, b as i64 * 1_000);
            }
        }
        *chess_clock_replay.borrow_mut() = replay_clock;
        let show_clocks = tc.use_player_clock();

        if let Some(win) = window_weak_replay.upgrade() {
            analysis_for_replay.borrow_mut().stop();

            // Re-init the engine bridge with the saved config
            if let Some(ref cfg) = saved_config {
                game_bridge_for_replay.borrow_mut().init(cfg, &win.as_weak());
            } else {
                game_bridge_for_replay.borrow_mut().reset();
            }

            let mut ctrl = controller_for_replay.borrow_mut();
            ctrl.reset();
            score_history_replay.borrow_mut().clear();

            win.set_show_promotion_modal(false);
            win.set_is_game_over(false);
            win.set_game_over_result(slint::SharedString::default());
            win.set_game_over_reason(slint::SharedString::default());
            win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
            win.set_white_curve_path(slint::SharedString::default());
            win.set_black_curve_path(slint::SharedString::default());
            win.set_engine_depth("—".into());
            win.set_engine_score("—".into());
            win.set_engine_pv("—".into());
            win.set_game_paused(false);
            win.set_engine_thinking(false);
            win.set_engine_playing(false);
            win.set_eval_bar_visible(false);
            win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
            win.set_hint_arrow_path("".into());
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
            win.set_hint_computing(false);
            {
                let replay_mode = saved_config.as_ref().map_or(0, |c| match c.mode {
                    GameMode::HumanVsEngine => 1,
                    GameMode::EngineVsEngine => 2,
                    GameMode::HumanVsHuman => 0,
                });
                win.set_current_game_mode(replay_mode);
            }
            // PHASE 66 — same bugfix as `on_setup_start`: orient
            // the board according to the human side of the replayed config. Here the
            // color is already fixed in `saved_config` (engine/human
            // assignment already resolved during the first game, including
            // if "Random" had been chosen): human is White
            // in H vs M if `white_engine` is absent — no new draw.
            win.set_board_flipped(
                saved_config.as_ref().is_some_and(|c| {
                    c.mode == GameMode::HumanVsEngine && c.white_engine.is_some()
                })
            );
            refresh_game_state(&win, &ctrl, lang);

            // Clock: initial display + start
            win.set_show_clocks(show_clocks);
            if show_clocks {
                let clk = chess_clock_replay.borrow();
                push_clock_to_window(&win, &clk);
                drop(clk);
                chess_clock_replay.borrow_mut().start(true);
                win.set_white_clock_active(true);
                win.set_black_clock_active(false);
            } else {
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);
                win.set_white_clock_text("--:--".into());
                win.set_black_clock_text("--:--".into());
            }

            // Trigger the engine if it moves first
            if !ctrl.is_over() {
                drop(ctrl); // release the borrow before try_play_book_moves

                // PHASE 15: book move(s) before consulting the engine.
                if try_play_book_moves(&win, &controller_for_replay, &chess_clock_replay, &game_bridge_for_replay, &book_white_replay, &book_black_replay) {
                    refresh_game_state(&win, &controller_for_replay.borrow(), lang);
                }

                let (is_white_turn, is_over, fen) = {
                    let ctrl = controller_for_replay.borrow();
                    (ctrl.is_white_turn(), ctrl.is_over(), ctrl.current_fen())
                };
                if !is_over {
                    win.set_white_clock_active(is_white_turn);
                    win.set_black_clock_active(!is_white_turn);
                    if show_clocks {
                        chess_clock_replay.borrow_mut().start(is_white_turn);
                    }
                    let limits_opt = build_go_limits(&chess_clock_replay.borrow());
                    let _ = game_bridge_for_replay.borrow().trigger_if_engine_turn(is_white_turn, fen, limits_opt);
                }
            }
        }
    });

    // ── Active tournament state (None = no tournament in progress) ────────────────
    let tournament_runner: Rc<RefCell<Option<TournamentRunner>>> =
        Rc::new(RefCell::new(None));

    // 19. Callback: engine player found its move → apply it + chain
    let window_weak_engine        = window.as_weak();
    let controller_for_engine     = controller.clone();
    let analysis_for_engine       = analysis.clone();
    let game_bridge_for_engine    = game_bridge.clone();
    let score_history_engine      = score_history.clone();
    let lang_for_engine           = lang_cell.clone();
    let chess_clock_engine        = chess_clock.clone();
    let tournament_runner_engine  = tournament_runner.clone();
    let book_white_engine         = book_white.clone();
    let book_black_engine         = book_black.clone();

    window.on_engine_move_ready(move |uci_str| {
        // Ignore the move if the game is paused
        if window_weak_engine.upgrade().is_some_and(|w| w.get_game_paused()) {
            return;
        }
        // PHASE 26, Step 2: also ignore any engine move that arrives
        // during variation editing (calculation started before entering this
        // mode) — the result must never apply to the line currently
        // being explored.
        if controller_for_engine.borrow().is_variation_editing() {
            return;
        }

        let lang = *lang_for_engine.borrow();

        // Apply the UCI move to the controller (no active borrow_mut during the following calls)
        let move_applied = {
            let mut ctrl = controller_for_engine.borrow_mut();
            ctrl.apply_uci_move(uci_str.as_str())
        };

        if !move_applied { return; }

        let Some(win) = window_weak_engine.upgrade() else { return };

        // Refresh the visual state
        {
            let ctrl = controller_for_engine.borrow();
            refresh_game_state(&win, &ctrl, lang);
        }

        let is_over;
        let mut is_white_turn;
        let mut fen;
        {
            let ctrl = controller_for_engine.borrow();
            is_over       = ctrl.is_over();
            is_white_turn = ctrl.is_white_turn();
            fen           = ctrl.current_fen();
        }

        if is_over {
            chess_clock_engine.borrow_mut().stop();
            win.set_white_clock_active(false);
            win.set_black_clock_active(false);

            // ── Tournament mode: record result and chain ─────────────
            if win.get_is_tournament_mode() {
                // PGN result of the finished game
                let pgn_result = {
                    let ctrl = controller_for_engine.borrow();
                    ctrl.result_pgn().unwrap_or("*").to_owned()
                };
                let t_result = match pgn_result.as_str() {
                    "1-0"     => tournament::GameResult::WhiteWins,
                    "0-1"     => tournament::GameResult::BlackWins,
                    "1/2-1/2" => tournament::GameResult::Draw,
                    _ => {
                        // Unknown result → clean tournament stop
                        win.set_is_tournament_mode(false);
                        let _ = score_history_engine.borrow();
                        return;
                    }
                };

                // Record + fetch the next game
                let (played, next_game_opt) = {
                    let mut tr_opt = tournament_runner_engine.borrow_mut();
                    let Some(tr) = tr_opt.as_mut() else {
                        win.set_is_tournament_mode(false);
                        let _ = score_history_engine.borrow();
                        return;
                    };
                    let played    = tr.state.record_result(t_result);
                    let next_game = if tr.state.is_finished() {
                        None
                    } else {
                        tr.state.next_game().cloned()
                    };
                    (played, next_game)
                };

                // Save the result to the DB. Reuses the SQLite
                // connection opened only once at tournament creation
                // (tr.db_conn) instead of reopening/re-migrating the DB for each
                // game (perf audit 02/07/2026, point 6).
                if let Some(ref g) = played {
                    // Defensive indexing: if an engine index were ever
                    // invalid (future bug in the tournament scheduler),
                    // cleanly abandon saving THIS game
                    // instead of panicking and crashing the whole application.
                    let tr_opt = tournament_runner_engine.borrow();
                    if let Some(tr) = tr_opt.as_ref() {
                        match (
                            tr.state.config.engines.get(g.white),
                            tr.state.config.engines.get(g.black),
                        ) {
                            (Some(w), Some(b)) => {
                                let wname = w.0.clone();
                                let bname = b.0.clone();
                                // Actual PGN of the game (previously saved as hardcoded empty).
                                let pgn   = controller_for_engine.borrow().export_pgn(&wname, &bname);
                                let moves = controller_for_engine.borrow().move_count() as i64;
                                let _ = db::repository::tournament_repo::save_game_result(
                                    &tr.db_conn, tr.tournament_id, &wname, &bname,
                                    &pgn_result, &pgn, g.round, moves,
                                );
                            }
                            _ => {
                                eprintln!(
                                    "[Tournament] Indice moteur invalide (white={}, black={}) — sauvegarde ignorée.",
                                    g.white, g.black
                                );
                            }
                        }
                    }
                }

                // Update the standings (after record_result)
                {
                    let tr_opt = tournament_runner_engine.borrow();
                    if let Some(tr) = tr_opt.as_ref() {
                        push_tournament_standings(&win, tr);
                    }
                }

                match next_game_opt {
                    None => {
                        // Tournament finished — show the final standings
                        win.set_tournament_finished(true);
                        win.set_is_game_over(false);
                        eprintln!("[Tournament] Terminé — classement final affiché");
                    }
                    Some(next) => {
                        // Prepare the next game (defensive indexing:
                        // see equivalent comment for the DB save above).
                        // Code audit 04/07/2026, point 2: defensive `if let` instead
                        // of an `unwrap()` — aligned with the style already used at
                        // nearby sites (above), with no risk of a panic if
                        // the runner were to disappear in the meantime.
                        let next_engines = {
                            let tr_opt = tournament_runner_engine.borrow();
                            match tr_opt.as_ref() {
                                Some(tr) => match (
                                    tr.state.config.engines.get(next.white),
                                    tr.state.config.engines.get(next.black),
                                ) {
                                    (Some(w), Some(b)) => Some((
                                        w.0.clone(), w.1.clone(), b.0.clone(), b.1.clone(), tr.time_control,
                                    )),
                                    _ => None,
                                },
                                None => None,
                            }
                        };
                        let Some((wname, wpath, bname, bpath, next_tc)) = next_engines else {
                            eprintln!(
                                "[Tournament] Indice moteur invalide pour la partie suivante (white={}, black={}) — arrêt du tournoi.",
                                next.white, next.black
                            );
                            win.set_is_tournament_mode(false);
                            let _ = score_history_engine.borrow();
                            return;
                        };

                        // Stop analysis + clear the score graph
                        analysis_for_engine.borrow_mut().stop();
                        win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
                        win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
                        win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
                        score_history_engine.borrow_mut().clear();
                        win.set_score_history(
                            ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
                        win.set_white_curve_path("".into());
                        win.set_black_curve_path("".into());

                        // Reset the controller + refresh the board
                        controller_for_engine.borrow_mut().reset();
                        {
                            let ctrl = controller_for_engine.borrow();
                            refresh_game_state(&win, &ctrl, lang);
                        }

                        // Clock: reset for the new game
                        *chess_clock_engine.borrow_mut() = ChessClock::new(&next_tc);
                        let show_clocks = next_tc.use_player_clock();
                        win.set_show_clocks(show_clocks);
                        win.set_white_clock_active(show_clocks);
                        win.set_black_clock_active(false);
                        if show_clocks {
                            chess_clock_engine.borrow_mut().start(true);
                            push_clock_to_window(&win, &chess_clock_engine.borrow());
                        }

                        // MvM config with the tournament's time control
                        let mut gc = GameConfig::engine_vs_engine(&wpath, &bpath);
                        if let Some(e) = gc.white_engine.as_mut() {
                            e.time_control = next_tc;
                        }
                        if let Some(e) = gc.black_engine.as_mut() {
                            e.time_control = next_tc;
                        }

                        // Reset bridge + launch
                        game_bridge_for_engine.borrow_mut().reset();
                        game_bridge_for_engine.borrow_mut().init(&gc, &win.as_weak());

                        // Current game in the tournament panel
                        win.set_tournament_current_white(wname.clone().into());
                        win.set_tournament_current_black(bname.clone().into());

                        // Player names (black/white sidebar)
                        win.set_white_player_name(wname.into());
                        win.set_black_player_name(bname.into());

                        // Trigger the first move (White)
                        let fen = controller_for_engine.borrow().current_fen();
                        let limits_opt = build_go_limits(&chess_clock_engine.borrow());
                        let _ = game_bridge_for_engine.borrow()
                            .trigger_if_engine_turn(true, fen, limits_opt);
                    }
                }

                let _ = score_history_engine.borrow();
                return;
            }
            // ── End tournament mode ─────────────────────────────────────────────

            let _ = score_history_engine.borrow();
            return;
        }

        // Clock: bonus to the engine that just moved, start the next player
        let just_moved_white = !is_white_turn;
        {
            let mut clk = chess_clock_engine.borrow_mut();
            clk.apply_move_bonus(just_moved_white);
            clk.start(is_white_turn);
        }
        win.set_white_clock_active(is_white_turn);
        win.set_black_clock_active(!is_white_turn);

        // PHASE 15: book move(s) before consulting the opposing engine.
        // Automatic no-op in tournament mode (guard internal to the function) —
        // needed here because this path is shared with a tournament game
        // still in progress (not finished, hence not covered by the `if is_over`
        // above).
        let mut book_is_over = false;
        if try_play_book_moves(&win, &controller_for_engine, &chess_clock_engine, &game_bridge_for_engine, &book_white_engine, &book_black_engine) {
            refresh_game_state(&win, &controller_for_engine.borrow(), lang);

            let ctrl = controller_for_engine.borrow();
            book_is_over = ctrl.is_over();
            is_white_turn = ctrl.is_white_turn();
            fen           = ctrl.current_fen();
            drop(ctrl);

            if book_is_over {
                chess_clock_engine.borrow_mut().stop();
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);
            } else {
                chess_clock_engine.borrow_mut().start(is_white_turn);
                win.set_white_clock_active(is_white_turn);
                win.set_black_clock_active(!is_white_turn);
            }
        }
        if book_is_over { return; }

        // Trigger the opposing engine if it's its turn (M vs M) — unless
        // variation editing is in progress (PHASE 26, Step 2).
        if !controller_for_engine.borrow().is_variation_editing() {
            let limits_opt = build_go_limits(&chess_clock_engine.borrow());
            let _ = game_bridge_for_engine.borrow()
                .trigger_if_engine_turn(is_white_turn, fen.clone(), limits_opt);
        }

        // Start analysis for all modes (H vs H, H vs M, M vs M)
        let mpv_n = if is_white_turn { win.get_analysis_multipv_white() } else { win.get_analysis_multipv_black() };
        if mpv_n > 0 {
            analysis_for_engine.borrow_mut().start_for(is_white_turn, fen, win.as_weak(), is_white_turn, mpv_n as u32);
        }

        let _ = score_history_engine.borrow(); // avoids an unused warning on the capture
    });

    // 20. Callback: the user confirms "Quit" from the wizard.
    window.on_quit_app(|| {
        slint::quit_event_loop().ok();
    });

    // 21. Callback: full reset of preferences.
    let window_weak_reset  = window.as_weak();
    let engine_list_reset  = engine_list.clone();
    let book_white_reset   = book_white.clone();
    let book_black_reset   = book_black.clone();
    let puzzle_session_reset = puzzle_session.clone();
    let lang_for_reset        = lang_cell.clone();
    let controller_for_reset = controller.clone();
    window.on_reset_prefs(move || {
        // Delete the entire prefs folder (lang, engines, game configs,
        // Polyglot opening books, Puzzle mode choice)
        prefs::reset_all();

        // Immediate switch of the current session to the default language
        // (English, `Lang::default()`) — explicit user request
        // from 05/07/2026. Deliberately **in-memory only**: no
        // call to `prefs::save_lang()` here, since `reset_all()` has
        // just erased `lang.txt`. If the user quits without
        // going back through Preferences, the absence of a saved file will
        // bring back the language-choice screen on next launch
        // (like a first launch); if they choose a language in
        // Preferences before quitting, that choice is saved normally
        // by `on_pref_language_chosen`.
        *lang_for_reset.borrow_mut() = Lang::default();
        i18n::set_lang(Lang::default());

        // Clear the in-memory engine list
        engine_list_reset.borrow_mut().clear();

        // PHASE 24, Step 6: also delete the book files themselves
        // (ouvertures/blancs.bin, ouvertures/noirs.bin). Since the removal
        // of the separate registry (book_white.txt/book_black.txt), the presence of a
        // book is inferred solely from the file's existence — without this
        // deletion, "Reset" would no longer change anything for the books
        // (the file would still be detected on next startup).
        let _ = std::fs::remove_file(app_paths::book_blancs_path());
        let _ = std::fs::remove_file(app_paths::book_noirs_path());

        // Also clear the in-memory Polyglot books — without this, a book already
        // loaded would remain active in-game (and shown in Preferences) even
        // after "Reset", since the deletions above only affect
        // disk, not the runtime state already in RAM.
        *book_white_reset.borrow_mut() = None;
        *book_black_reset.borrow_mut() = None;

        // PHASE 14: same principle for the current puzzle session.
        *puzzle_session_reset.borrow_mut() = None;

        // Bugfix from 03/07/2026 (user feedback): full reset now
        // ALSO clears the puzzle database (puzzles +
        // progress statistics), unlike the initial decision from
        // Step 5 which left it intact. `reset_all()` only erases the
        // prefs folder, never the SQLite database — so `clear_all` must
        // be called explicitly here. Silent failure if the database is
        // unreachable, consistent with the handling of other non-blocking
        // DB errors elsewhere in the project.
        let puzzle_count_after_reset =
            if let Ok(conn) = db::schema::open_and_migrate(&tournament_db_path()) {
                let _ = db::repository::puzzle_repo::clear_all(&conn);
                db::repository::puzzle_repo::global_stats(&conn).ok()
            } else {
                None
            };

        if let Some(win) = window_weak_reset.upgrade() {
            // Refresh the entire Slint UI (`Tr` properties) in the new
            // default language, and the current status text — same rationale
            // as `on_pref_language_chosen` above.
            i18n_bridge::apply_translations(&win.global::<Tr>(), Lang::default());
            win.set_current_lang_index(Lang::default().ui_index());
            {
                let ctrl = controller_for_reset.borrow();
                win.set_status_text(i18n::translate(ctrl.status_key()).into());
                if ctrl.is_over() {
                    win.set_game_over_result(i18n::translate_in(Lang::default(), ctrl.status_key()).into());
                    win.set_game_over_reason(i18n::translate_in(Lang::default(), ctrl.end_reason_key()).into());
                }
            }

            // Update the engine list in the window (empty)
            update_engines_in_window(&win, &[]);

            // Clear the book display in Preferences
            win.set_book_name_white("".into());
            win.set_book_name_black("".into());

            // Puzzle mode choice reset to defaults (No hint / hint button active)
            win.set_puzzle_hint_theme(false);
            win.set_puzzle_hint_button(true);
            // Hide the Puzzle control banner (Step 6) if a session
            // was in progress at the time of the reset.
            win.set_puzzle_mode_active(false);
            win.set_eval_bar_visible(false);
            // Puzzle database cleared above: reset the display to zero.
            win.set_puzzle_count(0);
            if let Some(stats) = puzzle_count_after_reset {
                push_puzzle_stats(&win, stats);
            } else {
                win.set_puzzle_stats_text("".into());
            }

            // Close the Preferences panel and relaunch the wizard
            win.set_show_preferences(false);
            win.set_pref_active_node(0);
            win.set_show_setup_recap(false);
            win.set_wizard_step(0);
            win.set_show_setup_wizard(true);
        }
    });

    // 22. Callback: query an engine's UCI options (Preferences → engine selection)
    //
    //   Run in a separate thread to avoid blocking the UI:
    //   UciEngine::connect_with_timeout → options() → quit() → invoke_from_event_loop
    let engine_list_query = engine_list.clone();
    let window_weak_query = window.as_weak();

    window.on_query_engine_options(move |idx| {
        let idx = idx as usize;

        // Fetch the engine path (from the main thread)
        let path = {
            let engines = engine_list_query.borrow();
            engines.get(idx).map(|e| e.path.clone())
        };
        let Some(path) = path else { return };

        // Options already saved by the user for this engine
        let user_opts: std::collections::HashMap<String, String> = {
            engine_list_query.borrow()
                .get(idx)
                .map(|e| e.options.clone())
                .unwrap_or_default()
        };

        let win_weak2 = window_weak_query.clone();

        std::thread::spawn(move || {
            use uci::{engine::UciEngine, parser::UciOptionKind};
            use std::time::Duration;

            // Options considered a priority (displayed first)
            const PRIORITY: &[&str] = &[
                "Threads", "Hash", "MultiPV", "Contempt",
                "Skill Level", "UCI_Elo", "Move Overhead",
            ];

            let items = match UciEngine::connect_with_timeout(&path, Duration::from_secs(5)) {
                Ok(engine) => {
                    let mut opts: Vec<UciOptionItem> = engine.options().iter().map(|opt| {
                        // Value: user override > default > ""
                        let value = user_opts
                            .get(&opt.name)
                            .cloned()
                            .or_else(|| opt.default.clone())
                            .unwrap_or_default();

                        let kind_str: &str = match &opt.kind {
                            UciOptionKind::Spin      => "spin",
                            UciOptionKind::Check     => "check",
                            UciOptionKind::Combo     => "combo",
                            UciOptionKind::Button    => "button",
                            UciOptionKind::StringOpt => "string",
                            UciOptionKind::Unknown(_) => "?",
                        };

                        UciOptionItem {
                            name:     opt.name.clone().into(),
                            kind:     kind_str.into(),
                            value:    value.into(),
                            default:  opt.default.clone().unwrap_or_default().into(),
                            min:      opt.min.unwrap_or(0) as i32,
                            max:      opt.max.unwrap_or(0) as i32,
                            vars:     opt.vars.join("|").into(),
                            priority: PRIORITY.contains(&opt.name.as_str()),
                            invalid:  false,
                        }
                    }).collect();

                    // Sort: priority ones first, then original UCI order
                    opts.sort_by(|a, b| {
                        let a_p = PRIORITY.contains(&a.name.as_str());
                        let b_p = PRIORITY.contains(&b.name.as_str());
                        b_p.cmp(&a_p)
                    });

                    engine.quit();
                    opts
                }
                Err(e) => {
                    eprintln!("[Prefs] Impossible de lire les options UCI : {e}");
                    Vec::new()
                }
            };

            slint::invoke_from_event_loop(move || {
                if let Some(win) = win_weak2.upgrade() {
                    // Robustness audit 11/07/2026, finding 3.2: no guard
                    // previously prevented several of these threads from
                    // running concurrently if the user clicked through
                    // multiple engines quickly in the Preferences list —
                    // whichever UCI handshake happened to finish last won,
                    // regardless of click order (a slower/unreachable
                    // engine responding after a faster one selected since
                    // could overwrite the currently-displayed options with
                    // stale data for the wrong engine). `selected-engine-idx`
                    // is set synchronously, on the UI thread, at the exact
                    // moment this thread was spawned (see `configure =>`
                    // in `preferences.slint`) — comparing it against `idx`
                    // here discards the response if the user has since
                    // selected a different engine, with no new flag/
                    // generation counter needed.
                    if win.get_selected_engine_idx() == i32::try_from(idx).unwrap_or(-1) {
                        win.set_engine_options(ModelRc::new(VecModel::from(items)));
                    }
                }
            }).ok();
        });
    });

    // ── Internal helpers: modify a UCI option in the model + persist ────
    //
    //  Each handler (inc / dec / toggle / cycle) follows the same pattern:
    //    1. Fetch engine-options (ModelRc shared with Slint)
    //    2. Find the option by name
    //    3. Compute the new value
    //    4. set_row_data → immediate Slint re-render
    //    5. Update engine_list[selected_idx].options + engines.json

    // 23. Callback: increment a spin option (+1, bounded by max)
    //
    // PHASE 56: now only modifies the displayed model (`engine-options`) —
    // persistence to engines.json is now deferred to the explicit click
    // on "Validate" (`on_validate_engine_options` further below), at the
    // user's request (Validate/Cancel buttons, following the revert of PHASE 55).
    let window_weak_inc = window.as_weak();
    window.on_inc_option(move |name| {
        let Some(win) = window_weak_inc.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            if opt.name.as_str() != name.as_str() { continue; }
            let current: i32 = opt.value.parse().unwrap_or(0);
            let new_val = if opt.max > 0 { current.saturating_add(1).min(opt.max) }
                          else            { current + 1 };
            let mut updated = opt;
            updated.value = new_val.to_string().into();
            updated.invalid = false; // PHASE 57: any new change clears the red marker
            model.set_row_data(i, updated);
            break;
        }
    });

    // 24. Callback: decrement a spin option (−1, bounded by min) — draft (PHASE 56).
    let window_weak_dec = window.as_weak();
    window.on_dec_option(move |name| {
        let Some(win) = window_weak_dec.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            if opt.name.as_str() != name.as_str() { continue; }
            let current: i32 = opt.value.parse().unwrap_or(0);
            let new_val = current.saturating_sub(1).max(opt.min);
            let mut updated = opt;
            updated.value = new_val.to_string().into();
            updated.invalid = false; // PHASE 57: any new change clears the red marker
            model.set_row_data(i, updated);
            break;
        }
    });

    // 25. Callback: toggle a check option (true ↔ false) — draft (PHASE 56).
    let window_weak_tog = window.as_weak();
    window.on_toggle_option(move |name| {
        let Some(win) = window_weak_tog.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            if opt.name.as_str() != name.as_str() { continue; }
            let new_str = if opt.value.as_str() == "true" { "false" } else { "true" };
            let mut updated = opt;
            updated.value = new_str.into();
            updated.invalid = false; // PHASE 57: any new change clears the red marker
            model.set_row_data(i, updated);
            break;
        }
    });

    // 26. Callback: direct entry of a spin value — draft (PHASE 56).
    //
    // No longer performs any validation here (neither parsing, nor bounding, nor
    // persistence): the typed text is stored as-is in the displayed model,
    // even if it isn't a valid integer. The actual validation (parsing,
    // bounding [min, max], falling back to default if non-numeric) is now
    // done in bulk by "Validate" (`on_validate_engine_options`), across
    // all options of the engine currently being edited.
    let window_weak_setv = window.as_weak();
    window.on_set_option_value(move |name, value_str| {
        let Some(win) = window_weak_setv.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            if opt.name.as_str() != name.as_str() { continue; }
            let mut updated = opt;
            updated.value = value_str.clone();
            updated.invalid = false; // PHASE 57: any new change clears the red marker
            model.set_row_data(i, updated);
            break;
        }
    });

    // 27. Callback: cycle a combo option (forward = true → next, false → previous)
    // — draft (PHASE 56).
    let window_weak_cyc = window.as_weak();
    window.on_cycle_option(move |name, forward| {
        let Some(win) = window_weak_cyc.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            if opt.name.as_str() != name.as_str() { continue; }
            let vars: Vec<&str> = opt.vars.split('|').filter(|s| !s.is_empty()).collect();
            if vars.is_empty() { break; }
            let pos = vars.iter().position(|&v| v == opt.value.as_str()).unwrap_or(0);
            let new_pos = if forward {
                (pos + 1) % vars.len()
            } else if pos == 0 {
                vars.len() - 1
            } else {
                pos - 1
            };
            // `new_str` must be an owned string (`to_owned`), not a `&str`
            // borrowed from `opt.vars`: `opt` is moved right after (`let mut
            // updated = opt;`), which would end this borrow too early
            // (compile error E0505 otherwise).
            let new_str = vars[new_pos].to_owned();
            let mut updated = opt;
            updated.value = new_str.into();
            updated.invalid = false; // PHASE 57: any new change clears the red marker
            model.set_row_data(i, updated);
            break;
        }
    });

    // 27bis. Callback: "Validate" (PHASE 56, extended PHASE 57, fixed PHASE 60,
    // made transactional PHASE 61) — applies all options currently
    // displayed for the selected engine in one go. Each "spin" option
    // is reparsed as an integer.
    //
    // PHASE 61 — the user reported that PHASE 60, while fixing
    // "revalidate without changing anything", was still dangerous: an
    // invalid value in ONE field did correctly block that field, but still let
    // the OTHER valid changes in the same form through and persist.
    // This behavior was explicitly rejected: "we validate all or
    // nothing, we save all or nothing. If something isn't right we don't
    // validate". "Validate" is therefore now transactional: a first pass
    // in READ-ONLY mode checks ALL options; if at least one is
    // invalid, NOTHING is modified (neither the displayed model for valid
    // rows, nor the persisted file) — only the faulty rows are
    // marked `invalid: true` for red display. The second pass,
    // which computes and applies the final values (clamp, persistence), only
    // happens if absolutely everything is valid.
    let engine_list_val = engine_list.clone();
    let window_weak_val = window.as_weak();
    window.on_validate_engine_options(move || {
        let Some(win) = window_weak_val.upgrade() else { return };
        let model = win.get_engine_options();
        let engine_idx = win.get_selected_engine_idx();
        if engine_idx < 0 { return; }
        let engine_idx = engine_idx as usize;

        // Pass 1 (read-only): check the entire form
        // before touching anything. `row_invalid[i]` stores the
        // result so no value gets reparsed a second time.
        let mut row_invalid: Vec<bool> = Vec::with_capacity(model.row_count());
        let mut any_invalid = false;
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else {
                row_invalid.push(false);
                continue;
            };
            let invalid = opt.kind.as_str() == "spin"
                && opt.value.trim().parse::<i32>().is_err();
            if invalid {
                any_invalid = true;
            }
            row_invalid.push(invalid);
        }

        if any_invalid {
            // Nothing is applied or persisted: we just mark the
            // faulty rows (the others remain as-is, without
            // touching their text). The user must fix
            // ALL errors themselves before a new "Validate" can
            // succeed.
            for (i, invalid) in row_invalid.iter().enumerate() {
                let Some(mut opt) = model.row_data(i) else { continue };
                opt.invalid = *invalid;
                model.set_row_data(i, opt);
            }
            win.set_invalid_options_detected(true);
            return;
        }

        // Pass 2 (everything is valid): compute the final values (clamp the
        // "spin" ones) and apply to the model + full persistence.
        let mut new_options: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            let default_str = opt.default.to_string();

            let final_str: String = if opt.kind.as_str() == "spin" {
                let parsed = opt.value.trim().parse::<i32>().unwrap_or(0);
                let clamped = match (opt.min, opt.max) {
                    (mn, mx) if mx > 0 => parsed.clamp(mn.max(0), mx),
                    (mn, _)  if mn > 0 => parsed.max(mn),
                    _                  => parsed.max(0),
                };
                clamped.to_string()
            } else {
                opt.value.to_string()
            };

            if final_str != default_str {
                new_options.insert(opt.name.to_string(), final_str.clone());
            }

            let mut updated = opt;
            updated.value = final_str.into();
            updated.invalid = false;
            model.set_row_data(i, updated);
        }

        let mut engines = engine_list_val.borrow_mut();
        if let Some(eng) = engines.get_mut(engine_idx) {
            eng.options = new_options;
        }
        prefs::save_engines(&engines);

        // Boolean rather than a translated string built here: the displayed
        // message (`Tr.prefs-invalid-options-reset-msg`) thus stays
        // automatically up to date if the user switches language while
        // the options window is open, like every other text in the
        // Preferences panel. We only get here when everything was
        // valid (otherwise an early return above): always `false`.
        win.set_invalid_options_detected(false);
    });

    // 27ter. Callback: "Reset" (PHASE 57) — resets every option in the
    // displayed model to its default value. Persists nothing (draft,
    // like the other changes): the user must then click "Validate"
    // to save — explicit user choice (allows undoing
    // an accidental reset).
    let window_weak_reset = window.as_weak();
    window.on_reset_engine_options(move || {
        let Some(win) = window_weak_reset.upgrade() else { return };
        let model = win.get_engine_options();
        for i in 0..model.row_count() {
            let Some(opt) = model.row_data(i) else { continue };
            let mut updated = opt;
            updated.value = updated.default.clone();
            updated.invalid = false;
            model.set_row_data(i, updated);
        }
        win.set_invalid_options_detected(false);
    });

    // 28. Clock timer — tick every 100 ms → updates the Slint clocks.
    //     Detects the flag falling (time expired) → end-of-game banner.
    //     blink_ctr counter: alternates blink-tick every 500 ms when ≤ 10 s active.
    //     H1 fix: real elapsed time is measured via Instant to avoid drift.
    let chess_clock_timer     = chess_clock.clone();
    let window_weak_timer     = window.as_weak();
    let analysis_for_timer    = analysis.clone();
    let game_bridge_for_timer = game_bridge.clone();
    let controller_for_timer  = controller.clone();
    let blink_ctr             = Rc::new(Cell::new(0u32));
    // Last tick instant — allows measuring the actual gap between two calls.
    let last_tick_instant: Rc<RefCell<Option<std::time::Instant>>> =
        Rc::new(RefCell::new(None));

    let clock_timer = slint::Timer::default();
    clock_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(100),
        {
            let blink_ctr        = blink_ctr.clone();
            let last_tick_instant = last_tick_instant.clone();
            move || {
            let Some(win) = window_weak_timer.upgrade() else { return };

            // If the game is over, stop the clock without displaying anything
            if win.get_is_game_over() {
                chess_clock_timer.borrow_mut().stop();
                *last_tick_instant.borrow_mut() = None;
                return;
            }

            // PHASE 26, Step 2: freeze the clock during variation editing,
            // independently of `game_paused` — the Pause button remains a gesture
            // purely tied to the clock, unrelated to this mode. The tick
            // marker is reset to None to avoid an artificial delta on exit
            // (same logic as for pause/game-over above).
            if controller_for_timer.borrow().is_variation_editing() {
                *last_tick_instant.borrow_mut() = None;
                return;
            }

            let mut clk = chess_clock_timer.borrow_mut();
            if !clk.has_clock() { return; } // no time control → nothing to do

            // Measure the actual elapsed time since the last tick (H1).
            // If the clock is paused, the marker is reset to None to avoid
            // a large fictitious delta when resuming.
            let now = std::time::Instant::now();
            // Cap on the measured delta: this only protects against
            // absurd jumps (system sleep/wake, several minutes) without
            // under-counting a realistic UI stall (native save/open
            // dialog, PGN import...). A cap that's too low
            // (e.g. 500 ms, the previous value) silently favors the
            // active player on every somewhat-long UI thread stall — nearly
            // equivalent to ignoring the actual elapsed-time measurement.
            #[allow(clippy::items_after_statements)] // stays close to its only usage right below
            const MAX_TICK_DELTA_MS: i64 = 30_000;
            let elapsed_ms = if clk.active_player().is_some() {
                let mut guard = last_tick_instant.borrow_mut();
                let ms = guard.map_or(100, |prev| {
                    now.duration_since(prev).as_millis().min(MAX_TICK_DELTA_MS as u128) as i64
                });
                *guard = Some(now);
                ms
            } else {
                *last_tick_instant.borrow_mut() = None;
                0 // tick() will ignore it anyway (active = None)
            };
            clk.tick(elapsed_ms);

            // Update the Slint display
            let wms = clk.white_ms();
            let bms = clk.black_ms();
            win.set_white_clock_text(ChessClock::format(wms).into());
            win.set_black_clock_text(ChessClock::format(bms).into());
            let wms_i32 = wms.max(0).min(i64::from(i32::MAX)) as i32;
            let bms_i32 = bms.max(0).min(i64::from(i32::MAX)) as i32;
            win.set_white_clock_ms(wms_i32);
            win.set_black_clock_ms(bms_i32);

            // Blink ≤ 10 s: alternates every 500 ms (5 ticks × 100 ms)
            let white_critical = win.get_white_clock_active() && wms_i32 > 0 && wms_i32 <= 10_000;
            let black_critical = win.get_black_clock_active() && bms_i32 > 0 && bms_i32 <= 10_000;
            if white_critical || black_critical {
                let c = (blink_ctr.get() + 1) % 10;
                blink_ctr.set(c);
                win.set_blink_tick(c < 5);
            } else {
                blink_ctr.set(0);
                win.set_blink_tick(false);
            }

            // Flag fallen → immediate end of game
            if let Some(white_flagged) = clk.is_flagged() {
                clk.stop();
                let result_key = if white_flagged { "game.result.black_wins" } else { "game.result.white_wins" };
                let result = i18n::translate(result_key);
                let reason = i18n::translate("board.time_forfeit");
                drop(clk); // release before other mutable borrows
                win.set_white_clock_active(false);
                win.set_black_clock_active(false);
                win.set_is_game_over(true);
                win.set_game_over_result(result.into());
                win.set_game_over_reason(reason.into());
                analysis_for_timer.borrow_mut().stop();
                win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
                win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
                win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
                game_bridge_for_timer.borrow_mut().reset();
            }
        }},
    );

    // 29. Startup — 0ms Timer to run as soon as the first Slint event loop turn happens.
    //     (formerly #22/23 — renamed to avoid numbering conflicts)
    //
    //   IMPORTANT: set_show_setup_wizard() before window.run() does not work
    //   (the component isn't rendered yet). A SingleShot Timer with zero
    //   duration is used to trigger the action right at the start of the event loop.
    //
    //   Behavior: very first launch (no saved language,
    //   `prefs::has_saved_lang()` false) → show the language picker
    //   (`LanguageSetupScreen`, no Close button, the user MUST
    //   choose); this then automatically chains into the wizard or the
    //   recap via `on_language_chosen` → `invoke_open_new_game()`
    //   (see callback above). Subsequent launches: always start
    //   directly on the "New game" panel (wizard) as before —
    //   language and preferences also remain accessible via the
    //   "⚙ Preferences" card in the wizard.
    let win_startup   = window.as_weak();
    let startup_timer = slint::Timer::default();
    startup_timer.start(
        slint::TimerMode::SingleShot,
        std::time::Duration::ZERO,
        move || {
            if let Some(win) = win_startup.upgrade() {
                if prefs::has_saved_lang() {
                    win.set_wizard_step(0);
                    win.set_show_setup_wizard(true);
                } else {
                    win.set_show_language_setup(true);
                }
            }
        },
    );

    // 30. Callback: launch a tournament
    //     kind        : "roundrobin" | "gauntlet"
    //     gpp         : games per pair (1 or 2)
    //     engine_mask : bitmask — bit i = engine i selected in saved-engines
    //     tc_idx : time control index (0-3 = movetime, 4-10 = Fischer/PerGame clocks)

    /// Converts the tournament wizard's time-control index to a `TimeControl`.
    ///
    /// | idx | time control                |
    /// |-----|-----------------------------|
    /// |  0  | MoveTime 1 s/move           |
    /// |  1  | MoveTime 3 s/move           |
    /// |  2  | MoveTime 5 s/move           |
    /// |  3  | MoveTime 10 s/move          |
    /// |  4  | Bullet 1+0  (PerGame 60 s)  |
    /// |  5  | Bullet 2+1  (Fischer)        |
    /// |  6  | Blitz  3+2  (Fischer)        |
    /// |  7  | Blitz  5+0  (PerGame 300 s)  |
    /// |  8  | Rapid 10+5 (Fischer)        |
    /// |  9  | Rapid 15+10 (Fischer)       |
    /// | 10  | Classical 90+30 (Fischer)    |
    // Clippy: local helper function for this callback, defined after the
    // preceding `let`s to stay close to its only call site.
    #[allow(clippy::items_after_statements)]
    fn tournament_tc_from_idx(idx: i32) -> TimeControl {
        match idx {
            0  => TimeControl::MoveTime(1_000),
            2  => TimeControl::MoveTime(5_000),
            3  => TimeControl::MoveTime(10_000),
            4  => TimeControl::BULLET_1_0,
            5  => TimeControl::BULLET_2_1,
            6  => TimeControl::BLITZ_3_2,
            7  => TimeControl::BLITZ_5_0,
            8  => TimeControl::RAPID_10_5,
            9  => TimeControl::RAPID_15_10,
            10 => TimeControl::CLASSICAL_90_30,
            _  => TimeControl::MoveTime(3_000),
        }
    }

    let win_tourn      = window.as_weak();
    let eng_tourn      = engine_list.clone();
    let runner_tourn   = tournament_runner.clone();
    let bridge_tourn   = game_bridge.clone();
    let ctrl_tourn     = controller.clone();
    let analysis_tourn = analysis.clone();
    let scores_tourn   = score_history.clone();
    let clock_tourn    = chess_clock.clone();
    let lang_tourn     = lang_cell.clone();

    window.on_start_tournament(move |kind, gpp, engine_mask, tc_idx| {
        let Some(win) = win_tourn.upgrade() else { return };
        let lang = *lang_tourn.borrow();
        let tc = tournament_tc_from_idx(tc_idx);

        // Build the list of selected engines from the bitmask
        let engines: Vec<(String, String)> = {
            let list = eng_tourn.borrow();
            (0..list.len().min(16))
                .filter(|&i| ((engine_mask as u32) >> i) & 1 == 1)
                .map(|i| (list[i].name.clone(), list[i].path.clone()))
                .collect()
        };
        if engines.len() < 2 {
            eprintln!("[Tournament] Moins de 2 moteurs sélectionnés — annulé.");
            return;
        }

        let t_kind = match kind.as_str() {
            "gauntlet" => tournament::TournamentKind::Gauntlet,
            _          => tournament::TournamentKind::RoundRobin,
        };

        // Tournament name (short UNIX timestamp for uniqueness)
        let name = {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("Tournoi #{}", secs % 100_000)
        };

        let config = tournament::TournamentConfig {
            name:           name.clone(),
            kind:           t_kind,
            engines,
            games_per_pair: gpp as u32,
            movetime_ms:    0, // time control managed via TimeControl / ChessClock
        };
        if config.validate().is_err() {
            eprintln!("[Tournament] Config invalide — annulé.");
            return;
        }

        gui::debug_log::new_game();
        gui::debug_log::log_event("game_started", &serde_json::json!({
            "source": "tournament",
            "name": config.name,
            "kind": config.kind.as_str(),
            "nb_engines": config.engines.len(),
            "games_per_pair": config.games_per_pair,
        }));

        // Create the entry in the database. The connection is kept
        // (see TournamentRunner::db_conn) and reused for every game
        // of the tournament instead of being reopened each time.
        let db = tournament_db_path();
        let db_conn = match db::schema::open_and_migrate(&db) {
            Err(e) => {
                eprintln!("[Tournament] DB inaccessible : {e}");
                return;
            }
            Ok(conn) => conn,
        };
        let tournament_id = match db::repository::tournament_repo::create_tournament(
            &db_conn, &config.name, config.kind.as_str(),
        ) {
            Ok(id) => id,
            Err(e) => { eprintln!("[Tournament] Erreur DB : {e}"); return; }
        };

        // Reset the current game state
        analysis_tourn.borrow_mut().stop();
        win.set_engine_pv_lines_white(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
        win.set_engine_pv_lines_black(ModelRc::new(VecModel::<gui::PvLine>::from(vec![])));
        win.set_pv_selected_rank_white(0); win.set_pv_selected_rank_black(0);
        clock_tourn.borrow_mut().stop();
        scores_tourn.borrow_mut().clear();
        win.set_score_history(ModelRc::new(VecModel::from(Vec::<ScoreBar>::new())));
        win.set_white_curve_path("".into());
        win.set_black_curve_path("".into());

        ctrl_tourn.borrow_mut().reset();
        {
            let ctrl = ctrl_tourn.borrow();
            refresh_game_state(&win, &ctrl, lang);
        }

        // Enable tournament mode
        win.set_is_tournament_mode(true);
        win.set_current_game_mode(2); // M vs M

        // Create the runner and fetch the first game
        let runner = TournamentRunner::new(config, tournament_id, db, db_conn, tc);
        *runner_tourn.borrow_mut() = Some(runner);

        // Initial standings (all at 0) + reset finished state
        win.set_tournament_finished(false);
        {
            let tr_opt = runner_tourn.borrow();
            if let Some(tr) = tr_opt.as_ref() {
                push_tournament_standings(&win, tr);
            }
        }

        let first_game = {
            let tr_opt = runner_tourn.borrow();
            tr_opt.as_ref().and_then(|tr| tr.state.next_game().cloned())
        };
        let Some(first) = first_game else {
            win.set_is_tournament_mode(false);
            return;
        };

        // Parameters of the first game (defensive indexing: see
        // equivalent comment in on_engine_move_ready for subsequent games).
        // Code audit 04/07/2026, point 2: defensive `if let` instead of an
        // `unwrap()` — aligned with the style already used at nearby sites.
        let first_engines = {
            let tr_opt = runner_tourn.borrow();
            match tr_opt.as_ref() {
                Some(tr) => match (
                    tr.state.config.engines.get(first.white),
                    tr.state.config.engines.get(first.black),
                ) {
                    (Some(w), Some(b)) => Some((w.0.clone(), w.1.clone(), b.0.clone(), b.1.clone())),
                    _ => None,
                },
                None => None,
            }
        };
        let Some((wname, wpath, bname, bpath)) = first_engines else {
            eprintln!(
                "[Tournament] Indice moteur invalide pour la première partie (white={}, black={}) — annulé.",
                first.white, first.black
            );
            win.set_is_tournament_mode(false);
            return;
        };

        // Clock: initialize from the chosen time control
        *clock_tourn.borrow_mut() = ChessClock::new(&tc);
        let show_clocks = tc.use_player_clock();
        win.set_show_clocks(show_clocks);
        if show_clocks {
            clock_tourn.borrow_mut().start(true); // White moves first
            push_clock_to_window(&win, &clock_tourn.borrow());
        }

        // MvM GameConfig with the chosen time control
        let mut gc = GameConfig::engine_vs_engine(&wpath, &bpath);
        if let Some(e) = gc.white_engine.as_mut() {
            e.time_control = tc;
        }
        if let Some(e) = gc.black_engine.as_mut() {
            e.time_control = tc;
        }

        // Initialize the bridge and trigger the first move
        bridge_tourn.borrow_mut().reset();
        bridge_tourn.borrow_mut().init(&gc, &win.as_weak());

        // Current game in the tournament panel
        win.set_tournament_current_white(wname.clone().into());
        win.set_tournament_current_black(bname.clone().into());

        // Player names (black/white sidebar)
        win.set_white_player_name(wname.into());
        win.set_black_player_name(bname.into());

        let fen = ctrl_tourn.borrow().current_fen();
        let limits_opt = build_go_limits(&clock_tourn.borrow());
        let _ = bridge_tourn.borrow().trigger_if_engine_turn(true, fen, limits_opt);
    });

    // 31. Callback: stop the current tournament
    let win_stop     = window.as_weak();
    let runner_stop  = tournament_runner.clone();
    let bridge_stop  = game_bridge.clone();
    let ctrl_stop    = controller.clone();
    let lang_stop    = lang_cell.clone();

    window.on_stop_tournament(move || {
        let Some(win) = win_stop.upgrade() else { return };
        let lang = *lang_stop.borrow();

        // Stop the bridge
        bridge_stop.borrow_mut().reset();

        // Clear the tournament state
        *runner_stop.borrow_mut() = None;

        // Reset controller + board
        ctrl_stop.borrow_mut().reset();
        {
            let ctrl = ctrl_stop.borrow();
            refresh_game_state(&win, &ctrl, lang);
        }

        // Clear the tournament panel
        win.set_tournament_standings(ModelRc::new(VecModel::from(
            Vec::<TournamentStanding>::new())));
        win.set_tournament_current_white("".into());
        win.set_tournament_current_black("".into());
        win.set_tournament_games_played(0);
        win.set_tournament_total_games(0);
        win.set_tournament_progress(0.0);
        win.set_tournament_finished(false);

        win.set_is_tournament_mode(false);
    });

    // 24. Event loop (blocks until the window closes)
    window.run()
}
