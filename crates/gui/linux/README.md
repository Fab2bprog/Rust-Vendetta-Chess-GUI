# Intégration Linux (fichier `.desktop`)

Ce dossier contient le fichier `vendetta-chess.desktop` qui permet à Vendetta Chess GUI
d'apparaître proprement dans le menu des applications (GNOME, KDE, XFCE...) et de se lancer en
un clic depuis Nautilus/Dolphin/etc., sans que le gestionnaire de fichiers propose un dialogue
"Exécuter / Afficher" ni qu'aucune fenêtre de terminal n'apparaisse (contrairement à Windows et
macOS, Linux n'a de toute façon jamais ce problème de fenêtre console pour un exécutable GUI —
mais sans fichier `.desktop`, l'app n'a ni icône ni entrée dans le menu).

## Installation (utilisateur courant, sans droits root)

```bash
# 1. Compiler en release
cd vendetta_chess_gui
cargo build --release

# 2. Choisir un emplacement stable pour le binaire (ne pas rester dans target/,
#    qui peut être supprimé par un `cargo clean`)
mkdir -p ~/.local/opt/vendetta-chess
cp target/release/vendetta-chess-gui ~/.local/opt/vendetta-chess/

# 3. Copier le .desktop et corriger le chemin Exec= pour pointer vers ~/.local/opt/...
mkdir -p ~/.local/share/applications
sed "s|/opt/vendetta-chess/vendetta-chess-gui|$HOME/.local/opt/vendetta-chess/vendetta-chess-gui|" \
    crates/gui/linux/vendetta-chess.desktop > ~/.local/share/applications/vendetta-chess.desktop

# 4. Copier l'icône (voir section "Icône" ci-dessous pour le détail)
mkdir -p ~/.local/share/icons/hicolor/256x256/apps
cp crates/gui/linux/vendetta-chess.png ~/.local/share/icons/hicolor/256x256/apps/

# 5. (optionnel) mettre à jour les caches pour que l'entrée/l'icône apparaissent immédiatement
update-desktop-database ~/.local/share/applications/ 2>/dev/null || true
gtk-update-icon-cache ~/.local/share/icons/hicolor 2>/dev/null || true
```

L'application apparaît alors dans le menu des applications sous le nom "Vendetta Chess" et peut
être épinglée à la barre des tâches/dock comme n'importe quel autre logiciel.

## Installation système (tous les utilisateurs, nécessite `sudo`)

```bash
sudo mkdir -p /opt/vendetta-chess
sudo cp target/release/vendetta-chess-gui /opt/vendetta-chess/
sudo cp crates/gui/linux/vendetta-chess.desktop /usr/share/applications/
sudo mkdir -p /usr/share/icons/hicolor/256x256/apps
sudo cp crates/gui/linux/vendetta-chess.png /usr/share/icons/hicolor/256x256/apps/
sudo update-desktop-database /usr/share/applications/
```

Dans ce cas, le chemin `Exec=/opt/vendetta-chess/vendetta-chess-gui` du fichier fourni est déjà
correct, aucune substitution nécessaire.

## Icône

`vendetta-chess.png` (256×256, fourni dans ce dossier) correspond au nom référencé par
`Icon=vendetta-chess` du fichier `.desktop` — sa copie vers le dossier `hicolor` du thème
d'icônes est déjà incluse dans les deux procédures d'installation ci-dessus (étape 4 pour
l'installation utilisateur, ligne dédiée pour l'installation système).
