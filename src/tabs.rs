//! Native macOS window tabs.
//!
//! Every filing window shares a tabbing identifier, so AppKit provides the tab
//! bar, dragging tabs out into their own window / back into another, and the
//! Window menu's next/previous/merge items. What AppKit doesn't do on its own:
//! put new windows into the frontmost window's tab group regardless of the
//! user's "Prefer tabs" setting ([`join_tab_group`]), and ⌘1–⌘9. And gpui
//! (built for Zed, which draws its own tabs) hides the native tab bar, so we
//! undo that ([`restore_native_tab_bar`]).
//!
//! Elsewhere the identifier is ignored and every filing gets its own window.

use gpui_kit::*;

pub const TABBING_IDENTIFIER: &str = "dev.libfec.fec-viewer.filing";

/// Activate the tab at this zero-based position in the active window's group.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = fec_viewer, no_json)]
pub struct SelectTab(pub usize);

actions!(fec_viewer, [SelectLastTab]);

pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::restore_native_tab_bar();

    cx.bind_keys(
        (1..=8).map(|n| KeyBinding::new(&format!("secondary-{n}"), SelectTab(n - 1), None)),
    );
    cx.bind_keys([KeyBinding::new("secondary-9", SelectLastTab, None)]);
    // Deferred: actions dispatch while the active window is mid-update, and a
    // window can't be read from inside its own update.
    cx.on_action(|action: &SelectTab, cx| {
        let ix = action.0;
        cx.defer(move |cx| activate_tab(cx, |tabs| tabs.get(ix)));
    });
    cx.on_action(|_: &SelectLastTab, cx| cx.defer(|cx| activate_tab(cx, |tabs| tabs.last())));
}

fn activate_tab(cx: &mut App, pick: impl FnOnce(&[SystemWindowTab]) -> Option<&SystemWindowTab>) {
    let Some(active) = cx.active_window() else {
        return;
    };
    let Ok(Some(tabs)) = active.update(cx, |_, window, _| window.tabbed_windows()) else {
        return;
    };
    if let Some(tab) = pick(&tabs) {
        let _ = tab
            .handle
            .update(cx, |_, window, _| window.activate_window());
    }
}

/// Add `window` as the last tab of `host`'s tab group, unless it's already in it.
#[cfg(target_os = "macos")]
pub fn join_tab_group(window: AnyWindowHandle, host: AnyWindowHandle, cx: &mut App) {
    let Ok(Some(host)) = host.update(cx, |_, w, _| macos::ns_window(w)) else {
        return;
    };
    let Ok(Some(window)) = window.update(cx, |_, w, _| macos::ns_window(w)) else {
        return;
    };
    unsafe { macos::add_tabbed_window(host, window) };
}

#[cfg(not(target_os = "macos"))]
pub fn join_tab_group(_window: AnyWindowHandle, _host: AnyWindowHandle, _cx: &mut App) {}

#[cfg(target_os = "macos")]
mod macos {
    use gpui_kit::Window;
    use objc2::{
        ffi, msg_send,
        runtime::{AnyClass, AnyObject, Imp, Sel},
        sel,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn ns_window(window: &Window) -> Option<*mut AnyObject> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let view = handle.ns_view.as_ptr() as *mut AnyObject;
        let ns_window: *mut AnyObject = unsafe { msg_send![view, window] };
        (!ns_window.is_null()).then_some(ns_window)
    }

    pub unsafe fn add_tabbed_window(host: *mut AnyObject, window: *mut AnyObject) {
        const NS_WINDOW_ABOVE: isize = 1;
        unsafe {
            let host_group: *mut AnyObject = msg_send![host, tabGroup];
            let group: *mut AnyObject = msg_send![window, tabGroup];
            if host_group == group {
                return;
            }
            // "Above" the last tab, i.e. appended, like a browser.
            let tabs: *mut AnyObject = msg_send![host, tabbedWindows];
            let last: *mut AnyObject = if tabs.is_null() {
                host
            } else {
                msg_send![tabs, lastObject]
            };
            let _: () = msg_send![last, addTabbedWindow: window, ordered: NS_WINDOW_ABOVE];
        }
    }

    /// gpui's `GPUIWindow` overrides a few NSWindow tab methods for Zed, which
    /// draws its own tab bar: `addTitlebarAccessoryViewController:` hides the
    /// native tab bar, and next/previous tab go through gpui's own record of
    /// tab groups, which never sees tabs AppKit rearranges (or that we add in
    /// [`add_tabbed_window`]). Point them back at NSWindow's implementations.
    pub fn restore_native_tab_bar() {
        unsafe extern "C-unwind" fn add_accessory(
            this: &AnyObject,
            _: Sel,
            controller: *mut AnyObject,
        ) {
            unsafe {
                let _: () = msg_send![super(this, ns_window_class()), addTitlebarAccessoryViewController: controller];
            }
        }
        unsafe extern "C-unwind" fn select_next_tab(
            this: &AnyObject,
            _: Sel,
            sender: *mut AnyObject,
        ) {
            unsafe {
                let _: () = msg_send![super(this, ns_window_class()), selectNextTab: sender];
            }
        }
        unsafe extern "C-unwind" fn select_previous_tab(
            this: &AnyObject,
            _: Sel,
            sender: *mut AnyObject,
        ) {
            unsafe {
                let _: () = msg_send![super(this, ns_window_class()), selectPreviousTab: sender];
            }
        }
        fn ns_window_class() -> &'static AnyClass {
            AnyClass::get(c"NSWindow").unwrap()
        }

        let Some(class) = AnyClass::get(c"GPUIWindow") else {
            return;
        };
        type Method = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);
        for (sel, method) in [
            (
                sel!(addTitlebarAccessoryViewController:),
                add_accessory as Method,
            ),
            (sel!(selectNextTab:), select_next_tab as Method),
            (sel!(selectPreviousTab:), select_previous_tab as Method),
        ] {
            unsafe {
                ffi::class_replaceMethod(
                    class as *const AnyClass as *mut AnyClass,
                    sel,
                    std::mem::transmute::<Method, Imp>(method),
                    c"v@:@".as_ptr(),
                );
            }
        }
    }
}
