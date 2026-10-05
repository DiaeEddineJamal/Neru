fn main() {
    // Tauri embeds this ICO in the Windows executable. Rebuild the resource when
    // the artwork changes, even if no Rust source file has changed.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return tauri_build::build();
    }
    // The dialog plugin links TaskDialogIndirect, which only Common Controls 6 has. Tauri's manifest
    // asking for it reaches the app's own binaries only, so `cargo test` binaries failed to start
    // (STATUS_ENTRYPOINT_NOT_FOUND). The linker embeds the same manifest in every binary instead.
    let manifest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app.manifest");
    std::fs::write(&manifest, COMMON_CONTROLS_6).unwrap();
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    let windows = tauri_build::WindowsAttributes::new_without_app_manifest();
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("tauri build");
}

/// tauri-build's default Windows app manifest.
const COMMON_CONTROLS_6: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
"#;
