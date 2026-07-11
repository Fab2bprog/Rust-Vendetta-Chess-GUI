// build.rs — compilation des fichiers .slint en Rust.
fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");
}
