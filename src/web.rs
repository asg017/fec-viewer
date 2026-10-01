//! Browser entry point for the experimental WebAssembly demo (see `web/`).
//!
//! The page holds a single gpui window showing the same [`FilingView`] as
//! the desktop app. The browser has no filesystem, so filings arrive as
//! bytes: dropped onto the page, picked with a file input, or fetched from
//! the server (`?file=…` or the "Load sample filing" button). Each path ends
//! in [`deliver`], which hands a [`Source::Bytes`] to the view.

use std::{borrow::Cow, cell::RefCell, rc::Rc};

use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::{
    component::Theme,
    web::{CanvasFontFallback, WebBackendPreference, WebPlatform},
    *,
};
use wasm_bindgen::{JsCast as _, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::{filing::Source, view::FilingView};

/// The bundled filing the empty state offers, served next to `index.html`.
pub const SAMPLE_FILE: &str = "1944135.fec";

type Delivery = Result<Source, String>;

thread_local! {
    /// Keeps the application alive after `main` returns to the browser.
    static APP: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
    static DELIVERIES: RefCell<Option<mpsc::UnboundedSender<Delivery>>> =
        const { RefCell::new(None) };
    /// DOM listeners, kept alive for the life of the page.
    static LISTENERS: RefCell<Vec<Closure<dyn FnMut(web_sys::Event)>>> =
        const { RefCell::new(Vec::new()) };
}

// Only the icons the viewer draws, embedded so the demo works offline.
gpui_kit::assets::icon_assets!(
    WebAssets,
    [ChevronDown, ChevronRight, ChevronsUpDown, Inbox]
);

pub fn main() {
    gpui_kit::platform::web_init();

    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        WebBackendPreference::Auto,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let app = Application::with_platform(platform).with_assets(WebAssets);

    let (tx, rx) = mpsc::unbounded();
    DELIVERIES.with(|d| *d.borrow_mut() = Some(tx));

    let handle = app.run_embedded(move |cx| {
        gpui_kit::init(cx);
        // The web platform starts with no fonts. These must be in place
        // before the first window lays out any text. gpui's `.SystemUIFont`
        // resolves to IBM Plex Sans here.
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../web/fonts/IBMPlexSans-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../web/fonts/JetBrainsMono-Regular.ttf").as_slice()),
            ])
            .expect("failed to load bundled fonts");
        Theme::sync_system_appearance(None, cx);
        Theme::update(cx, |theme| theme.mono_font_family = "JetBrains Mono".into());

        let (window, view) =
            match gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| FilingView::new(None, window, cx))
            }) {
                Ok(opened) => opened,
                Err(e) => {
                    web_sys::console::error_1(&format!("failed to open window: {e:#}").into());
                    return;
                }
            };
        cx.activate(true);

        cx.spawn(async move |cx| {
            let mut rx = rx;
            while let Some(delivery) = rx.next().await {
                let view = view.clone();
                let _ = window.update(cx, |_, window, cx| {
                    view.update(cx, |v, cx| match delivery {
                        Ok(source) => v.load(source, window, cx),
                        Err(e) => v.show_error(e, cx),
                    })
                });
            }
        })
        .detach();

        listen_for_drops();
        if let Some(file) = url_param("file") {
            fetch_file(file);
        }
    });
    APP.with(|app| *app.borrow_mut() = Some(handle));
}

fn deliver(delivery: Delivery) {
    if let Err(e) = &delivery {
        web_sys::console::error_1(&e.into());
    }
    DELIVERIES.with(|d| {
        if let Some(tx) = d.borrow().as_ref() {
            let _ = tx.unbounded_send(delivery);
        }
    });
}

fn js_error(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            e.dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Read a browser `File` into memory and hand it to the view.
fn read_file(file: web_sys::File) {
    spawn_local(async move {
        let name = file.name();
        let delivery = match JsFuture::from(file.array_buffer()).await {
            Ok(buffer) => Ok(Source::Bytes {
                name,
                bytes: js_sys::Uint8Array::new(&buffer).to_vec().into(),
            }),
            Err(e) => Err(format!("Failed to read {name}: {}", js_error(e))),
        };
        deliver(delivery);
    });
}

/// Fetch a filing from the server (relative URLs resolve against the page).
pub fn fetch_file(url: String) {
    spawn_local(async move {
        let result = async {
            let window = web_sys::window().ok_or("no window")?;
            let response: web_sys::Response = JsFuture::from(window.fetch_with_str(&url))
                .await
                .map_err(js_error)?
                .unchecked_into();
            if !response.ok() {
                return Err(format!("HTTP {}", response.status()));
            }
            let buffer = JsFuture::from(response.array_buffer().map_err(js_error)?)
                .await
                .map_err(js_error)?;
            Ok(js_sys::Uint8Array::new(&buffer).to_vec())
        }
        .await;
        let name = url.rsplit('/').next().unwrap_or(&url).to_owned();
        deliver(match result {
            Ok(bytes) => Ok(Source::Bytes {
                name,
                bytes: bytes.into(),
            }),
            Err(e) => Err(format!("Failed to fetch {url}: {e}")),
        });
    });
}

/// Show the browser's file picker. Must run while handling a user gesture
/// (gpui click handlers do), or the browser ignores it.
pub fn choose_file() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(input) = document.create_element("input") else {
        return;
    };
    let input: web_sys::HtmlInputElement = input.unchecked_into();
    input.set_type("file");
    input.set_accept(".fec,text/plain");
    let target = input.clone();
    let on_change = Closure::once_into_js(move |_: web_sys::Event| {
        if let Some(file) = target.files().and_then(|files| files.get(0)) {
            read_file(file);
        }
    });
    input.set_onchange(Some(on_change.unchecked_ref()));
    input.click();
}

/// gpui's canvas swallows drop events without exposing the files, so listen
/// on the document (the events bubble there) and read the files ourselves.
fn listen_for_drops() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let on_dragover = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        // Without this the browser refuses the drop.
        event.prevent_default();
    });
    let on_drop = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        event.prevent_default();
        let event: web_sys::DragEvent = event.unchecked_into();
        // One window, so only the first file is opened.
        if let Some(file) = event
            .data_transfer()
            .and_then(|dt| dt.files())
            .and_then(|files| files.get(0))
        {
            read_file(file);
        }
    });
    for (name, listener) in [("dragover", on_dragover), ("drop", on_drop)] {
        let _ = document.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
        LISTENERS.with(|l| l.borrow_mut().push(listener));
    }
}

fn url_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search)
        .ok()?
        .get(name)
        .filter(|v| !v.is_empty())
}
