mod model;
mod pricing;
mod providers;
mod store;
mod watcher;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, WebviewWindow, WindowEvent};

use model::Tool;
use store::{Snapshot, Store};

const PANEL: &str = "panel";
const PANEL_WIDTH: f64 = 380.0;
const TRAY_ID: &str = "main";

struct AppState {
    store: Mutex<Store>,
    /// Where the tray icon was last clicked, to place the panel next to it.
    anchor: Mutex<Option<PhysicalRect<f64, f64>>>,
    /// When the panel was last hidden by losing focus. Clicking the tray icon to close an
    /// open panel first blurs it, so the click must not immediately reopen it.
    hidden_at: Mutex<Option<Instant>>,
}

#[tauri::command]
fn get_snapshot(state: tauri::State<AppState>) -> Snapshot {
    state.store.lock().unwrap().snapshot(Utc::now())
}

#[tauri::command]
fn set_filter(app: AppHandle, state: tauri::State<AppState>, filter: Option<Tool>) -> Snapshot {
    let snapshot = {
        let mut store = state.store.lock().unwrap();
        store.filter = filter;
        store.snapshot(Utc::now())
    };
    update_tray(&app, &snapshot);
    snapshot
}

/// Called by the page whenever its content height changes.
#[tauri::command]
fn fit_window(window: WebviewWindow, state: tauri::State<AppState>, height: f64) {
    let anchor = *state.anchor.lock().unwrap();
    place(&window, height, anchor);
}

#[tauri::command]
fn hide_window(window: WebviewWindow) {
    let _ = window.hide();
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Sizes the panel to its content and pins it next to the tray icon: below it when the
/// tray is at the top of the screen (macOS, most Linux desktops), above it otherwise (Windows).
fn place(window: &WebviewWindow, height: f64, anchor: Option<PhysicalRect<f64, f64>>) {
    let Ok(scale) = window.scale_factor() else { return };
    let (width, height) = (PANEL_WIDTH * scale, height.ceil() * scale);
    let _ = window.set_size(tauri::PhysicalSize::new(width, height));

    let monitor = anchor
        .and_then(|a| window.monitor_from_point(a.position.x, a.position.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let area = monitor.work_area();
    let (left, top) = (area.position.x as f64, area.position.y as f64);
    let (right, bottom) = (left + area.size.width as f64, top + area.size.height as f64);
    let gap = 6.0 * scale;

    let (x, y) = match anchor {
        Some(a) => {
            let x = a.position.x + a.size.width / 2.0 - width / 2.0;
            let tray_at_top = a.position.y < top + (bottom - top) / 2.0;
            let y = if tray_at_top { a.position.y + a.size.height + gap } else { a.position.y - height - gap };
            (x, y)
        }
        // No click position (Linux app indicators): top-right corner.
        None => (right - width - gap * 2.0, top + gap),
    };
    let x = x.clamp(left + gap, (right - width - gap).max(left));
    let y = y.clamp(top, (bottom - height).max(top));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Lets the panel appear over full-screen apps. By default a macOS window belongs to one
/// Space, so opening it while another app is full screen showed nothing (it opened on the
/// desktop Space instead). Also raises it to the pop-up menu level, like a real menu.
#[cfg(target_os = "macos")]
fn float_over_fullscreen(window: &WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    const POP_UP_MENU_LEVEL: isize = 101; // kCGPopUpMenuWindowLevel
    let Ok(ptr) = window.ns_window() else { return };
    // SAFETY: Tauri returns the panel's live NSWindow, and this runs on the main thread.
    let ns_window = unsafe { &*(ptr as *const NSWindow) };
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient,
    );
    ns_window.setLevel(POP_UP_MENU_LEVEL);
}

fn toggle_panel(app: &AppHandle, anchor: Option<PhysicalRect<f64, f64>>) {
    let Some(window) = app.get_webview_window(PANEL) else { return };
    let state = app.state::<AppState>();
    let just_hidden = state.hidden_at.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(300));
    if window.is_visible().unwrap_or(false) || just_hidden {
        let _ = window.hide();
        return;
    }
    if anchor.is_some() {
        *state.anchor.lock().unwrap() = anchor;
    }
    // Re-place at the current height; the page only reports height when it changes.
    if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
        place(&window, size.height as f64 / scale, *state.anchor.lock().unwrap());
    }
    let _ = window.emit("panel-shown", ());
    let _ = window.show();
    let _ = window.set_focus();
}

fn format_tokens(n: u64) -> String {
    let v = n as f64;
    match n {
        1_000_000_000.. => format!("{:.2}B", v / 1e9),
        1_000_000.. => format!("{:.1}M", v / 1e6),
        1_000.. => format!("{:.1}K", v / 1e3),
        _ => n.to_string(),
    }
}

fn update_tray(app: &AppHandle, snapshot: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let today = format_tokens(snapshot.today_all_tools);
    // Only macOS shows text next to tray icons; elsewhere the tooltip carries the number.
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(Some(format!(" {today}")));
    let _ = tray.set_tooltip(Some(format!("Claude Usage Monitor · today {today} tokens")));
}

fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &quit])?;

    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Claude Usage Monitor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => toggle_panel(app, None),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                let scale = tray.app_handle().get_webview_window(PANEL).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
                let anchor = PhysicalRect { position: rect.position.to_physical(scale), size: rect.size.to_physical(scale) };
                toggle_panel(tray.app_handle(), Some(anchor));
            }
        })
        .build(app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_single_instance::init(|app, _, _| toggle_panel(app, None)))
        .manage(AppState { store: Mutex::new(Store::new()), anchor: Mutex::new(None), hidden_at: Mutex::new(None) })
        .invoke_handler(tauri::generate_handler![get_snapshot, set_filter, fit_window, hide_window, quit])
        .setup(|app| {
            // Tray only: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            build_tray(app.handle())?;

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window(PANEL) {
                float_over_fullscreen(&window);
            }

            let handle = app.handle().clone();
            watcher::spawn(providers::all(), move |out, initial| {
                let state = handle.state::<AppState>();
                let snapshot = {
                    let mut store = state.store.lock().unwrap();
                    store.ingest(out, initial);
                    store.snapshot(Utc::now())
                };
                update_tray(&handle, &snapshot);
                let _ = handle.emit("snapshot", &snapshot);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                let _ = window.hide();
                *window.state::<AppState>().hidden_at.lock().unwrap() = Some(Instant::now());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Claude Usage Monitor");
}
