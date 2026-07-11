//! Test d'intégration — PHASE 16 : Variantes PGN (annotations et lignes
//! alternatives), Étape 8 (clôture).
//!
//! Contrairement aux tests unitaires de chaque sous-étape (déjà nombreux
//! dans `crates/gui/src/game_controller.rs` et `crates/core/src/{game_tree,
//! history, pgn}.rs`, chacun isolant une seule fonctionnalité), ce test
//! rejoue un scénario complet et réaliste enchaînant **toutes** les
//! fonctionnalités de PHASE 16 dans l'ordre où un utilisateur les
//! rencontrerait, via l'API publique de [`GameController`] uniquement
//! (jamais d'accès direct à `chess_core::game_tree`/`history`) :
//!
//! 1. Jouer une ligne principale de 6 demi-coups (Italienne : 1.e4 e5 2.Nf3
//!    Nc6 3.Bc4 Bc5).
//! 2. Revenir dans l'historique et jouer un coup différent (3.Bb5 à la place
//!    de 3.Bc4) — crée une variante (Étape 5) qui démote *toute la suite
//!    déjà enregistrée* (Bc4 **et** Bc5, pas seulement Bc4) en un seul bloc.
//! 3. Annoter la variante démotée d'un NAG (Étape 6.1) et d'un commentaire
//!    (Étape 6.3) — le commentaire n'est jamais affiché tant que le nœud
//!    reste une variante (restriction actée Étape 6.3, limitée à la ligne
//!    principale).
//! 4. Promouvoir la variante en ligne principale (Étape 6.2) : la suite
//!    complète (Bc4 *et* Bc5) doit être restaurée, le NAG et le commentaire
//!    doivent rester attachés au nœud (ils ne sont jamais perdus par une
//!    opération purement structurelle), et le commentaire devient visible
//!    maintenant que son nœud est sur la ligne principale.
//! 5. Supprimer la variante devenue résiduelle (l'ancien Bb5, Étape 6.2).
//! 6. Exporter la partie en PGN (Étape 7.1) et vérifier que le NAG (`$6`) et
//!    le commentaire (`{...}`) apparaissent avec la renumérotation correcte,
//!    et que la variante supprimée n'y figure plus.
//! 7. Réimporter ce PGN (Étape 7.2) dans un second `GameController` et
//!    vérifier que la position finale, le NAG et le commentaire sont
//!    identiques à l'originale — preuve de bout en bout que le cycle complet
//!    partie → annotations → variantes → PGN → partie ne perd aucune
//!    information.

use gui::game_controller::GameController;

// Clippy (04/07/2026) : `bc4_var_id`/`bb5_id` sont des identifiants de nœud
// `i32` explicitement vérifiés `>= 0` par les `assert!` juste au-dessus de
// chaque cast vers `usize` — jamais négatifs en pratique.
#[allow(clippy::cast_sign_loss)]
#[test]
fn test_phase16_full_variation_annotation_promote_remove_export_import_roundtrip() {
    let mut ctrl = GameController::new();

    // ── 1. Ligne principale : 1.e4 e5 2.Nf3 Nc6 3.Bc4 Bc5 ──────────────────
    assert!(ctrl.on_click(6, 4) && ctrl.on_click(4, 4)); // 1.e4
    assert!(ctrl.on_click(1, 4) && ctrl.on_click(3, 4)); // 1...e5
    assert!(ctrl.on_click(7, 6) && ctrl.on_click(5, 5)); // 2.Nf3
    assert!(ctrl.on_click(0, 1) && ctrl.on_click(2, 2)); // 2...Nc6
    assert!(ctrl.on_click(7, 5) && ctrl.on_click(4, 2)); // 3.Bc4
    assert!(ctrl.on_click(0, 5) && ctrl.on_click(3, 2)); // 3...Bc5

    assert_eq!(ctrl.move_count(), 6);
    let rows = ctrl.build_move_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].white_san.as_str(), "Bc4");
    assert_eq!(rows[2].black_san.as_str(), "Bc5");
    assert!(rows[2].white_variations.as_str().is_empty());

    // ── 2. Retour après 2...Nc6, jouer 3.Bb5 à la place de 3.Bc4 ───────────
    // (Étape 5 : la suite déjà enregistrée, Bc4 ET Bc5, n'est jamais perdue
    // — elle devient une variante d'un seul bloc, voir doc de
    // `History::branch_at`.)
    assert!(ctrl.go_to_ply(3)); // visualise la position après 2...Nc6
    ctrl.set_variation_mode_enabled(true);
    assert!(ctrl.on_click(7, 5)); // sélectionne le fou f1
    assert!(ctrl.on_click(3, 1), "3.Bb5 doit créer une variante"); // f1-b5
    ctrl.set_variation_mode_enabled(false);

    assert_eq!(ctrl.viewed_ply_slint(), -1, "retour automatique à la position courante");
    assert_eq!(ctrl.move_count(), 5, "Bb5 remplace Bc4 : la ligne active est raccourcie à 5 coups");

    let rows = ctrl.build_move_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].white_san.as_str(), "Bb5");
    assert_eq!(rows[2].black_san.as_str(), "");
    assert!(
        rows[2].white_variations.as_str().contains("Bc4") && rows[2].white_variations.as_str().contains("Bc5"),
        "Bc4 ET Bc5 doivent apparaître ensemble dans le même bloc de variante — texte obtenu : {}",
        rows[2].white_variations.as_str()
    );

    let bc4_var_id = rows[2].white_variation_node_id;
    assert!(bc4_var_id >= 0, "la variante doit exposer un identifiant de nœud cible");

    // ── 3. Annoter la variante démotée (NAG + commentaire) ─────────────────
    assert!(ctrl.toggle_move_nag(bc4_var_id as usize, 6)); // "?!" (Nag::Dubious)
    let rows = ctrl.build_move_rows();
    assert!(
        rows[2].white_variations.as_str().contains("?!"),
        "le glyphe NAG doit apparaître dans le texte de la variante — texte obtenu : {}",
        rows[2].white_variations.as_str()
    );

    assert!(ctrl.set_move_comment(bc4_var_id as usize, "Retour à l'Italienne"));
    // Aucun champ de `MoveRow` n'expose jamais le commentaire d'un nœud de
    // variante (restriction Étape 6.3, `comment_for` n'est appliqué qu'aux
    // nœuds `white_node_id`/`black_node_id`) : le commentaire est bien
    // enregistré dans l'arbre (vérifié après promotion, étape suivante),
    // mais rien à observer côté `MoveRow` tant que le nœud reste une
    // variante.

    // ── 4. Promouvoir la variante : Bc4 (et sa suite Bc5) redevient la ligne
    //       active, NAG et commentaire doivent avoir survécu ─────────────────
    assert!(ctrl.promote_variation_to_mainline(bc4_var_id as usize));
    assert_eq!(ctrl.move_count(), 6, "la suite complète (Bc4 Bc5) doit être restaurée intégralement");

    let rows = ctrl.build_move_rows();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2].white_san.as_str(), "Bc4");
    assert_eq!(rows[2].black_san.as_str(), "Bc5");
    assert_eq!(rows[2].white_nag.as_str(), "?!", "le NAG posé avant la promotion doit survivre");
    assert_eq!(
        rows[2].white_comment.as_str(), "Retour à l'Italienne",
        "le commentaire doit devenir visible maintenant que le nœud est sur la ligne principale"
    );
    assert!(
        rows[2].white_variations.as_str().contains("Bb5"),
        "l'ancienne ligne active (Bb5) doit à son tour devenir une variante — texte obtenu : {}",
        rows[2].white_variations.as_str()
    );

    let bb5_id = rows[2].white_variation_node_id;
    assert!(bb5_id >= 0);

    // ── 5. Supprimer la variante résiduelle (Bb5) ──────────────────────────
    assert!(ctrl.remove_variation(bb5_id as usize));
    let rows = ctrl.build_move_rows();
    assert!(rows[2].white_variations.as_str().is_empty(), "la variante supprimée ne doit plus apparaître");
    assert_eq!(ctrl.move_count(), 6, "supprimer une variante n'affecte jamais la ligne active");

    // ── 6. Export PGN (Étape 7.1) ───────────────────────────────────────────
    let pgn = ctrl.export_pgn("Alice", "Bob");
    assert!(pgn.contains("[White \"Alice\"]"));
    assert!(pgn.contains("[Black \"Bob\"]"));
    assert!(!pgn.contains("Bb5"), "la variante supprimée ne doit pas réapparaître dans l'export — PGN produit : {pgn}");
    assert!(
        pgn.contains("Bc4 $6 {Retour à l'Italienne}"),
        "NAG et commentaire doivent être exportés ensemble juste après le coup — PGN produit : {pgn}"
    );
    assert!(
        pgn.contains("3... Bc5"),
        "le commentaire doit forcer la renumérotation du coup noir suivant — PGN produit : {pgn}"
    );

    // ── 7. Réimport (Étape 7.2) : la partie reconstruite doit être identique
    //       à l'originale (position, NAG, commentaire), sans aucune variante
    //       résiduelle ────────────────────────────────────────────────────────
    let mut ctrl2 = GameController::new();
    ctrl2
        .load_from_pgn(&pgn)
        .expect("le PGN produit par l'export doit être ré-importable sans erreur");

    assert_eq!(ctrl2.move_count(), ctrl.move_count());
    assert_eq!(ctrl2.current_fen(), ctrl.current_fen(), "la position finale doit être identique après réimport");

    let rows2 = ctrl2.build_move_rows();
    assert_eq!(rows2.len(), rows.len());
    assert_eq!(rows2[2].white_san.as_str(), "Bc4");
    assert_eq!(rows2[2].black_san.as_str(), "Bc5");
    assert_eq!(rows2[2].white_nag.as_str(), "?!", "le NAG doit survivre à l'aller-retour export → import");
    assert_eq!(
        rows2[2].white_comment.as_str(), "Retour à l'Italienne",
        "le commentaire doit survivre à l'aller-retour export → import"
    );
    assert!(
        rows2[2].white_variations.as_str().is_empty(),
        "aucune variante ne doit réapparaître après réimport — texte obtenu : {}",
        rows2[2].white_variations.as_str()
    );
}
