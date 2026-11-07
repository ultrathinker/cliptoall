//! macOS 27: left-click on the menu-bar icon must reach our handler.
//!
//! tray-icon 0.21 (the version Tauri 2.10 pins) attaches the tray's context
//! menu to the `NSStatusItem` permanently (`statusItem.menu = menu`). On macOS
//! 27, AppKit then takes over every click on the item itself, so tray-icon's
//! own click view never sees a left click: no `TrayIconEvent::Click` reaches
//! `on_tray_icon_event` and a left click does nothing, although the right-click
//! menu still opens. (An event posted to the status-bar window from inside the
//! process still arrives, because it skips that system routing; only a real
//! click is affected.)
//!
//! Upstream fixed this in tray-icon 0.25.1 (tauri-apps/tray-icon#365) by
//! attaching the menu only while it is being shown. Until Tauri picks that
//! up, this module does the same from the outside:
//!   * `detach_menu` takes the menu off the status item once, after the tray
//!     is built, and keeps it;
//!   * `show_menu` puts it back, opens it (`performClick`, which runs the menu
//!     until it closes) and takes it off again, for a right click.
//!
//! Remove this module when the bundled tray-icon contains #365.
//!
//! Everything here runs on the main thread (tray setup and tray events both
//! do), so the retained objects live in a thread-local.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::msg_send;
use std::cell::RefCell;

struct Detached {
    status_item: Retained<NSObject>,
    menu: Retained<AnyObject>,
}

thread_local! {
    static DETACHED: RefCell<Option<Detached>> = const { RefCell::new(None) };
}

/// Take the context menu off the status item (see the module docs).
pub fn detach_menu(tray: &tauri::tray::TrayIcon) {
    // The closure runs on the main thread (Tauri hops there if needed) and
    // must return a `Send` value, so the work and the stash happen inside it.
    let result = tray.with_inner_tray_icon(|inner| {
        let Some(item) = inner.ns_status_item().map(Retained::into_super) else {
            return "no NSStatusItem, menu left attached";
        };
        // SAFETY: `menu` / `setMenu:` are NSStatusItem's documented property
        // accessors; `menu` returns +0, which `retain` balances.
        unsafe {
            let menu: *mut AnyObject = msg_send![&*item, menu];
            let Some(menu) = Retained::retain(menu) else {
                return "status item has no menu to detach";
            };
            let _: () = msg_send![&*item, setMenu: std::ptr::null_mut::<AnyObject>()];
            DETACHED.with(|d| *d.borrow_mut() = Some(Detached { status_item: item, menu }));
        }
        "menu detached from status item (left click now reaches the app)"
    });
    match result {
        Ok(msg) => crate::log(&format!("tray: {}", msg)),
        Err(e) => crate::log(&format!("tray: could not reach the status item: {}", e)),
    }
}

/// Open the context menu under the icon, as AppKit would for an attached menu.
pub fn show_menu() {
    // Clone out of the RefCell first: performClick runs a nested event loop
    // that could re-enter this module.
    let Some((item, menu)) =
        DETACHED.with(|d| d.borrow().as_ref().map(|d| (d.status_item.clone(), d.menu.clone())))
    else {
        return;
    };
    // SAFETY: documented NSStatusItem / NSButton API, main thread only.
    unsafe {
        let button: *mut AnyObject = msg_send![&*item, button];
        if button.is_null() {
            return;
        }
        let _: () = msg_send![&*item, setMenu: &*menu];
        let _: () = msg_send![button, performClick: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![&*item, setMenu: std::ptr::null_mut::<AnyObject>()];
    }
}
