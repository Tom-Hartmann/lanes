fn main() {
    // Embed our own manifest so the execution level is explicit (asInvoker).
    let windows = tauri_build::WindowsAttributes::new().app_manifest(include_str!("lanes.manifest"));
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("tauri-build failed");
}
