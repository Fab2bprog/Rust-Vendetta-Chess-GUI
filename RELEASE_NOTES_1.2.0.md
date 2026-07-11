# Vendetta Chess GUI — v1.2.0

## 🇬🇧 English

**Vendetta Chess GUI** is a professional-grade chess interface written entirely in **Rust**, with a user interface built on **[Slint](https://slint.dev)** — a lightweight, native, GPU-accelerated GUI toolkit. It embeds no chess engine of its own: it is a full **UCI client**, designed to drive one or several external engines (Stockfish, and others) for play, analysis, and training. Fully portable, available in 40 languages, and released under the GNU GPLv3.

### What's new in v1.2.0

This release is centered on **SCID database support** — the ability to browse, search, and study massive reference game collections (millions of games) directly inside the app, in addition to the existing PGN reference database.

- **SCID database import** — Import reference game databases in the SCID format (`.si4` and `.si5`), a format widely used for large game collections (e.g. Lumbra's Gigabase). It runs alongside your existing PGN reference database as a second, independent source — import or clear one without touching the other.
- **Reference game browser** — New full-screen browser to search and explore your reference database: filter by player name, Elo range, date range, and ECO opening code, with pagination for large result sets.
- **Interactive opening tree** — Browse candidate moves from any position with live statistics (games played, win rates), drill down move by move, and jump straight to the list of games behind any branch.
- **On-demand game analysis** — Open any reference game to see its full move list, request a quick or deep engine analysis pass, view its evaluation curve, and step through the position with a live preview board — export the position as PGN or PNG, or copy/paste its FEN.
- **Start a new game from the database** — The "New Game" wizard can now use a position picked directly from your reference database as the starting point, alongside the existing position editor and PGN loading options.
- **Move comments now imported** — Comments attached to moves in SCID databases are now decoded and displayed, instead of being silently discarded.
- **NAG annotations preserved** — Move-quality annotations (`!!`, `!`, `!?`, `?!`, `?`, `??`) are now correctly captured on import and re-exported to PGN.
- **Custom starting positions supported** — Games starting from a non-standard position (Chess960, training positions, etc.) are no longer rejected during import.
- **Faster imports** — SCID imports now use batched database transactions, significantly cutting import time on large files.
- **Codebase now fully in English** — All internal source code comments have been translated from French to English, making the project easier to read and contribute to for the open-source community.

### License

Released under the **GNU GPLv3**.

---

## 🇫🇷 Français

**Vendetta Chess GUI** est une interface d'échecs de niveau professionnel écrite entièrement en **Rust**, avec une interface graphique construite sur **[Slint](https://slint.dev)** — un toolkit GUI natif, léger et accéléré par GPU. Elle n'intègre aucun moteur d'échecs : c'est un **client UCI complet**, conçu pour piloter un ou plusieurs moteurs externes (Stockfish, entre autres) pour le jeu, l'analyse et l'entraînement. Totalement portable, disponible en 40 langues, et distribuée sous licence GNU GPLv3.

### Nouveautés de la v1.2.0

Cette version est centrée sur le **support des bases SCID** — la possibilité de parcourir, chercher et étudier de très grandes collections de parties de référence (des millions de parties) directement dans le logiciel, en complément de la base de référence PGN déjà existante.

- **Import de bases SCID** — Importez des bases de parties de référence au format SCID (`.si4` et `.si5`), un format très répandu pour les grandes collections de parties (ex. Lumbra's Gigabase). Elle fonctionne en parallèle de votre base de référence PGN existante, comme une seconde source indépendante — importer ou vider l'une ne touche jamais l'autre.
- **Explorateur de parties de référence** — Nouvel écran plein écran pour chercher et explorer votre base de référence : filtres par joueur, plage d'Elo, plage de dates et code d'ouverture ECO, avec pagination pour les grands ensembles de résultats.
- **Arbre d'ouvertures interactif** — Parcourez les coups candidats depuis n'importe quelle position avec des statistiques en direct (parties jouées, taux de victoire), descendez coup après coup, et accédez directement à la liste des parties derrière n'importe quelle branche.
- **Analyse de partie à la demande** — Ouvrez n'importe quelle partie de référence pour voir la liste complète de ses coups, lancez une analyse moteur rapide ou approfondie, consultez sa courbe d'évaluation, et naviguez dans la position avec un échiquier d'aperçu en direct — exportez la position en PGN ou en PNG, ou copiez/collez son FEN.
- **Démarrer une partie depuis la base** — L'assistant « Nouvelle partie » peut désormais utiliser une position choisie directement dans votre base de référence comme position de départ, en plus de l'éditeur de position et du chargement PGN déjà existants.
- **Commentaires de coups désormais importés** — Les commentaires attachés aux coups dans les bases SCID sont maintenant décodés et affichés, au lieu d'être silencieusement ignorés.
- **Annotations NAG conservées** — Les annotations de qualité de coup (`!!`, `!`, `!?`, `?!`, `?`, `??`) sont désormais correctement capturées à l'import et réexportées en PGN.
- **Positions de départ personnalisées prises en charge** — Les parties commençant depuis une position non standard (Chess960, positions d'entraînement, etc.) ne sont plus rejetées à l'import.
- **Imports plus rapides** — Les imports SCID utilisent désormais des transactions groupées, réduisant nettement le temps d'import sur les gros fichiers.
- **Code source désormais entièrement en anglais** — Tous les commentaires internes du code source ont été traduits du français vers l'anglais, pour faciliter la lecture et la contribution au projet par la communauté open source.

### Licence

Distribué sous licence **GNU GPLv3**.

---

## 🇪🇸 Español

**Vendetta Chess GUI** es una interfaz de ajedrez de nivel profesional escrita íntegramente en **Rust**, con una interfaz gráfica construida sobre **[Slint](https://slint.dev)** — un toolkit de GUI nativo, ligero y acelerado por GPU. No incluye ningún motor de ajedrez propio: es un **cliente UCI completo**, diseñado para dirigir uno o varios motores externos (Stockfish, entre otros) para jugar, analizar y entrenar. Totalmente portátil, disponible en 40 idiomas y distribuido bajo licencia GNU GPLv3.

### Novedades de la v1.2.0

Esta versión se centra en el **soporte de bases de datos SCID** — la posibilidad de explorar, buscar y estudiar grandes colecciones de partidas de referencia (millones de partidas) directamente dentro de la aplicación, junto a la base de referencia PGN ya existente.

- **Importación de bases SCID** — Importa bases de datos de partidas de referencia en formato SCID (`.si4` y `.si5`), un formato muy utilizado para grandes colecciones de partidas (por ejemplo, Lumbra's Gigabase). Funciona en paralelo con tu base de referencia PGN existente, como una segunda fuente independiente — importar o vaciar una no afecta a la otra.
- **Explorador de partidas de referencia** — Nueva pantalla completa para buscar y explorar tu base de referencia: filtros por jugador, rango de Elo, rango de fechas y código de apertura ECO, con paginación para grandes conjuntos de resultados.
- **Árbol de aperturas interactivo** — Explora los movimientos candidatos desde cualquier posición con estadísticas en vivo (partidas jugadas, porcentaje de victorias), profundiza jugada a jugada y accede directamente a la lista de partidas de cualquier rama.
- **Análisis de partida bajo demanda** — Abre cualquier partida de referencia para ver su lista completa de jugadas, solicita un análisis rápido o profundo del motor, consulta su curva de evaluación y navega por la posición con un tablero de vista previa en vivo — exporta la posición como PGN o PNG, o copia/pega su FEN.
- **Iniciar una partida desde la base de datos** — El asistente de "Nueva partida" ahora puede usar una posición elegida directamente desde tu base de referencia como punto de partida, además del editor de posiciones y la carga de PGN ya existentes.
- **Comentarios de jugadas ahora importados** — Los comentarios asociados a las jugadas en las bases SCID ahora se decodifican y se muestran, en lugar de descartarse silenciosamente.
- **Anotaciones NAG conservadas** — Las anotaciones de calidad de jugada (`!!`, `!`, `!?`, `?!`, `?`, `??`) ahora se capturan correctamente al importar y se vuelven a exportar en PGN.
- **Soporte para posiciones iniciales personalizadas** — Las partidas que comienzan desde una posición no estándar (Chess960, posiciones de entrenamiento, etc.) ya no se rechazan durante la importación.
- **Importaciones más rápidas** — Las importaciones SCID ahora usan transacciones agrupadas en la base de datos, reduciendo notablemente el tiempo de importación en archivos grandes.
- **Código fuente ahora íntegramente en inglés** — Todos los comentarios internos del código fuente se han traducido del francés al inglés, facilitando la lectura y la contribución al proyecto por parte de la comunidad de código abierto.

### Licencia

Distribuido bajo licencia **GNU GPLv3**.
