fn main() {
    // Tauri embeds this ICO in the Windows executable. Rebuild the resource when
    // the artwork changes, even if no Rust source file has changed.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    tauri_build::build()
}
