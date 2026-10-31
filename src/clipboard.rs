//! Desktop clipboard writes that gpui can't do: HTML with a plain-text
//! fallback, so a copied table pastes into spreadsheets as columns and into
//! text editors as the text. (The browser's equivalent is in `web.rs`.)

use std::cell::RefCell;

thread_local! {
    /// Kept for the life of the app: on Linux (X11) the copied contents are
    /// served from this, and vanish if it's dropped.
    static CLIPBOARD: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) };
}

/// Put `html` on the clipboard, with `text` for apps that don't read HTML.
pub fn write_html(text: &str, html: &str) -> Result<(), String> {
    CLIPBOARD.with(|clipboard| {
        let mut clipboard = clipboard.borrow_mut();
        if clipboard.is_none() {
            *clipboard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        let clipboard = clipboard.as_mut().expect("just made");
        clipboard
            .set_html(html, Some(text))
            .map_err(|e| e.to_string())
    })
}
