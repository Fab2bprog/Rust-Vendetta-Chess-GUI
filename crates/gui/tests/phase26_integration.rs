//! Test d'intégration — PHASE 26 : mode d'édition de variante explicite,
//! Étape 5 (clôture).
//!
//! Rejoue, via l'API publique de [`GameController`] uniquement, le scénario
//! complet qui a motivé cette phase : l'utilisateur signalait que (1) le
//! panneau "Partie en pause" se rallumait dès le premier coup joué dans une
//! variante (bloquant toute suite) et que (2) le mécanisme n'était même pas
//! accessible en cadence `Infinite`. La décision retenue (04/07/2026) a été
//! de remplacer la dérivation implicite `hvh_paused || is_over` par un état
//! explicite et autonome, `variation_editing`, piloté par un bandeau dédié
//! (`enter_variation_editing()` / `exit_variation_editing()`) et totalement
//! indépendant de la pause et de la cadence de jeu.
//!
//! Ce test verrouille le comportement de bout en bout, au-delà des tests
//! unitaires ciblés déjà présents dans `game_controller.rs` (chacun isolant
//! une seule facette de PHASE 26) :
//!
//! 1. Jouer une ligne principale de 4 demi-coups.
//! 2. Revenir dans l'historique (`go_to_ply`) et entrer en édition de
//!    variante (`enter_variation_editing`).
//! 3. Enchaîner **trois** coups de variante à la suite — reproduit fidèlement
//!    le bug d'origine (le premier coup joué remet `viewed_ply` à `None`,
//!    mais ne doit *jamais* faire sortir du mode d'édition tant que
//!    `exit_variation_editing()` n'a pas été appelé explicitement).
//! 4. Sortir explicitement du mode et vérifier un retour propre à la
//!    position courante.
//! 5. Vérifier qu'un `reset()` en cours d'édition force bien la sortie du
//!    mode (protection déjà unitaire, revérifiée ici dans un scénario
//!    complet et réaliste).

use gui::game_controller::GameController;

#[test]
fn test_phase26_variation_editing_persists_across_several_moves_then_exits_cleanly() {
    let mut ctrl = GameController::new();

    // ── 1. Ligne principale : 1.e4 e5 2.Nf3 Nc6 ─────────────────────────────
    assert!(ctrl.on_click(6, 4) && ctrl.on_click(4, 4)); // 1.e4
    assert!(ctrl.on_click(1, 4) && ctrl.on_click(3, 4)); // 1...e5
    assert!(ctrl.on_click(7, 6) && ctrl.on_click(5, 5)); // 2.Nf3
    assert!(ctrl.on_click(0, 1) && ctrl.on_click(2, 2)); // 2...Nc6
    assert_eq!(ctrl.move_count(), 4);

    // ── 2. Retour après 1.e4, entrée en édition de variante ────────────────
    assert!(ctrl.go_to_ply(0), "doit pouvoir visualiser la position après 1.e4");
    assert!(
        !ctrl.is_variation_editing(),
        "consulter l'historique seul ne doit jamais activer l'édition de variante"
    );
    assert!(
        ctrl.enter_variation_editing(),
        "l'entrée doit réussir : un coup passé est bien visualisé (viewed_ply is_some)"
    );
    assert!(ctrl.is_variation_editing());

    // ── 3. Trois coups enchaînés dans la variante ───────────────────────────
    // 1...c5 à la place de 1...e5 : crée la variante.
    assert!(ctrl.on_click(1, 2) && ctrl.on_click(3, 2));
    assert_eq!(
        ctrl.viewed_ply_slint(), -1,
        "viewed_ply repasse bien à -1 dès ce premier coup (mécanisme d'origine, inchangé)"
    );
    assert!(
        ctrl.is_variation_editing(),
        "BUG D'ORIGINE : le mode d'édition ne doit PAS se désactiver après le premier coup"
    );

    // 2.Nf3
    assert!(ctrl.on_click(7, 6) && ctrl.on_click(5, 5));
    assert!(ctrl.is_variation_editing(), "doit rester actif après un deuxième coup");

    // 2...Nc6
    assert!(ctrl.on_click(0, 1) && ctrl.on_click(2, 2));
    assert!(ctrl.is_variation_editing(), "doit rester actif après un troisième coup");
    assert_eq!(ctrl.move_count(), 4, "la nouvelle ligne (c5 Nf3 Nc6) remplace l'ancienne (e5 Nf3 Nc6)");

    let rows = ctrl.build_move_rows();
    assert_eq!(rows[0].black_san.as_str(), "c5", "la ligne active reflète bien la variante jouée");
    assert!(
        rows[0].black_variations.as_str().contains("e5"),
        "l'ancienne suite (1...e5) doit survivre comme variante, jamais être perdue — texte obtenu : {}",
        rows[0].black_variations.as_str()
    );

    // ── 4. Sortie explicite du mode d'édition ───────────────────────────────
    ctrl.exit_variation_editing();
    assert!(!ctrl.is_variation_editing());
    assert_eq!(ctrl.viewed_ply_slint(), -1, "retour à la position courante");

    // ── 5. reset() en cours d'édition doit forcer la sortie ────────────────
    assert!(ctrl.go_to_ply(0));
    assert!(ctrl.enter_variation_editing());
    assert!(ctrl.is_variation_editing());

    ctrl.reset();
    assert!(
        !ctrl.is_variation_editing(),
        "une nouvelle partie ne doit jamais laisser le mode d'édition actif par inadvertance"
    );
}

#[test]
fn test_phase26_enter_variation_editing_requires_a_viewed_past_ply() {
    // Sans consultation de l'historique, rien à éditer : l'entrée doit être
    // refusée, contrairement à l'ancien mécanisme qui se basait sur l'état de
    // pause (accessible indépendamment de tout `viewed_ply`).
    let mut ctrl = GameController::new();
    assert!(ctrl.on_click(6, 4) && ctrl.on_click(4, 4)); // 1.e4

    assert!(!ctrl.enter_variation_editing(), "aucun coup passé visualisé : l'entrée doit échouer");
    assert!(!ctrl.is_variation_editing());
}
