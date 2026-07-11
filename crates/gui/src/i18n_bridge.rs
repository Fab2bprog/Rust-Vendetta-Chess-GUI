//! Bridge between the Rust i18n system and the Slint global [`Tr`].
//!
//! A single call to [`apply_translations`] feeds all the properties of
//! the `Tr` global from the embedded TOML translations. Slint's reactivity
//! ensures that all interface bindings recompute immediately.
//!
//! # Usage example (Phase 6.3+)
//!
//! ```no_run
//! use gui::{AppWindow, i18n_bridge};
//! use i18n::Lang;
//! use slint::ComponentHandle as _;
//!
//! let window = AppWindow::new().unwrap();
//! i18n_bridge::apply_translations(&window.global::<gui::Tr>(), Lang::Fr);
//! window.run().unwrap();
//!
//! // Hot language switch:
//! i18n_bridge::apply_translations(&window.global::<gui::Tr>(), Lang::En);
//! ```

use i18n::Lang;
use crate::Tr;

// ---------------------------------------------------------------------------
// Canonical list of i18n keys
// ---------------------------------------------------------------------------

/// The 375 i18n keys used in the interface (11/07/2026, robustness audit
/// fix: +2 — `dialog.unexpected_error_title`/`_desc`, generic fallback
/// dialog shown by `run_guarded_thread`'s `on_panic` callbacks when a
/// background import/analysis thread panics instead of returning `Err`
/// normally, see `Analyse_Projet/AUDIT_ROBUSTESSE_2026-07-11.md` finding
/// 1.2; previous change same day: +1 — `app.quit_confirm_import_warning`,
/// warning shown in the quit confirmation modal when a reference-database
/// or puzzle import is still running in the background, see
/// `docs/ARCHITECTURE.md`; previous bugfix 09/07/2026: +2 —
/// `dialog.clear_reference_base_title`/`_desc`, missing "Clear base"
/// button reported by the user; previous step 11: +1 —
/// `refbrowser.col_opening`, header of the "Opening" column of the
/// game list; previous step 9: +9 — 6 `gamedetail.*` keys tied to
/// `Tr` properties, plus 3 `status.game_analysis_*` keys translated
/// directly via `i18n::translate()`, see below), in the same order as the
/// properties of the Slint `Tr` global, except for the 55 `dialog.*` keys
/// (native `rfd::MessageDialog`/`FileDialog` dialogs from `main.rs`) and the
/// 27 `board.time_forfeit`/`status.*` keys (one-off status messages from
/// `main.rs`), which are all translated via `i18n::translate()` called
/// directly in Rust rather than via a `Tr` property — this content is
/// outside the Slint system.
///
/// Used in the tests to verify that all keys are present in the 40 locales.
pub const ALL_KEYS: &[&str] = &[
    // Application
    "app.name",
    "app.version",
    // File menu
    "menu.file",
    "menu.file.new_game",
    "menu.file.open_pgn",
    "menu.file.save_pgn",
    "menu.file.quit",
    // Edit menu
    "menu.edit",
    "menu.edit.undo",
    "menu.edit.copy_pgn",
    "menu.edit.copy_fen",
    // View menu
    "menu.view",
    "menu.view.rotate_board",
    "menu.view.coordinates",
    "menu.view.legal_moves",
    // Engine menu
    "menu.engine",
    "menu.engine.configure",
    "menu.engine.analyze",
    "menu.engine.stop",
    "menu.engine.multi_pv",
    // Tournament menu
    "menu.tournament",
    "menu.tournament.new",
    // Language / Help menu
    "menu.language",
    "menu.help",
    "menu.help.about",
    // Chessboard
    "board.white",
    "board.black",
    "board.turn.white",
    "board.turn.black",
    "board.check",
    "board.checkmate",
    "board.stalemate",
    "board.draw",
    // Results
    "game.result.white_wins",
    "game.result.black_wins",
    "game.result.draw",
    "game.result.ongoing",
    // Analysis
    "analysis.depth",
    "analysis.score",
    "analysis.nodes",
    "analysis.time",
    "analysis.pv",
    "analysis.best_move",
    "analysis.mate_in",
    // Engine
    "engine.ready",
    "engine.thinking",
    "engine.stopped",
    "engine.no_engine",
    // LAN network
    "network.host",
    "network.join",
    "network.connecting",
    "network.connected",
    "network.disconnected",
    "network.waiting",
    "network.your_turn",
    "network.opponent_turn",
    // Promotion
    "promo.choose",
    // Buttons
    "btn.ok",
    "btn.cancel",
    "btn.close",
    "btn.apply",
    "btn.start",
    "btn.stop",
    "btn.new",
    "btn.save",
    "btn.open",
    // Header bar tooltips
    "tooltip.export_pgn",
    "tooltip.import_pgn",
    "tooltip.print_game",
    "tooltip.export_board_png",
    "tooltip.undo",
    "tooltip.copy_fen",
    "tooltip.paste_fen",
    "tooltip.flip_board",
    "tooltip.assist_mode",
    "tooltip.edit_position",
    "tooltip.hint",
    "tooltip.theme",
    "tooltip.prefs",
    // NAG annotation tooltips (context menu on a move)
    "tooltip.nag_brilliant",
    "tooltip.nag_good",
    "tooltip.nag_interesting",
    "tooltip.nag_dubious",
    "tooltip.nag_mistake",
    "tooltip.nag_blunder",
    // Variation-editing-mode banner (PHASE 26)
    "variation.banner_viewing",
    "variation.banner_editing",
    "btn.variation_create",
    "btn.variation_exit",
    // Preferences → "Misc" tab: debug mode (PHASE 26sexies)
    "prefs.misc_title",
    "prefs.debug_mode_label",
    "prefs.debug_mode_explanation",
    // "New Game" wizard → step 0 (mode choice)
    "wizard.mode_human_vs_human",
    "wizard.mode_human_vs_engine",
    "wizard.mode_engine_vs_engine",
    "wizard.mode_tournament",
    "wizard.mode_puzzle",
    "wizard.mode_reference_base",
    "wizard.hint_need_2_engines",
    "wizard.hint_need_puzzles",
    "wizard.hint_need_engine",
    "wizard.hint_need_base",
    "wizard.hint_add_engine_here",
    // Reference games database browsing screen (PHASE 82, step 7)
    "refbrowser.filter_player",
    "refbrowser.filter_elo_min",
    "refbrowser.filter_elo_max",
    "refbrowser.filter_date_from",
    "refbrowser.filter_date_to",
    "refbrowser.filter_eco",
    "refbrowser.search_button",
    "refbrowser.reset_button",
    "refbrowser.col_white",
    "refbrowser.col_black",
    "refbrowser.col_result",
    "refbrowser.col_date",
    "refbrowser.col_elo",
    "refbrowser.col_opening",
    "refbrowser.no_results",
    "refbrowser.page_label",
    "refbrowser.prev_page",
    "refbrowser.next_page",
    "refbrowser.results_suffix",
    // Opening tree (PHASE 82, step 8)
    "refbrowser.tab_games",
    "refbrowser.tab_tree",
    "refbrowser.tree_start_position",
    "refbrowser.tree_back",
    "refbrowser.tree_restart",
    "refbrowser.tree_apply",
    "refbrowser.tree_col_move",
    "refbrowser.tree_col_white_pct",
    "refbrowser.tree_col_draw_pct",
    "refbrowser.tree_col_black_pct",
    "refbrowser.tree_games_suffix",
    "refbrowser.tree_no_moves",
    // Ergonomics follow-up 10/07/2026 — bridge from opening tree → game list
    "refbrowser.tree_subtitle",
    "refbrowser.tree_check_all",
    "refbrowser.tree_uncheck_all",
    "refbrowser.tree_list_games",
    "refbrowser.tree_filter_chip_prefix",
    // Game detail view (PHASE 82, step 9)
    "gamedetail.title",
    "gamedetail.moves_label",
    "gamedetail.eval_label",
    "gamedetail.analyze_button",
    "gamedetail.deepen_button",
    "gamedetail.not_analyzed",
    "gamedetail.start_from_here",
    // Ergonomics follow-up 10/07/2026: selected-move info block.
    "gamedetail.score_label",
    "gamedetail.quality_excellent",
    "gamedetail.quality_good",
    "gamedetail.quality_inaccuracy",
    "gamedetail.quality_mistake",
    "gamedetail.quality_blunder",
    "gamedetail.depth_label",
    "gamedetail.pass_quick",
    "gamedetail.pass_deep",
    "gamedetail.best_move_label",
    "status.game_analysis_in_progress",
    "status.game_analysis_progress",
    "status.game_analysis_no_engine",
    // "About" window (PHASE 30, completed here)
    "about.section_author",
    "about.section_license",
    "about.section_technologies",
    "about.lang_rust",
    "about.ui_slint",
    "about.made_with",
    // "New Game" wizard → step 1 (config) + summary screen
    "wizard.step1_time_title",
    "wizard.tc_cat_unlimited",
    "wizard.tc_cat_bullet",
    "wizard.tc_cat_blitz",
    "wizard.tc_cat_rapid",
    "wizard.tc_cat_classical",
    "wizard.step1_cadence_title",
    "wizard.tc_desc_unlimited",
    "wizard.tc_desc_bullet1",
    "wizard.tc_desc_bullet2",
    "wizard.tc_desc_blitz1",
    "wizard.tc_desc_blitz2",
    "wizard.tc_desc_rapid1",
    "wizard.tc_desc_rapid2",
    "wizard.tc_desc_classical",
    "wizard.handicap_toggle",
    "wizard.your_color_title",
    "wizard.color_random",
    "wizard.engine_opponent",
    "wizard.engine_white",
    "wizard.engine_black",
    "wizard.puzzle_goal_title",
    "wizard.puzzle_goal_explanation",
    "wizard.puzzle_no_hint",
    "wizard.puzzle_with_theme",
    "wizard.puzzle_hint_button_title",
    "wizard.puzzle_hint_button_explanation",
    "wizard.state_active",
    "wizard.state_inactive",
    "wizard.start_position_title",
    "wizard.pgn_load_placeholder",
    "wizard.editor_define_placeholder",
    "wizard.editor_custom_position",
    "wizard.base_define_placeholder",
    "wizard.base_custom_position",
    "wizard.nav_previous",
    "wizard.quit_confirm_title",
    "wizard.quit_confirm_body",
    "wizard.btn_no",
    "wizard.btn_yes_quit",
    "wizard.btn_next",
    "wizard.no_engine_configured",
    "wizard.recap_title",
    "wizard.recap_back",
    "wizard.recap_new_config",
    "wizard.btn_replay",
    "wizard.recap_mode_prefix",
    "wizard.recap_no_engine",
    "wizard.recap_two_engines",
    "wizard.recap_human_prefix",
    // Preferences (full panel)
    "prefs.tab_language",
    "prefs.tab_engines",
    "prefs.tab_openings",
    "prefs.tab_puzzles",
    "prefs.tab_reference_base",
    "prefs.tab_appearance",
    "prefs.tab_reset",
    "prefs.tab_misc",
    "prefs.engines_title",
    "prefs.btn_add_engine",
    "prefs.engines_empty",
    "prefs.engine_click_hint",
    "prefs.uci_params_title",
    "prefs.engine_querying",
    "prefs.options_count_suffix",
    "prefs.hide_advanced_options",
    "prefs.show_advanced_options",
    "prefs.engines_auto_removed_note",
    "prefs.hint_engine_title",
    "prefs.hint_engine_explanation",
    "prefs.none_label",
    "prefs.uci_button_at_startup",
    "prefs.btn_reset_options",
    "prefs.invalid_options_reset_msg",
    "prefs.openings_title",
    "prefs.openings_explanation",
    "prefs.btn_load",
    "prefs.puzzles_title",
    "prefs.puzzles_explanation",
    "prefs.puzzles_count_suffix",
    "prefs.puzzles_none_imported",
    "prefs.reference_base_title",
    "prefs.reference_base_explanation",
    "prefs.reference_base_count_suffix",
    "prefs.reference_base_none_imported",
    "prefs.btn_unload",
    "prefs.btn_import",
    "prefs.reset_title",
    "prefs.reset_explanation",
    "prefs.reset_button",
    "prefs.reset_confirm_title",
    "prefs.reset_confirm_body",
    "prefs.btn_yes_reset",
    "prefs.appearance_title",
    "prefs.theme_label",
    "prefs.theme_light",
    "prefs.theme_dark",
    "prefs.appearance_more_soon",
    // Engine Tournament (wizard + tracking panel)
    "tournament.title",
    "tournament.step1_title",
    "tournament.step2_title",
    "tournament.step3_title",
    "tournament.format_title",
    "tournament.format_roundrobin_label",
    "tournament.format_roundrobin_desc",
    "tournament.format_gauntlet_label",
    "tournament.format_gauntlet_desc",
    "tournament.games_per_pair_title",
    "tournament.gpp_one",
    "tournament.gpp_two",
    "tournament.engines_title",
    "tournament.engines_selected_suffix",
    "tournament.engines_select_hint",
    "tournament.need_2_engines",
    "tournament.challenger_label",
    "tournament.gauntlet_note",
    "tournament.quick_test_title",
    "tournament.timed_cadences_title",
    "tournament.recap_title",
    "tournament.recap_engines_suffix",
    "tournament.recap_games_per_pair_suffix",
    "tournament.finished_title",
    "tournament.in_progress_title",
    "tournament.games_played_suffix",
    "tournament.col_engine",
    "tournament.col_points",
    "tournament.col_wins",
    "tournament.col_draws",
    "tournament.col_losses",
    "tournament.btn_stop",
    // Position editor
    "posedit.title",
    "posedit.eraser",
    "posedit.clear_all",
    "posedit.turn_label",
    "posedit.castling_rights",
    "posedit.reset_position",
    "posedit.btn_validate",
    "posedit.btn_use_position",
    // Remaining app.slint content (NAG menu, puzzle end, pause, quit confirmation)
    "app.nag_add_comment",
    "app.nag_promote_variation",
    "app.nag_delete_variation",
    "app.puzzle_show_solution",
    "app.puzzle_next",
    "app.pause_pause",
    "app.pause_resume",
    "app.pause_banner_title",
    "app.quit_confirm_title",
    "app.quit_confirm_body",
    "app.quit_confirm_import_warning",

    // ── Native dialogs (rfd::MessageDialog / FileDialog) — main.rs, direct Rust ──
    // Generic fallback shown by `run_guarded_thread`'s `on_panic` callbacks
    // (AUDIT_ROBUSTESSE 11/07/2026, finding 1.2: a background import/analysis
    // thread panicking instead of returning `Err` normally) — no specific
    // translated message exists for an unforeseen internal panic.
    "dialog.unexpected_error_title",
    "dialog.unexpected_error_desc",
    "dialog.db_unavailable_title",
    "dialog.no_puzzle_available_title",
    "dialog.no_puzzle_available_desc",
    "dialog.db_error_title",
    "dialog.invalid_puzzle_title",
    "dialog.invalid_puzzle_desc",
    "dialog.load_book_white_title",
    "dialog.load_book_black_title",
    "dialog.polyglot_filter_label",
    "dialog.import_impossible_title",
    "dialog.book_import_failed_desc",
    "dialog.engine_import_failed_desc",
    // PHASE 64: confirmation for removing an engine (removal from the
    // list + question about deleting the executable file on disk,
    // to prevent it from being redetected on the next startup).
    "dialog.remove_engine_confirm_title",
    "dialog.remove_engine_confirm_desc",
    "dialog.engine_delete_failed_desc",
    "dialog.book_loaded_title",
    "dialog.book_loaded_desc",
    "dialog.invalid_file_title",
    "dialog.invalid_book_file_desc",
    "dialog.invalid_puzzle_file_desc",
    "dialog.import_puzzles_title",
    "dialog.puzzles_filter_label",
    "dialog.import_finished_title",
    "dialog.import_finished_desc",
    "dialog.unload_puzzles_title",
    "dialog.unload_puzzles_desc",
    "dialog.clear_reference_base_title",
    "dialog.clear_reference_base_desc",
    "dialog.import_reference_base_title",
    "dialog.reference_pgn_filter_label",
    "dialog.import_reference_base_finished_desc",
    "dialog.invalid_reference_pgn_file_desc",
    "dialog.generic_error_title",
    "dialog.invalid_fen_title",
    "dialog.invalid_fen_desc",
    "dialog.load_fen_confirm_title",
    "dialog.load_fen_confirm_desc",
    "dialog.invalid_position_title",
    "dialog.need_one_white_king_desc",
    "dialog.need_one_black_king_desc",
    "dialog.illegal_position_desc",
    "dialog.save_pgn_title",
    "dialog.save_pdf_title",
    "dialog.save_error_title",
    "dialog.save_pgn_error_desc",
    "dialog.save_pdf_error_desc",
    "dialog.open_pgn_title",
    "dialog.select_engine_title",
    "dialog.invalid_executable_title",
    "dialog.invalid_executable_desc",
    "dialog.wizard_open_pgn_title",
    "dialog.pgn_filter_label",
    "dialog.pdf_filter_label",

    // ── One-off status messages — main.rs, direct Rust (Batch 8) ──
    "board.time_forfeit",
    "status.puzzle_turn_white",
    "status.puzzle_turn_black",
    "status.puzzle_theme_subtitle",
    "status.error_suffix_one_now",
    "status.error_suffix_many_now",
    "status.error_suffix_one_after",
    "status.error_suffix_many_after",
    "status.puzzle_solved_title",
    "status.puzzle_broken_title",
    "status.puzzle_revealed_title",
    "status.puzzle_failed_title",
    "status.no_puzzle_attempts",
    "status.puzzle_stats_summary",
    "status.puzzle_import_in_progress",
    "status.puzzle_import_lines_processed",
    "status.reference_import_in_progress",
    "status.reference_import_games_processed",
    "status.puzzle_feedback_incorrect",
    "status.puzzle_feedback_correct",
    "status.puzzle_feedback_solved",
    "status.puzzle_feedback_abandoned",
    "status.book_move_single",
    "status.book_move_multiple",
];

// ---------------------------------------------------------------------------
// Main bridge
// ---------------------------------------------------------------------------

/// Applies the translations of `lang` to the Slint global [`Tr`].
///
/// Changes the i18n system's current language then feeds each of the 243
/// properties of the global. All Slint bindings that depend on these
/// properties recompute automatically. (The 44 `dialog.*` keys and the
/// 22 `board.time_forfeit`/`status.*` keys are not affected by this
/// function: they are translated on the fly via `i18n::translate()`
/// directly in `main.rs`, at the moment each native dialog or status
/// message is displayed.)
///
/// # Call before `window.run()`
///
/// At startup, the language must be applied before showing the window
/// to avoid a flash of empty strings.
// clippy::pedantic (too_many_lines): function deliberately long — it's a
// simple linear sequence of ~243 `tr.set_X(i18n::translate("key"))` assignments,
// one per property of the Slint `Tr` global, grouped by menu/panel section.
// Splitting it into sub-functions would bring no readability or
// testability benefit (each line is independent and already covered by
// `test_all_keys_translated_in_all_langs`) and would introduce a risk of a typo
// when moving ~250 mechanical lines. Checked on 05/07/2026 following
// `cargo clippy --workspace --all-targets --all-features -- -W clippy::pedantic`.
#[allow(clippy::too_many_lines)]
pub fn apply_translations(tr: &Tr, lang: Lang) {
    i18n::set_lang(lang);

    // ── Application ──────────────────────────────────────────────────────────
    tr.set_app_name(i18n::translate("app.name").into());
    tr.set_app_version(i18n::translate("app.version").into());

    // ── File menu ─────────────────────────────────────────────────────────────
    tr.set_menu_file(i18n::translate("menu.file").into());
    tr.set_menu_file_new_game(i18n::translate("menu.file.new_game").into());
    tr.set_menu_file_open_pgn(i18n::translate("menu.file.open_pgn").into());
    tr.set_menu_file_save_pgn(i18n::translate("menu.file.save_pgn").into());
    tr.set_menu_file_quit(i18n::translate("menu.file.quit").into());

    // ── Edit menu ─────────────────────────────────────────────────────────────
    tr.set_menu_edit(i18n::translate("menu.edit").into());
    tr.set_menu_edit_undo(i18n::translate("menu.edit.undo").into());
    tr.set_menu_edit_copy_pgn(i18n::translate("menu.edit.copy_pgn").into());
    tr.set_menu_edit_copy_fen(i18n::translate("menu.edit.copy_fen").into());

    // ── View menu ─────────────────────────────────────────────────────────────
    tr.set_menu_view(i18n::translate("menu.view").into());
    tr.set_menu_view_rotate_board(i18n::translate("menu.view.rotate_board").into());
    tr.set_menu_view_coordinates(i18n::translate("menu.view.coordinates").into());
    tr.set_menu_view_legal_moves(i18n::translate("menu.view.legal_moves").into());

    // ── Engine menu ───────────────────────────────────────────────────────────
    tr.set_menu_engine(i18n::translate("menu.engine").into());
    tr.set_menu_engine_configure(i18n::translate("menu.engine.configure").into());
    tr.set_menu_engine_analyze(i18n::translate("menu.engine.analyze").into());
    tr.set_menu_engine_stop(i18n::translate("menu.engine.stop").into());
    tr.set_menu_engine_multi_pv(i18n::translate("menu.engine.multi_pv").into());

    // ── Tournament menu ───────────────────────────────────────────────────────
    tr.set_menu_tournament(i18n::translate("menu.tournament").into());
    tr.set_menu_tournament_new(i18n::translate("menu.tournament.new").into());

    // ── Language / Help menu ──────────────────────────────────────────────────
    tr.set_menu_language(i18n::translate("menu.language").into());
    tr.set_menu_help(i18n::translate("menu.help").into());
    tr.set_menu_help_about(i18n::translate("menu.help.about").into());

    // ── Chessboard ───────────────────────────────────────────────────────────
    tr.set_board_white(i18n::translate("board.white").into());
    tr.set_board_black(i18n::translate("board.black").into());
    tr.set_board_turn_white(i18n::translate("board.turn.white").into());
    tr.set_board_turn_black(i18n::translate("board.turn.black").into());
    tr.set_board_check(i18n::translate("board.check").into());
    tr.set_board_checkmate(i18n::translate("board.checkmate").into());
    tr.set_board_stalemate(i18n::translate("board.stalemate").into());
    tr.set_board_draw(i18n::translate("board.draw").into());

    // ── Results ───────────────────────────────────────────────────────────────
    tr.set_game_result_white_wins(i18n::translate("game.result.white_wins").into());
    tr.set_game_result_black_wins(i18n::translate("game.result.black_wins").into());
    tr.set_game_result_draw(i18n::translate("game.result.draw").into());
    tr.set_game_result_ongoing(i18n::translate("game.result.ongoing").into());

    // ── Analysis ──────────────────────────────────────────────────────────────
    tr.set_analysis_depth(i18n::translate("analysis.depth").into());
    tr.set_analysis_score(i18n::translate("analysis.score").into());
    tr.set_analysis_nodes(i18n::translate("analysis.nodes").into());
    tr.set_analysis_time(i18n::translate("analysis.time").into());
    tr.set_analysis_pv(i18n::translate("analysis.pv").into());
    tr.set_analysis_best_move(i18n::translate("analysis.best_move").into());
    tr.set_analysis_mate_in(i18n::translate("analysis.mate_in").into());

    // ── Engine ────────────────────────────────────────────────────────────────
    tr.set_engine_ready(i18n::translate("engine.ready").into());
    tr.set_engine_thinking(i18n::translate("engine.thinking").into());
    tr.set_engine_stopped(i18n::translate("engine.stopped").into());
    tr.set_engine_no_engine(i18n::translate("engine.no_engine").into());

    // ── LAN network ───────────────────────────────────────────────────────────
    tr.set_network_host(i18n::translate("network.host").into());
    tr.set_network_join(i18n::translate("network.join").into());
    tr.set_network_connecting(i18n::translate("network.connecting").into());
    tr.set_network_connected(i18n::translate("network.connected").into());
    tr.set_network_disconnected(i18n::translate("network.disconnected").into());
    tr.set_network_waiting(i18n::translate("network.waiting").into());
    tr.set_network_your_turn(i18n::translate("network.your_turn").into());
    tr.set_network_opponent_turn(i18n::translate("network.opponent_turn").into());

    // ── Promotion ─────────────────────────────────────────────────────────────
    tr.set_promo_choose(i18n::translate("promo.choose").into());

    // ── Buttons ───────────────────────────────────────────────────────────────
    tr.set_btn_ok(i18n::translate("btn.ok").into());
    tr.set_btn_cancel(i18n::translate("btn.cancel").into());
    tr.set_btn_close(i18n::translate("btn.close").into());
    tr.set_btn_apply(i18n::translate("btn.apply").into());
    tr.set_btn_start(i18n::translate("btn.start").into());
    tr.set_btn_stop(i18n::translate("btn.stop").into());
    tr.set_btn_new(i18n::translate("btn.new").into());
    tr.set_btn_save(i18n::translate("btn.save").into());
    tr.set_btn_open(i18n::translate("btn.open").into());

    // ── Header bar tooltips ───────────────────────────────────────────────────
    tr.set_tooltip_export_pgn(i18n::translate("tooltip.export_pgn").into());
    tr.set_tooltip_import_pgn(i18n::translate("tooltip.import_pgn").into());
    tr.set_tooltip_print_game(i18n::translate("tooltip.print_game").into());
    tr.set_tooltip_export_board_png(i18n::translate("tooltip.export_board_png").into());
    tr.set_tooltip_undo(i18n::translate("tooltip.undo").into());
    tr.set_tooltip_copy_fen(i18n::translate("tooltip.copy_fen").into());
    tr.set_tooltip_paste_fen(i18n::translate("tooltip.paste_fen").into());
    tr.set_tooltip_flip_board(i18n::translate("tooltip.flip_board").into());
    tr.set_tooltip_assist_mode(i18n::translate("tooltip.assist_mode").into());
    tr.set_tooltip_edit_position(i18n::translate("tooltip.edit_position").into());
    tr.set_tooltip_hint(i18n::translate("tooltip.hint").into());
    tr.set_tooltip_theme(i18n::translate("tooltip.theme").into());
    tr.set_tooltip_prefs(i18n::translate("tooltip.prefs").into());

    // ── NAG annotation tooltips (context menu on a move) ─────────────────────
    tr.set_tooltip_nag_brilliant(i18n::translate("tooltip.nag_brilliant").into());
    tr.set_tooltip_nag_good(i18n::translate("tooltip.nag_good").into());
    tr.set_tooltip_nag_interesting(i18n::translate("tooltip.nag_interesting").into());
    tr.set_tooltip_nag_dubious(i18n::translate("tooltip.nag_dubious").into());
    tr.set_tooltip_nag_mistake(i18n::translate("tooltip.nag_mistake").into());
    tr.set_tooltip_nag_blunder(i18n::translate("tooltip.nag_blunder").into());

    // ── Variation-editing-mode banner (PHASE 26) ─────────────────────────────
    tr.set_variation_banner_viewing(i18n::translate("variation.banner_viewing").into());
    tr.set_variation_banner_editing(i18n::translate("variation.banner_editing").into());
    tr.set_btn_variation_create(i18n::translate("btn.variation_create").into());
    tr.set_btn_variation_exit(i18n::translate("btn.variation_exit").into());

    // ── Preferences → "Misc" tab: debug mode (PHASE 26sexies) ────────────────
    tr.set_prefs_misc_title(i18n::translate("prefs.misc_title").into());
    tr.set_debug_mode_label(i18n::translate("prefs.debug_mode_label").into());
    tr.set_debug_mode_explanation(i18n::translate("prefs.debug_mode_explanation").into());

    // ── "New Game" wizard → step 0 (mode choice) ─────────────────────────────
    tr.set_wizard_mode_human_vs_human(i18n::translate("wizard.mode_human_vs_human").into());
    tr.set_wizard_mode_human_vs_engine(i18n::translate("wizard.mode_human_vs_engine").into());
    tr.set_wizard_mode_engine_vs_engine(i18n::translate("wizard.mode_engine_vs_engine").into());
    tr.set_wizard_mode_tournament(i18n::translate("wizard.mode_tournament").into());
    tr.set_wizard_mode_puzzle(i18n::translate("wizard.mode_puzzle").into());
    tr.set_wizard_mode_reference_base(i18n::translate("wizard.mode_reference_base").into());
    tr.set_wizard_hint_need_2_engines(i18n::translate("wizard.hint_need_2_engines").into());
    tr.set_wizard_hint_need_puzzles(i18n::translate("wizard.hint_need_puzzles").into());
    tr.set_wizard_hint_need_engine(i18n::translate("wizard.hint_need_engine").into());
    tr.set_wizard_hint_need_base(i18n::translate("wizard.hint_need_base").into());
    tr.set_wizard_hint_add_engine_here(i18n::translate("wizard.hint_add_engine_here").into());

    // ── Reference games database browsing screen (PHASE 82, step 7) ─────────
    tr.set_refbrowser_filter_player(i18n::translate("refbrowser.filter_player").into());
    tr.set_refbrowser_filter_elo_min(i18n::translate("refbrowser.filter_elo_min").into());
    tr.set_refbrowser_filter_elo_max(i18n::translate("refbrowser.filter_elo_max").into());
    tr.set_refbrowser_filter_date_from(i18n::translate("refbrowser.filter_date_from").into());
    tr.set_refbrowser_filter_date_to(i18n::translate("refbrowser.filter_date_to").into());
    tr.set_refbrowser_filter_eco(i18n::translate("refbrowser.filter_eco").into());
    tr.set_refbrowser_search_button(i18n::translate("refbrowser.search_button").into());
    tr.set_refbrowser_reset_button(i18n::translate("refbrowser.reset_button").into());
    tr.set_refbrowser_col_white(i18n::translate("refbrowser.col_white").into());
    tr.set_refbrowser_col_black(i18n::translate("refbrowser.col_black").into());
    tr.set_refbrowser_col_result(i18n::translate("refbrowser.col_result").into());
    tr.set_refbrowser_col_date(i18n::translate("refbrowser.col_date").into());
    tr.set_refbrowser_col_elo(i18n::translate("refbrowser.col_elo").into());
    tr.set_refbrowser_col_opening(i18n::translate("refbrowser.col_opening").into());
    tr.set_refbrowser_no_results(i18n::translate("refbrowser.no_results").into());
    tr.set_refbrowser_page_label(i18n::translate("refbrowser.page_label").into());
    tr.set_refbrowser_prev_page(i18n::translate("refbrowser.prev_page").into());
    tr.set_refbrowser_next_page(i18n::translate("refbrowser.next_page").into());
    tr.set_refbrowser_results_suffix(i18n::translate("refbrowser.results_suffix").into());
    tr.set_refbrowser_tab_games(i18n::translate("refbrowser.tab_games").into());
    tr.set_refbrowser_tab_tree(i18n::translate("refbrowser.tab_tree").into());
    tr.set_refbrowser_tree_start_position(i18n::translate("refbrowser.tree_start_position").into());
    tr.set_refbrowser_tree_back(i18n::translate("refbrowser.tree_back").into());
    tr.set_refbrowser_tree_restart(i18n::translate("refbrowser.tree_restart").into());
    tr.set_refbrowser_tree_apply(i18n::translate("refbrowser.tree_apply").into());
    tr.set_refbrowser_tree_col_move(i18n::translate("refbrowser.tree_col_move").into());
    tr.set_refbrowser_tree_col_white_pct(i18n::translate("refbrowser.tree_col_white_pct").into());
    tr.set_refbrowser_tree_col_draw_pct(i18n::translate("refbrowser.tree_col_draw_pct").into());
    tr.set_refbrowser_tree_col_black_pct(i18n::translate("refbrowser.tree_col_black_pct").into());
    tr.set_refbrowser_tree_games_suffix(i18n::translate("refbrowser.tree_games_suffix").into());
    tr.set_refbrowser_tree_no_moves(i18n::translate("refbrowser.tree_no_moves").into());
    tr.set_refbrowser_tree_subtitle(i18n::translate("refbrowser.tree_subtitle").into());
    tr.set_refbrowser_tree_check_all(i18n::translate("refbrowser.tree_check_all").into());
    tr.set_refbrowser_tree_uncheck_all(i18n::translate("refbrowser.tree_uncheck_all").into());
    tr.set_refbrowser_tree_list_games(i18n::translate("refbrowser.tree_list_games").into());
    tr.set_refbrowser_tree_filter_chip_prefix(i18n::translate("refbrowser.tree_filter_chip_prefix").into());

    // ── Game detail: on-demand evaluation curve (PHASE 82,
    // step 9) ───────────────────────────────────────────────────────────────
    tr.set_gamedetail_title(i18n::translate("gamedetail.title").into());
    tr.set_gamedetail_moves_label(i18n::translate("gamedetail.moves_label").into());
    tr.set_gamedetail_eval_label(i18n::translate("gamedetail.eval_label").into());
    tr.set_gamedetail_analyze_button(i18n::translate("gamedetail.analyze_button").into());
    tr.set_gamedetail_deepen_button(i18n::translate("gamedetail.deepen_button").into());
    tr.set_gamedetail_not_analyzed(i18n::translate("gamedetail.not_analyzed").into());
    tr.set_gamedetail_start_from_here(i18n::translate("gamedetail.start_from_here").into());
    tr.set_gamedetail_score_label(i18n::translate("gamedetail.score_label").into());
    tr.set_gamedetail_quality_excellent(i18n::translate("gamedetail.quality_excellent").into());
    tr.set_gamedetail_quality_good(i18n::translate("gamedetail.quality_good").into());
    tr.set_gamedetail_quality_inaccuracy(i18n::translate("gamedetail.quality_inaccuracy").into());
    tr.set_gamedetail_quality_mistake(i18n::translate("gamedetail.quality_mistake").into());
    tr.set_gamedetail_quality_blunder(i18n::translate("gamedetail.quality_blunder").into());
    tr.set_gamedetail_depth_label(i18n::translate("gamedetail.depth_label").into());
    tr.set_gamedetail_pass_quick(i18n::translate("gamedetail.pass_quick").into());
    tr.set_gamedetail_pass_deep(i18n::translate("gamedetail.pass_deep").into());
    tr.set_gamedetail_best_move_label(i18n::translate("gamedetail.best_move_label").into());

    // ── "About" window (PHASE 30, completed here) ────────────────────────────
    tr.set_about_section_author(i18n::translate("about.section_author").into());
    tr.set_about_section_license(i18n::translate("about.section_license").into());
    tr.set_about_section_technologies(i18n::translate("about.section_technologies").into());
    tr.set_about_lang_rust(i18n::translate("about.lang_rust").into());
    tr.set_about_ui_slint(i18n::translate("about.ui_slint").into());
    tr.set_about_made_with(i18n::translate("about.made_with").into());

    // ── "New Game" wizard → step 1 (config) + summary screen ────────────────
    tr.set_wizard_step1_time_title(i18n::translate("wizard.step1_time_title").into());
    tr.set_wizard_tc_cat_unlimited(i18n::translate("wizard.tc_cat_unlimited").into());
    tr.set_wizard_tc_cat_bullet(i18n::translate("wizard.tc_cat_bullet").into());
    tr.set_wizard_tc_cat_blitz(i18n::translate("wizard.tc_cat_blitz").into());
    tr.set_wizard_tc_cat_rapid(i18n::translate("wizard.tc_cat_rapid").into());
    tr.set_wizard_tc_cat_classical(i18n::translate("wizard.tc_cat_classical").into());
    tr.set_wizard_step1_cadence_title(i18n::translate("wizard.step1_cadence_title").into());
    tr.set_wizard_tc_desc_unlimited(i18n::translate("wizard.tc_desc_unlimited").into());
    tr.set_wizard_tc_desc_bullet1(i18n::translate("wizard.tc_desc_bullet1").into());
    tr.set_wizard_tc_desc_bullet2(i18n::translate("wizard.tc_desc_bullet2").into());
    tr.set_wizard_tc_desc_blitz1(i18n::translate("wizard.tc_desc_blitz1").into());
    tr.set_wizard_tc_desc_blitz2(i18n::translate("wizard.tc_desc_blitz2").into());
    tr.set_wizard_tc_desc_rapid1(i18n::translate("wizard.tc_desc_rapid1").into());
    tr.set_wizard_tc_desc_rapid2(i18n::translate("wizard.tc_desc_rapid2").into());
    tr.set_wizard_tc_desc_classical(i18n::translate("wizard.tc_desc_classical").into());
    tr.set_wizard_handicap_toggle(i18n::translate("wizard.handicap_toggle").into());
    tr.set_wizard_your_color_title(i18n::translate("wizard.your_color_title").into());
    tr.set_wizard_color_random(i18n::translate("wizard.color_random").into());
    tr.set_wizard_engine_opponent(i18n::translate("wizard.engine_opponent").into());
    tr.set_wizard_engine_white(i18n::translate("wizard.engine_white").into());
    tr.set_wizard_engine_black(i18n::translate("wizard.engine_black").into());
    tr.set_wizard_puzzle_goal_title(i18n::translate("wizard.puzzle_goal_title").into());
    tr.set_wizard_puzzle_goal_explanation(i18n::translate("wizard.puzzle_goal_explanation").into());
    tr.set_wizard_puzzle_no_hint(i18n::translate("wizard.puzzle_no_hint").into());
    tr.set_wizard_puzzle_with_theme(i18n::translate("wizard.puzzle_with_theme").into());
    tr.set_wizard_puzzle_hint_button_title(i18n::translate("wizard.puzzle_hint_button_title").into());
    tr.set_wizard_puzzle_hint_button_explanation(i18n::translate("wizard.puzzle_hint_button_explanation").into());
    tr.set_wizard_state_active(i18n::translate("wizard.state_active").into());
    tr.set_wizard_state_inactive(i18n::translate("wizard.state_inactive").into());
    tr.set_wizard_start_position_title(i18n::translate("wizard.start_position_title").into());
    tr.set_wizard_pgn_load_placeholder(i18n::translate("wizard.pgn_load_placeholder").into());
    tr.set_wizard_editor_define_placeholder(i18n::translate("wizard.editor_define_placeholder").into());
    tr.set_wizard_editor_custom_position(i18n::translate("wizard.editor_custom_position").into());
    tr.set_wizard_base_define_placeholder(i18n::translate("wizard.base_define_placeholder").into());
    tr.set_wizard_base_custom_position(i18n::translate("wizard.base_custom_position").into());
    tr.set_wizard_nav_previous(i18n::translate("wizard.nav_previous").into());
    tr.set_wizard_quit_confirm_title(i18n::translate("wizard.quit_confirm_title").into());
    tr.set_wizard_quit_confirm_body(i18n::translate("wizard.quit_confirm_body").into());
    tr.set_wizard_btn_no(i18n::translate("wizard.btn_no").into());
    tr.set_wizard_btn_yes_quit(i18n::translate("wizard.btn_yes_quit").into());
    tr.set_wizard_btn_next(i18n::translate("wizard.btn_next").into());
    tr.set_wizard_no_engine_configured(i18n::translate("wizard.no_engine_configured").into());
    tr.set_wizard_recap_title(i18n::translate("wizard.recap_title").into());
    tr.set_wizard_recap_back(i18n::translate("wizard.recap_back").into());
    tr.set_wizard_recap_new_config(i18n::translate("wizard.recap_new_config").into());
    tr.set_wizard_btn_replay(i18n::translate("wizard.btn_replay").into());
    tr.set_wizard_recap_mode_prefix(i18n::translate("wizard.recap_mode_prefix").into());
    tr.set_wizard_recap_no_engine(i18n::translate("wizard.recap_no_engine").into());
    tr.set_wizard_recap_two_engines(i18n::translate("wizard.recap_two_engines").into());
    tr.set_wizard_recap_human_prefix(i18n::translate("wizard.recap_human_prefix").into());

    // ── Preferences (full panel) ─────────────────────────────────────────────
    tr.set_prefs_tab_language(i18n::translate("prefs.tab_language").into());
    tr.set_prefs_tab_engines(i18n::translate("prefs.tab_engines").into());
    tr.set_prefs_tab_openings(i18n::translate("prefs.tab_openings").into());
    tr.set_prefs_tab_puzzles(i18n::translate("prefs.tab_puzzles").into());
    tr.set_prefs_tab_reference_base(i18n::translate("prefs.tab_reference_base").into());
    tr.set_prefs_tab_appearance(i18n::translate("prefs.tab_appearance").into());
    tr.set_prefs_tab_reset(i18n::translate("prefs.tab_reset").into());
    tr.set_prefs_tab_misc(i18n::translate("prefs.tab_misc").into());
    tr.set_prefs_engines_title(i18n::translate("prefs.engines_title").into());
    tr.set_prefs_btn_add_engine(i18n::translate("prefs.btn_add_engine").into());
    tr.set_prefs_engines_empty(i18n::translate("prefs.engines_empty").into());
    tr.set_prefs_engine_click_hint(i18n::translate("prefs.engine_click_hint").into());
    tr.set_prefs_uci_params_title(i18n::translate("prefs.uci_params_title").into());
    tr.set_prefs_engine_querying(i18n::translate("prefs.engine_querying").into());
    tr.set_prefs_options_count_suffix(i18n::translate("prefs.options_count_suffix").into());
    tr.set_prefs_hide_advanced_options(i18n::translate("prefs.hide_advanced_options").into());
    tr.set_prefs_show_advanced_options(i18n::translate("prefs.show_advanced_options").into());
    tr.set_prefs_engines_auto_removed_note(i18n::translate("prefs.engines_auto_removed_note").into());
    tr.set_prefs_hint_engine_title(i18n::translate("prefs.hint_engine_title").into());
    tr.set_prefs_hint_engine_explanation(i18n::translate("prefs.hint_engine_explanation").into());
    tr.set_prefs_none_label(i18n::translate("prefs.none_label").into());
    tr.set_prefs_uci_button_at_startup(i18n::translate("prefs.uci_button_at_startup").into());
    tr.set_prefs_btn_reset_options(i18n::translate("prefs.btn_reset_options").into());
    tr.set_prefs_invalid_options_reset_msg(i18n::translate("prefs.invalid_options_reset_msg").into());
    tr.set_prefs_openings_title(i18n::translate("prefs.openings_title").into());
    tr.set_prefs_openings_explanation(i18n::translate("prefs.openings_explanation").into());
    tr.set_prefs_btn_load(i18n::translate("prefs.btn_load").into());
    tr.set_prefs_puzzles_title(i18n::translate("prefs.puzzles_title").into());
    tr.set_prefs_puzzles_explanation(i18n::translate("prefs.puzzles_explanation").into());
    tr.set_prefs_puzzles_count_suffix(i18n::translate("prefs.puzzles_count_suffix").into());
    tr.set_prefs_puzzles_none_imported(i18n::translate("prefs.puzzles_none_imported").into());
    tr.set_prefs_reference_base_title(i18n::translate("prefs.reference_base_title").into());
    tr.set_prefs_reference_base_explanation(i18n::translate("prefs.reference_base_explanation").into());
    tr.set_prefs_reference_base_count_suffix(i18n::translate("prefs.reference_base_count_suffix").into());
    tr.set_prefs_reference_base_none_imported(i18n::translate("prefs.reference_base_none_imported").into());
    tr.set_prefs_btn_unload(i18n::translate("prefs.btn_unload").into());
    tr.set_prefs_btn_import(i18n::translate("prefs.btn_import").into());
    tr.set_prefs_btn_import_pgn(i18n::translate("prefs.btn_import_pgn").into());
    tr.set_prefs_btn_import_si4(i18n::translate("prefs.btn_import_si4").into());
    tr.set_prefs_reset_title(i18n::translate("prefs.reset_title").into());
    tr.set_prefs_reset_explanation(i18n::translate("prefs.reset_explanation").into());
    tr.set_prefs_reset_button(i18n::translate("prefs.reset_button").into());
    tr.set_prefs_reset_confirm_title(i18n::translate("prefs.reset_confirm_title").into());
    tr.set_prefs_reset_confirm_body(i18n::translate("prefs.reset_confirm_body").into());
    tr.set_prefs_btn_yes_reset(i18n::translate("prefs.btn_yes_reset").into());
    tr.set_prefs_appearance_title(i18n::translate("prefs.appearance_title").into());
    tr.set_prefs_theme_label(i18n::translate("prefs.theme_label").into());
    tr.set_prefs_theme_light(i18n::translate("prefs.theme_light").into());
    tr.set_prefs_theme_dark(i18n::translate("prefs.theme_dark").into());
    tr.set_prefs_appearance_more_soon(i18n::translate("prefs.appearance_more_soon").into());

    // ── Engine Tournament (wizard + tracking panel) ──────────────────────────
    tr.set_tournament_title(i18n::translate("tournament.title").into());
    tr.set_tournament_step1_title(i18n::translate("tournament.step1_title").into());
    tr.set_tournament_step2_title(i18n::translate("tournament.step2_title").into());
    tr.set_tournament_step3_title(i18n::translate("tournament.step3_title").into());
    tr.set_tournament_format_title(i18n::translate("tournament.format_title").into());
    tr.set_tournament_format_roundrobin_label(i18n::translate("tournament.format_roundrobin_label").into());
    tr.set_tournament_format_roundrobin_desc(i18n::translate("tournament.format_roundrobin_desc").into());
    tr.set_tournament_format_gauntlet_label(i18n::translate("tournament.format_gauntlet_label").into());
    tr.set_tournament_format_gauntlet_desc(i18n::translate("tournament.format_gauntlet_desc").into());
    tr.set_tournament_games_per_pair_title(i18n::translate("tournament.games_per_pair_title").into());
    tr.set_tournament_gpp_one(i18n::translate("tournament.gpp_one").into());
    tr.set_tournament_gpp_two(i18n::translate("tournament.gpp_two").into());
    tr.set_tournament_engines_title(i18n::translate("tournament.engines_title").into());
    tr.set_tournament_engines_selected_suffix(i18n::translate("tournament.engines_selected_suffix").into());
    tr.set_tournament_engines_select_hint(i18n::translate("tournament.engines_select_hint").into());
    tr.set_tournament_need_2_engines(i18n::translate("tournament.need_2_engines").into());
    tr.set_tournament_challenger_label(i18n::translate("tournament.challenger_label").into());
    tr.set_tournament_gauntlet_note(i18n::translate("tournament.gauntlet_note").into());
    tr.set_tournament_quick_test_title(i18n::translate("tournament.quick_test_title").into());
    tr.set_tournament_timed_cadences_title(i18n::translate("tournament.timed_cadences_title").into());
    tr.set_tournament_recap_title(i18n::translate("tournament.recap_title").into());
    tr.set_tournament_recap_engines_suffix(i18n::translate("tournament.recap_engines_suffix").into());
    tr.set_tournament_recap_games_per_pair_suffix(i18n::translate("tournament.recap_games_per_pair_suffix").into());
    tr.set_tournament_finished_title(i18n::translate("tournament.finished_title").into());
    tr.set_tournament_in_progress_title(i18n::translate("tournament.in_progress_title").into());
    tr.set_tournament_games_played_suffix(i18n::translate("tournament.games_played_suffix").into());
    tr.set_tournament_col_engine(i18n::translate("tournament.col_engine").into());
    tr.set_tournament_col_points(i18n::translate("tournament.col_points").into());
    tr.set_tournament_col_wins(i18n::translate("tournament.col_wins").into());
    tr.set_tournament_col_draws(i18n::translate("tournament.col_draws").into());
    tr.set_tournament_col_losses(i18n::translate("tournament.col_losses").into());
    tr.set_tournament_btn_stop(i18n::translate("tournament.btn_stop").into());

    // ── Position editor ───────────────────────────────────────────────────────
    tr.set_posedit_title(i18n::translate("posedit.title").into());
    tr.set_posedit_eraser(i18n::translate("posedit.eraser").into());
    tr.set_posedit_clear_all(i18n::translate("posedit.clear_all").into());
    tr.set_posedit_turn_label(i18n::translate("posedit.turn_label").into());
    tr.set_posedit_castling_rights(i18n::translate("posedit.castling_rights").into());
    tr.set_posedit_reset_position(i18n::translate("posedit.reset_position").into());
    tr.set_posedit_btn_validate(i18n::translate("posedit.btn_validate").into());
    tr.set_posedit_btn_use_position(i18n::translate("posedit.btn_use_position").into());

    // ── Remaining app.slint content (NAG menu, puzzle end, pause, quit confirmation) ─
    tr.set_app_nag_add_comment(i18n::translate("app.nag_add_comment").into());
    tr.set_app_nag_promote_variation(i18n::translate("app.nag_promote_variation").into());
    tr.set_app_nag_delete_variation(i18n::translate("app.nag_delete_variation").into());
    tr.set_app_puzzle_show_solution(i18n::translate("app.puzzle_show_solution").into());
    tr.set_app_puzzle_next(i18n::translate("app.puzzle_next").into());
    tr.set_app_pause_pause(i18n::translate("app.pause_pause").into());
    tr.set_app_pause_resume(i18n::translate("app.pause_resume").into());
    tr.set_app_pause_banner_title(i18n::translate("app.pause_banner_title").into());
    tr.set_app_quit_confirm_title(i18n::translate("app.quit_confirm_title").into());
    tr.set_app_quit_confirm_body(i18n::translate("app.quit_confirm_body").into());
    tr.set_app_quit_confirm_import_warning(i18n::translate("app.quit_confirm_import_warning").into());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use i18n::{Lang, translate_in};

    // -----------------------------------------------------------------------
    // Completeness of ALL_KEYS
    // -----------------------------------------------------------------------

    #[test]
    fn test_all_keys_count() {
        // 11/07/2026 (audit robustesse, finding 1.2) : +2 —
        // `dialog.unexpected_error_title`/`_desc`. Note au passage : un
        // recomptage statique précis du tableau donnait déjà 391 avant cet
        // ajout (et non 390 comme l'assertion précédente l'indiquait) —
        // écart d'1 clé antérieur à cette session, non expliqué, corrigé ici
        // en même temps plutôt que reporté (393 = 391 + 2).
        assert_eq!(ALL_KEYS.len(), 393, "ALL_KEYS doit contenir exactement 393 clés");
    }

    #[test]
    fn test_all_keys_unique() {
        let mut sorted = ALL_KEYS.to_vec();
        sorted.sort_unstable();
        let orig_len = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), orig_len, "ALL_KEYS contient des clés en double");
    }

    // -----------------------------------------------------------------------
    // Presence in every language
    // -----------------------------------------------------------------------

    /// Checks that every key of `ALL_KEYS` is translated (non-empty, != key)
    /// in every language. Covers 40 × 372 = 14880 combinations.
    #[test]
    fn test_all_keys_translated_in_all_langs() {
        for &lang in Lang::all() {
            for &key in ALL_KEYS {
                let value = translate_in(lang, key);
                assert!(
                    !value.is_empty(),
                    "clé '{key}' vide pour la langue {lang:?}"
                );
                assert_ne!(
                    value, key,
                    "clé '{key}' non traduite (fallback) pour la langue {lang:?}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Per-language spot checks
    // -----------------------------------------------------------------------

    #[test]
    fn test_fr_sample_values() {
        assert_eq!(translate_in(Lang::Fr, "app.name"),            "Vendetta Chess");
        assert_eq!(translate_in(Lang::Fr, "menu.file.new_game"),  "Nouvelle partie");
        assert_eq!(translate_in(Lang::Fr, "board.checkmate"),     "Échec et mat");
        assert_eq!(translate_in(Lang::Fr, "btn.cancel"),          "Annuler");
    }

    #[test]
    fn test_en_sample_values() {
        assert_eq!(translate_in(Lang::En, "menu.file.new_game"),  "New Game");
        assert_eq!(translate_in(Lang::En, "board.white"),         "White");
        assert_eq!(translate_in(Lang::En, "analysis.depth"),      "Depth");
        assert_eq!(translate_in(Lang::En, "btn.ok"),              "OK");
    }

    #[test]
    fn test_de_sample_values() {
        assert_eq!(translate_in(Lang::De, "board.checkmate"),     "Schachmatt");
        assert_eq!(translate_in(Lang::De, "menu.file.quit"),      "Beenden");
        assert_eq!(translate_in(Lang::De, "btn.cancel"),          "Abbrechen");
    }

    #[test]
    fn test_pl_sample_values() {
        assert_eq!(translate_in(Lang::Pl, "analysis.depth"),      "Głębokość");
        assert_eq!(translate_in(Lang::Pl, "board.checkmate"),     "Szach mat");
    }

    #[test]
    fn test_is_sample_values() {
        assert_eq!(translate_in(Lang::Is, "btn.ok"),              "Í lagi");
        assert_eq!(translate_in(Lang::Is, "board.check"),         "Skák!");
    }

    #[test]
    fn test_no_fi_sv_spot_check() {
        assert_eq!(translate_in(Lang::No, "board.check"),         "Sjakk!");
        assert_eq!(translate_in(Lang::Fi, "btn.cancel"),          "Peruuta");
        assert_eq!(translate_in(Lang::Sv, "board.draw"),          "Remi");
    }

    // -----------------------------------------------------------------------
    // A key outside ALL_KEYS must not be duplicated
    // -----------------------------------------------------------------------

    #[test]
    fn test_unknown_key_not_in_all_keys() {
        assert!(
            !ALL_KEYS.contains(&"clé.inconnue"),
            "une clé fictive ne doit pas être dans ALL_KEYS"
        );
    }
}
