// build.rs — compilation des fichiers .slint en Rust.
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");

    // Embarque assets/icon/icon.ico dans l'exécutable Windows (icône
    // Explorateur/barre des tâches/Alt-Tab) — sans quoi le binaire affiche
    // l'icône générique "exécutable inconnu" de Windows, contrairement au
    // bundle macOS (`.icns`, cf. [package.metadata.bundle]) et à l'icône
    // Linux (`crates/gui/linux/vendetta-chess.png`), déjà en place.
    // `#[cfg(windows)]` sur ce host build.rs (compilé pour l'hôte, pas la
    // cible finale) est correct ici car ce projet ne cross-compile pas les
    // binaires Windows depuis macOS/Linux : ils sont toujours produits par
    // une compilation native sur Windows, où hôte == cible.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.compile().expect("Impossible d'intégrer l'icône Windows dans l'exécutable");
    }
}
