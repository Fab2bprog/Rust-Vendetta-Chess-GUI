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

# 4. (optionnel) mettre à jour le cache pour que l'entrée apparaisse immédiatement
update-desktop-database ~/.local/share/applications/ 2>/dev/null || true
```

L'application apparaît alors dans le menu des applications sous le nom "Vendetta Chess" et peut
être épinglée à la barre des tâches/dock comme n'importe quel autre logiciel.

## Installation système (tous les utilisateurs, nécessite `sudo`)

```bash
sudo mkdir -p /opt/vendetta-chess
sudo cp target/release/vendetta-chess-gui /opt/vendetta-chess/
sudo cp crates/gui/linux/vendetta-chess.desktop /usr/share/applications/
sudo update-desktop-database /usr/share/applications/
```

Dans ce cas, le chemin `Exec=/opt/vendetta-chess/vendetta-chess-gui` du fichier fourni est déjà
correct, aucune substitution nécessaire.

## Icône

Aucune icône dédiée n'existe encore dans le projet (`Icon=vendetta-chess` référence un nom qui
n'est pour l'instant associé à aucun fichier — une icône générique s'affichera). Pour ajouter une
vraie icône une fois disponible (PNG carré, idéalement 256×256 et/ou SVG) :

```bash
# utilisateur courant
mkdir -p ~/.local/share/icons/hicolor/256x256/apps
cp vendetta-chess.png ~/.local/share/icons/hicolor/256x256/apps/

# système
sudo cp vendetta-chess.png /usr/share/icons/hicolor/256x256/apps/
```
