mod server;
mod updater;

use std::{path::PathBuf, sync::Mutex};
use tauri::{
  menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem},
  utils::config::BackgroundThrottlingPolicy,
  webview::PageLoadEvent,
  LogicalPosition, Manager, LogicalSize, TitleBarStyle, Webview, WebviewBuilder, WebviewUrl,
  WebviewWindowBuilder, WindowEvent,
};

// The inset's y does not move the traffic lights; an empty compact NSToolbar does:
// AppKit then centres them 20pt from the top, so a 40pt bar shares their middle line
const TOPBAR: f64 = 40.0;
const INSET: f64 = 8.0;
// The corner radius macOS 26 gives windows with a toolbar. This transparent window would
// otherwise get a tighter one, so its frame is rounded to it (set_window_corner_radius)
const WINDOW_RADIUS: f64 = 26.0;
// Telegram's card is concentric with the window: the window's radius less the inset
const RADIUS: f64 = WINDOW_RADIUS - INSET;

// Blur of what is behind the window, in points (0 turns it off). Its tint and
// opacity are the shell page's background: --glass in shell/index.html
const BLUR_RADIUS: i32 = 30;

const ZOOM_STEP: f64 = 0.05;
const ZOOM_MIN: f64 = 0.5;
const ZOOM_MAX: f64 = 3.0;

fn content_bounds(size: LogicalSize<f64>) -> (LogicalPosition<f64>, LogicalSize<f64>) {
  (
    LogicalPosition::new(INSET, TOPBAR),
    LogicalSize::new(
      (size.width - INSET * 2.0).max(0.0),
      (size.height - TOPBAR - INSET).max(0.0),
    ),
  )
}

// Back to the screen you land on after sign-in: the chat list, with no chat and no inner tab open
const GO_HOME_JS: &str = r#"(async() => {
  if(window.appSidebarLeft && (await appSidebarLeft.closeEverythingInsideNaturally()) === false) return;
  window.appImManager?.setPeer({});
})();"#;

#[tauri::command]
fn go_home(app: tauri::AppHandle) -> Result<(), String> {
  let tg = app.get_webview("tg").ok_or("Telegram webview is missing")?;
  tg.eval(GO_HOME_JS).map_err(|e| e.to_string())
}

// Telegram Web keeps its chats and screens in the page history, so back/forward walk them
fn navigate_tg(tg: &Webview, action: &str) -> tauri::Result<()> {
  match action {
    "back" => tg.eval("history.back()"),
    "forward" => tg.eval("history.forward()"),
    "reload" => tg.reload(),
    "stop" => {
      // A stopped load never reports Finished, so the bar is told it is over
      tg.eval("window.stop()")?;
      set_loading(tg.app_handle(), false);
      Ok(())
    }
    _ => Ok(()),
  }
}

#[tauri::command]
fn navigate(app: tauri::AppHandle, action: String) -> Result<(), String> {
  let tg = app.get_webview("tg").ok_or("Telegram webview is missing")?;
  navigate_tg(&tg, &action).map_err(|e| e.to_string())
}

// Turns the bar's reload button into stop while Telegram loads, and back
fn set_loading(app: &tauri::AppHandle, loading: bool) {
  if let Some(shell) = app.get_webview("shell") {
    let _ = shell.eval(format!("window.setLoading?.({loading})"));
  }
}

// WKWebView has no Web Notifications and no Badging API. Telegram Web uses exactly
// `new Notification(title, {body, tag})` and `navigator.setAppBadge(count)`
// (lib/uiNotificationsManager.ts), so these stand-ins hand both to the app:
// a macOS notification and the Dock badge. Clicking a macOS notification brings gramm forward
const TG_INIT_JS: &str = r#"(() => {
  const invoke = (cmd, args) => window.__TAURI_INTERNALS__?.invoke(cmd, args)?.catch(() => {});

  class AppNotification extends EventTarget {
    static get permission() { return 'granted'; }
    static requestPermission(callback) {
      callback?.('granted');
      return Promise.resolve('granted');
    }

    constructor(title, options = {}) {
      super();
      this.title = String(title);
      this.body = options.body || '';
      this.tag = options.tag || '';
      this.data = options.data;
      this.onclick = this.onclose = this.onshow = this.onerror = null;
      invoke('notify', {title: this.title, body: this.body});
    }

    close() {}
  }

  Object.defineProperty(window, 'Notification', {value: AppNotification, configurable: true, writable: true});
  navigator.setAppBadge = (count) => invoke('set_badge', {count: Math.max(0, Number(count) || 0)});
  navigator.clearAppBadge = () => invoke('set_badge', {count: 0});
})();"#;

#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
  use tauri_plugin_notification::NotificationExt;
  app.notification().builder().title(title).body(body).show().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_badge(app: tauri::AppHandle, count: i64) -> Result<(), String> {
  let window = app.get_window("shell").ok_or("window is missing")?;
  window.set_badge_count((count > 0).then_some(count)).map_err(|e| e.to_string())
}

// Where the built Telegram Web is: TG_WEB, else inside gramm.app (bundle.sh lays dist over the
// assets it needs from public), else the repo this crate sits in
fn tweb_roots() -> Vec<PathBuf> {
  if let Ok(dir) = std::env::var("TG_WEB") {
    let dir = PathBuf::from(dir);
    return vec![dir.join("dist"), dir.join("public")];
  }
  let bundled = std::env::current_exe()
    .ok()
    .and_then(|exe| Some(exe.parent()?.parent()?.join("Resources/tweb")));
  if let Some(bundled) = bundled.filter(|dir| dir.is_dir()) {
    return vec![bundled];
  }
  let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
  vec![repo.join("dist"), repo.join("public")]
}

fn main() {
  // The built tweb by default (`pnpm exec vite build`); TG_URL=http://localhost:8080 runs
  // against the dev server instead
  let tg_url = match std::env::var("TG_URL") {
    Ok(url) => url,
    Err(_) => server::start(tweb_roots()).expect("could not serve Telegram"),
  };

  tauri::Builder::default()
    .plugin(tauri_plugin_notification::init())
    .plugin(tauri_plugin_dialog::init())
    .invoke_handler(tauri::generate_handler![go_home, navigate, notify, set_badge])
    .setup(move |app| {
      // The window's own webview is the shell (top bar + background) and follows the window size
      let shell = WebviewWindowBuilder::new(app, "shell", WebviewUrl::App("index.html".into()))
        .title("gramm")
        .inner_size(1200.0, 800.0)
        .min_inner_size(480.0, 360.0)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        // With the toolbar the zoom button ends at 72pt. y only sizes the buttons' title bar
        // container (button height 14 + y), kept to the top bar's height so it does not catch
        // clicks below the bar
        .traffic_light_position(LogicalPosition::new(16.0, TOPBAR - 14.0))
        // A see-through window: the shell page paints a translucent tint over a blur of
        // what is behind, in the top bar and the inset around the Telegram webview
        .transparent(true)
        .build()?;
      let window = shell.as_ref().window();

      #[cfg(target_os = "macos")]
      if let Ok(ns_window) = window.ns_window() {
        unsafe {
          add_compact_toolbar(ns_window.cast());
          set_background_blur(ns_window.cast(), BLUR_RADIUS);
          set_window_corner_radius(ns_window.cast(), WINDOW_RADIUS);
          disable_app_nap();
        }
      }

      let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
      let (pos, content_size) = content_bounds(size);
      let tg = window.add_child(
        WebviewBuilder::new("tg", WebviewUrl::External(tg_url.parse()?))
          .initialization_script(TG_INIT_JS)
          // WebKit would otherwise suspend a hidden or minimized view after a few minutes,
          // and messages and calls would stop arriving
          .background_throttling(BackgroundThrottlingPolicy::Disabled)
          .on_page_load(|webview, payload| {
            set_loading(webview.app_handle(), payload.event() == PageLoadEvent::Started);
          }),
        pos,
        content_size,
      )?;

      #[cfg(target_os = "macos")]
      tg.with_webview(|webview| unsafe { round_corners(webview.inner().cast(), RADIUS) })?;

      // Cmd+[ / Cmd+] / Cmd+R navigate and Cmd+= / Cmd+- / Cmd+0 zoom the Telegram webview.
      // They live in the native View menu, so they work wherever focus is and Telegram itself
      // needs no access to Tauri
      let handle = app.handle();
      let menu = Menu::default(handle)?;
      let back = MenuItem::with_id(handle, "nav_back", "Back", true, Some("CmdOrCtrl+["))?;
      let forward = MenuItem::with_id(handle, "nav_forward", "Forward", true, Some("CmdOrCtrl+]"))?;
      let reload = MenuItem::with_id(handle, "nav_reload", "Reload", true, Some("CmdOrCtrl+R"))?;
      let zoom_in = MenuItem::with_id(handle, "zoom_in", "Zoom In", true, Some("CmdOrCtrl+="))?;
      let zoom_out = MenuItem::with_id(handle, "zoom_out", "Zoom Out", true, Some("CmdOrCtrl+-"))?;
      let zoom_reset = MenuItem::with_id(handle, "zoom_reset", "Actual Size", true, Some("CmdOrCtrl+0"))?;
      let separator = PredefinedMenuItem::separator(handle)?;
      let separator2 = PredefinedMenuItem::separator(handle)?;
      let check_updates = MenuItem::with_id(handle, "check_updates", "Check for Updates…", true, None::<&str>)?;
      for (index, item) in menu.items()?.into_iter().enumerate() {
        if let MenuItemKind::Submenu(submenu) = item {
          // The app menu (gramm) comes first: the item goes right under About gramm
          if index == 0 {
            submenu.insert(&check_updates, 1)?;
          }
          if submenu.text()? == "View" {
            submenu.insert_items(
              &[&back, &forward, &reload, &separator2, &zoom_in, &zoom_out, &zoom_reset, &separator],
              0,
            )?;
          }
        }
      }
      app.set_menu(menu)?;

      let zoom = Mutex::new(1.0_f64);
      let tg_zoom = tg.clone();
      app.on_menu_event(move |app, event| {
        if event.id().as_ref() == "check_updates" {
          updater::check(app.clone());
          return;
        }
        if let Some(action) = event.id().as_ref().strip_prefix("nav_") {
          let _ = navigate_tg(&tg_zoom, action);
          return;
        }
        let mut level = zoom.lock().unwrap();
        *level = match event.id().as_ref() {
          // Rounded to whole percents, so repeated 5% steps do not drift (0.95000000001)
          "zoom_in" => ((*level + ZOOM_STEP) * 100.0).round().min(ZOOM_MAX * 100.0) / 100.0,
          "zoom_out" => ((*level - ZOOM_STEP) * 100.0).round().max(ZOOM_MIN * 100.0) / 100.0,
          "zoom_reset" => 1.0,
          _ => return,
        };
        let _ = tg_zoom.set_zoom(*level);
      });

      let win = window.clone();
      window.on_window_event(move |event| {
        if let WindowEvent::Resized(physical) = event {
          let size = physical.to_logical::<f64>(win.scale_factor().unwrap_or(1.0));
          let (pos, content_size) = content_bounds(size);
          let _ = tg.set_position(pos);
          let _ = tg.set_size(content_size);
        }
      });

      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running gramm");
}

// An empty toolbar in the unified compact style (NSWindowToolbarStyleUnifiedCompact = 4)
// makes the title bar taller and centres the traffic lights in it
#[cfg(target_os = "macos")]
unsafe fn add_compact_toolbar(ns_window: *mut objc2::runtime::AnyObject) {
  use objc2::{class, msg_send, runtime::AnyObject};
  use objc2_foundation::NSString;

  let identifier = NSString::from_str("tg");
  let toolbar: *mut AnyObject = msg_send![class!(NSToolbar), alloc];
  let toolbar: *mut AnyObject = msg_send![toolbar, initWithIdentifier: &*identifier];
  let _: () = msg_send![ns_window, setToolbar: toolbar];
  let _: () = msg_send![ns_window, setToolbarStyle: 4isize];
}

// The window-server call Ghostty and iTerm2 use for their background blur. It is a private
// CoreGraphics function: there is no public API for a blur of a chosen radius
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
  fn CGSMainConnectionID() -> i32;
  fn CGSSetWindowBackgroundBlurRadius(connection: i32, window: u32, radius: i32) -> i32;
}

#[cfg(target_os = "macos")]
unsafe fn set_background_blur(ns_window: *mut objc2::runtime::AnyObject, radius: i32) {
  use objc2::msg_send;

  let number: isize = msg_send![ns_window, windowNumber];
  CGSSetWindowBackgroundBlurRadius(CGSMainConnectionID(), number as u32, radius);
}

// App Nap would throttle gramm while its window is hidden or covered. A user-initiated
// activity held for the app's whole life keeps it awake; idle system sleep stays allowed.
// (Info.plist's NSAppSleepDisabled does the same for gramm.app; this covers `cargo run` too)
#[cfg(target_os = "macos")]
unsafe fn disable_app_nap() {
  use objc2::{class, msg_send, runtime::AnyObject};
  use objc2_foundation::NSString;

  const USER_INITIATED: u64 = 0x00FF_FFFF;
  const IDLE_SYSTEM_SLEEP_DISABLED: u64 = 1 << 20;

  let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
  let reason = NSString::from_str("Telegram keeps receiving messages and calls");
  let activity: *mut AnyObject = msg_send![
    info,
    beginActivityWithOptions: USER_INITIATED & !IDLE_SYSTEM_SLEEP_DISABLED,
    reason: &*reason
  ];
  // Never ended: retained so it lives as long as the process
  let _: *mut AnyObject = msg_send![activity, retain];
}

// A transparent window is not clipped by AppKit's own corner shape, so its corners come
// from the layer of the window's frame view: rounded here to `radius` with the continuous
// curve macOS windows use. The shadow is then rebuilt from the new outline
#[cfg(target_os = "macos")]
unsafe fn set_window_corner_radius(ns_window: *mut objc2::runtime::AnyObject, radius: f64) {
  use objc2::{msg_send, runtime::AnyObject};
  use objc2_foundation::NSString;

  let content: *mut AnyObject = msg_send![ns_window, contentView];
  let frame: *mut AnyObject = msg_send![content, superview];
  let _: () = msg_send![frame, setWantsLayer: true];
  let layer: *mut AnyObject = msg_send![frame, layer];
  if !layer.is_null() {
    let continuous = NSString::from_str("continuous");
    let _: () = msg_send![layer, setCornerRadius: radius];
    let _: () = msg_send![layer, setCornerCurve: &*continuous];
    let _: () = msg_send![layer, setMasksToBounds: true];
  }
  let _: () = msg_send![ns_window, invalidateShadow];
}

#[cfg(target_os = "macos")]
unsafe fn round_corners(view: *mut objc2::runtime::AnyObject, radius: f64) {
  use objc2::{msg_send, runtime::AnyObject};

  let _: () = msg_send![view, setWantsLayer: true];
  let layer: *mut AnyObject = msg_send![view, layer];
  if !layer.is_null() {
    let _: () = msg_send![layer, setCornerRadius: radius];
    let _: () = msg_send![layer, setMasksToBounds: true];
  }
}
