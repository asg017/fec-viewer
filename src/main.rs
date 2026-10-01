mod filing;
mod tabs;
mod view;

use std::path::PathBuf;

use futures::StreamExt as _;
use gpui_kit::*;

use crate::view::FilingView;

actions!(fec_viewer, [Open, CloseWindow, Minimize, Zoom, Quit]);

/// Windows we've opened, so "Open…" can reuse an empty one.
#[derive(Default)]
struct OpenWindows(Vec<(AnyWindowHandle, WeakEntity<FilingView>)>);
impl Global for OpenWindows {}

pub fn open_filing_window(path: Option<PathBuf>, cx: &mut App) {
    // Reuse an empty window (e.g. the one shown at launch) if there is one.
    if let Some(path) = path.clone() {
        let windows = cx.default_global::<OpenWindows>().0.clone();
        let empty = windows
            .into_iter()
            .filter_map(|(h, v)| Some((h, v.upgrade()?)))
            .find(|(_, v)| !v.read(cx).has_filing());
        if let Some((handle, view)) = empty {
            let _ = handle.update(cx, |_, window, cx| {
                view.update(cx, |v, cx| v.load(path, window, cx));
                window.activate_window();
            });
            return;
        }
    }

    // New filings open as a tab of the frontmost filing window.
    let host = {
        let ours = cx.default_global::<OpenWindows>().0.clone();
        cx.window_stack()
            .unwrap_or_default()
            .into_iter()
            .find(|h| ours.iter().any(|(o, v)| o == h && v.upgrade().is_some()))
    };

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("FEC Viewer".into()),
            ..Default::default()
        }),
        tabbing_identifier: Some(tabs::TABBING_IDENTIFIER.into()),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| FilingView::new(path, window, cx))
    }) {
        Ok((handle, view)) => {
            cx.default_global::<OpenWindows>()
                .0
                .push((handle, view.downgrade()));
            if let Some(host) = host {
                tabs::join_tab_group(handle, host, cx);
            }
        }
        Err(e) => eprintln!("failed to open window: {e:#}"),
    }
}

/// Run `f` on the active window. Deferred because actions dispatch while that
/// window is mid-update, and a window can't be updated from inside itself.
fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window) + 'static) {
    cx.defer(|cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| f(window));
        }
    });
}

/// Finder / LaunchServices hand us `file://` URLs.
fn path_from_url(url: &str) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://")?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

fn main() {
    let (url_tx, mut url_rx) = futures::channel::mpsc::unbounded::<Vec<String>>();
    let app = gpui_kit::application();
    app.on_open_urls(move |urls| {
        let _ = url_tx.unbounded_send(urls);
    });

    app.run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::component::Theme::sync_system_appearance(None, cx);

        cx.bind_keys([
            KeyBinding::new("secondary-o", Open, None),
            KeyBinding::new("secondary-w", CloseWindow, None),
            KeyBinding::new("secondary-q", Quit, None),
            KeyBinding::new("secondary-m", Minimize, None),
        ]);
        tabs::init(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &CloseWindow, cx| with_active_window(cx, |window| window.remove_window()));
        cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window| window.minimize_window()));
        cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window| window.zoom_window()));
        cx.on_action(|_: &Open, cx| {
            let rx = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: true,
                prompt: Some("Open".into()),
            });
            cx.spawn(async move |cx| {
                if let Ok(Ok(Some(paths))) = rx.await {
                    cx.update(|cx| {
                        for path in paths {
                            open_filing_window(Some(path), cx);
                        }
                    });
                }
            })
            .detach();
        });
        cx.set_menus(vec![
            Menu {
                name: "FEC Viewer".into(),
                items: vec![MenuItem::action("Quit FEC Viewer", Quit)],
                disabled: false,
            },
            Menu {
                name: "File".into(),
                items: vec![
                    MenuItem::action("Open…", Open),
                    MenuItem::separator(),
                    MenuItem::action("Close Window", CloseWindow),
                ],
                disabled: false,
            },
            // Named "Window" so AppKit adopts it as the windows menu and adds
            // its tab items (next/previous tab, move to new window, merge all).
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Zoom", Zoom),
                ],
                disabled: false,
            },
        ]);
        #[cfg(not(target_os = "macos"))]
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        // Paths from argv (Windows/Linux file associations, or the terminal)
        // plus any URLs macOS delivered before launch finished.
        let mut initial: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        while let Ok(Some(urls)) = url_rx.try_recv().map(Some) {
            initial.extend(urls.iter().filter_map(|u| path_from_url(u)));
        }
        if initial.is_empty() {
            open_filing_window(None, cx);
        }
        for path in initial {
            open_filing_window(Some(path), cx);
        }

        cx.spawn(async move |cx| {
            while let Some(urls) = url_rx.next().await {
                cx.update(|cx| {
                    for path in urls.iter().filter_map(|u| path_from_url(u)) {
                        open_filing_window(Some(path), cx);
                    }
                    cx.activate(true);
                });
            }
        })
        .detach();
        cx.activate(true);
    });
}
