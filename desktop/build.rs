fn main() {
  // Declaring the app's commands makes each one a permission (allow-<command>), so the
  // capabilities can hand the top bar and the Telegram webview only the ones they need
  tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
    tauri_build::AppManifest::new().commands(&["go_home", "navigate", "notify", "set_badge"]),
  ))
  .expect("failed to run tauri-build");
}
