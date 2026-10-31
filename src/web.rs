//! Browser entry point for the experimental WebAssembly demo (see `web/`).
//!
//! The page holds a single gpui window showing the same [`FilingView`] as
//! the desktop app. The browser has no filesystem, so filings arrive as
//! bytes: dropped onto the page, picked with a file input, or fetched from
//! the server (`?file=…` or the "Load sample filing" button), or looked up
//! by FEC filing ID (`?id=…` or the "Open filing by ID…" prompt) through
//! [`PROXY`]. Each path ends in [`deliver`], which hands a [`Source::Bytes`]
//! to the view.

use std::{borrow::Cow, cell::RefCell, rc::Rc};

use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::{
    component::Theme,
    web::{CanvasFontFallback, WebBackendPreference, WebPlatform},
    *,
};
use wasm_bindgen::{JsCast as _, prelude::*};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::{Assets, filing::Source, view::FilingView};

/// The bundled filing the empty state offers, served next to `index.html`.
pub const SAMPLE_FILE: &str = "1944135.fec";

/// CORS proxy for docquery.fec.gov's raw filings (github.com/asg017/fec-proxy).
/// It only answers allowed origins (and localhost), and only filings up to
/// 10 MB in one GET.
const PROXY: &str = "https://fec-cors-proxy.garcia.workers.dev/filings/";

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

pub fn main() {
    gpui_kit::platform::web_init();

    let platform = Rc::new(WebPlatform::new_with_backend_and_font_fallback(
        false,
        WebBackendPreference::Auto,
        CanvasFontFallback::EmojiAndCjk,
    ));
    let app = Application::with_platform(platform).with_assets(Assets);

    let (tx, rx) = mpsc::unbounded();
    DELIVERIES.with(|d| *d.borrow_mut() = Some(tx));

    let handle = app.run_embedded(move |cx| {
        gpui_kit::init(cx);
        crate::debug::init(cx);
        crate::view::init(cx);
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
        if let Some(id) = url_param("id") {
            match parse_filing_id(&id) {
                Some(id) => open_filing_id(id),
                None => deliver(Err(format!("Not a filing ID: {id}"))),
            }
        } else if let Some(file) = url_param("file") {
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
    // A local file can't be put in the URL; don't leave a stale one there.
    set_url_query("");
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
    set_url_param("file", &url);
    let name = url.rsplit('/').next().unwrap_or(&url).to_owned();
    fetch(url, name);
}

/// `1944135`, `FEC-1944135` or `1944135.fec` → `1944135`.
fn parse_filing_id(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s
        .strip_prefix("FEC-")
        .or_else(|| s.strip_prefix("fec-"))
        .unwrap_or(s);
    let s = s.strip_suffix(".fec").unwrap_or(s);
    (!s.is_empty() && s.len() <= 9 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.to_owned())
}

/// Fetch filing `id` from docquery through the proxy, and put it in the URL
/// (`?id=…`) so the page can be reloaded or shared.
fn open_filing_id(id: String) {
    set_url_param("id", &id);
    fetch(format!("{PROXY}{id}.fec"), format!("{id}.fec"));
}

/// Ask for a filing ID with the browser's prompt and open it.
pub fn prompt_filing_id() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(input)) = window.prompt_with_message("FEC filing ID (e.g. 1944135):") else {
        return;
    };
    if input.trim().is_empty() {
        return;
    }
    match parse_filing_id(&input) {
        Some(id) => open_filing_id(id),
        None => deliver(Err(format!("Not a filing ID: {}", input.trim()))),
    }
}

/// Replace the page URL's query with `name=value`, without reloading.
fn set_url_param(name: &str, value: &str) {
    let Ok(params) = web_sys::UrlSearchParams::new() else {
        return;
    };
    params.set(name, value);
    set_url_query(&format!("?{}", String::from(params.to_string())));
}

/// Replace the page URL's query (`""` drops it), without reloading.
fn set_url_query(query: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(location) = window.location().pathname() else {
        return;
    };
    if let Ok(history) = window.history() {
        let _ =
            history.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{location}{query}")));
    }
}

/// Fetch `url` and hand its bytes to the view as a filing called `name`.
fn fetch(url: String, name: String) {
    spawn_local(async move {
        let result = async {
            let window = web_sys::window().ok_or("no window")?;
            let response: web_sys::Response = JsFuture::from(window.fetch_with_str(&url))
                .await
                .map_err(js_error)?
                .unchecked_into();
            if !response.ok() {
                // The proxy explains itself in a short text body.
                let detail = match response.text() {
                    Ok(text) => JsFuture::from(text).await.ok().and_then(|t| t.as_string()),
                    Err(_) => None,
                };
                return Err(
                    match detail.filter(|d| !d.trim().is_empty() && d.len() < 500) {
                        Some(d) => format!("HTTP {}: {}", response.status(), d.trim()),
                        None => format!("HTTP {}", response.status()),
                    },
                );
            }
            let buffer = JsFuture::from(response.array_buffer().map_err(js_error)?)
                .await
                .map_err(js_error)?;
            Ok(js_sys::Uint8Array::new(&buffer).to_vec())
        }
        .await;
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

/// Save `contents` as a download named `name`.
pub fn download(name: &str, contents: &str, mime: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_str_sequence_and_options(
        &js_sys::Array::of1(&contents.into()),
        &options,
    )
    .map_err(js_error)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_error)?;
    let link: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(js_error)?
        .unchecked_into();
    link.set_href(&url);
    link.set_download(name);
    link.click();
    // Revoked once the browser has had a chance to start the download.
    let revoke = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    let _ = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000);
    Ok(())
}
