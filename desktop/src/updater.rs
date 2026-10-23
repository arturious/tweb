// Check for Updates: compares this build's version with the latest GitHub release of REPO
// and, when that is newer, replaces gramm.app with the gramm-<version>.zip attached to it.
// Releasing: bump "version" in tauri.conf.json, run bundle.sh, then create a release tagged
// v<version> with target/gramm-<version>.zip attached

use std::{
  fs,
  path::{Path, PathBuf},
  process::Command,
};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const REPO: &str = "arturious/tweb";
const TITLE: &str = "Check for Updates";

pub fn check(app: AppHandle) {
  // Off the main thread: the dialogs below block until answered
  std::thread::spawn(move || {
    if let Err(message) = run(&app) {
      app.dialog().message(message).title(TITLE).kind(MessageDialogKind::Error).blocking_show();
    }
  });
}

fn run(app: &AppHandle) -> Result<(), String> {
  let release = latest_release()?;
  let current = app.package_info().version.clone();
  let tag = release["tag_name"].as_str().ok_or("The latest release has no tag.")?;
  let latest = semver::Version::parse(tag.trim_start_matches('v'))
    .map_err(|_| format!("The release tag “{tag}” is not a version like v0.2.0."))?;

  if latest <= current {
    app.dialog().message(format!("gramm {current} is the latest version.")).title(TITLE).blocking_show();
    return Ok(());
  }

  let url = release["assets"]
    .as_array()
    .into_iter()
    .flatten()
    .find(|asset| asset["name"].as_str().is_some_and(|name| name.starts_with("gramm") && name.ends_with(".zip")))
    .and_then(|asset| asset["browser_download_url"].as_str())
    .ok_or(format!("Release {tag} has no gramm-*.zip attached."))?;
  let bundle = current_bundle().ok_or("Updates install into gramm.app, and this copy does not run from one.")?;

  let install_now = app
    .dialog()
    .message(format!("gramm {latest} is available — you have {current}. Install it and restart?"))
    .title(TITLE)
    .buttons(MessageDialogButtons::OkCancelCustom("Install and Restart".into(), "Later".into()))
    .blocking_show();
  if install_now {
    install(app, url, &bundle)?;
  }
  Ok(())
}

fn latest_release() -> Result<serde_json::Value, String> {
  let output = Command::new("curl")
    .args(["-fsSL", "-H", "Accept: application/vnd.github+json"])
    .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
    .output()
    .map_err(|e| e.to_string())?;
  if !output.status.success() {
    return Err(format!("No release found at github.com/{REPO}/releases, or GitHub is unreachable."));
  }
  serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}

// gramm.app around the running binary (Contents/MacOS/gramm), if there is one
fn current_bundle() -> Option<PathBuf> {
  let exe = std::env::current_exe().ok()?;
  let bundle = exe.parent()?.parent()?.parent()?.to_path_buf();
  (bundle.extension()? == "app").then_some(bundle)
}

fn install(app: &AppHandle, url: &str, bundle: &Path) -> Result<(), String> {
  let dir = std::env::temp_dir().join(format!("gramm-update-{}", std::process::id()));
  let _ = fs::remove_dir_all(&dir);
  fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

  let zip = dir.join("gramm.zip");
  succeed(Command::new("curl").args(["-fsSL", "-o"]).arg(&zip).arg(url), "The download failed.")?;
  succeed(Command::new("ditto").args(["-x", "-k"]).arg(&zip).arg(&dir), "The update could not be unpacked.")?;
  let new_bundle = dir.join("gramm.app");
  if !new_bundle.join("Contents/MacOS/gramm").is_file() {
    return Err("The downloaded zip holds no gramm.app.".into());
  }

  // A helper outlives this process: it waits for gramm to quit, moves the old bundle aside,
  // puts the new one in its place and opens it. On any failure the old bundle comes back
  let swap = r#"
    while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
    if mv "$2" "$2.old" && mv "$3" "$2"; then rm -rf "$2.old"; else [ -e "$2.old" ] && mv "$2.old" "$2"; fi
    open "$2"
  "#;
  Command::new("/bin/sh")
    .args(["-c", swap, "sh"])
    .arg(std::process::id().to_string())
    .arg(bundle)
    .arg(&new_bundle)
    .spawn()
    .map_err(|e| e.to_string())?;

  app.exit(0);
  Ok(())
}

fn succeed(command: &mut Command, message: &str) -> Result<(), String> {
  match command.status() {
    Ok(status) if status.success() => Ok(()),
    _ => Err(message.into()),
  }
}
