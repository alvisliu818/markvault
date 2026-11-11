fn main() {
    // Declaring app commands generates allow-<cmd>/deny-<cmd> permissions so a
    // capability can grant them to the remote main window (the SPA loads from
    // http://127.0.0.1:<port>, i.e. a remote origin — remote content can never
    // reach custom commands without an explicit capability).
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "browser_open",
            "browser_navigate",
            "browser_act",
            "browser_close",
            "browser_focus",
            "browser_list",
        ])),
    )
    .expect("tauri build failed");
}
