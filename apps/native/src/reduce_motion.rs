//! Mirrors macOS "Reduce motion" into GPUI. `App::reduce_motion` defaults to
//! false and GPUI never reads the system setting itself, so the host reads
//! `NSWorkspace.accessibilityDisplayShouldReduceMotion` at launch and again
//! whenever AppKit posts `NSWorkspaceAccessibilityDisplayOptionsDidChange`.
//!
//! No Objective-C crate is a direct dependency, so the bridge uses the
//! Objective-C runtime's C entry points. Every call is confined to `macos`.
use gpui::App;
use std::time::Duration;

/// The sidebar spring settles in ~0.3 s; both ends stay rendered a little
/// longer so the clip never shows a half-laid-out column.
const SIDEBAR_SETTLE: Duration = Duration::from_millis(450);

/// How long the sidebar keeps both ends rendered after a toggle. With reduced
/// motion the width snaps, so there is nothing to settle.
pub fn sidebar_settle(reduce_motion: bool) -> Option<Duration> {
    (!reduce_motion).then_some(SIDEBAR_SETTLE)
}

/// Applies the current system value and follows later changes for the
/// lifetime of the app. A missing or failed bridge leaves GPUI's default.
pub fn install(cx: &mut App) {
    if let Some(reduce) = platform::current() {
        cx.set_reduce_motion(reduce);
    }
    let Some(changes) = platform::observe() else {
        return;
    };
    cx.spawn(async move |cx| {
        while changes.recv().await.is_ok() {
            // Coalesce a burst of notifications into one read.
            while changes.try_recv().is_ok() {}
            if let Some(reduce) = platform::current() {
                cx.update(|cx| cx.set_reduce_motion(reduce));
            }
        }
    })
    .detach();
}

#[cfg(target_os = "macos")]
mod platform {
    use std::{
        ffi::{c_char, c_void},
        sync::OnceLock,
    };

    type Id = *mut c_void;
    type Sel = *mut c_void;
    type Class = *mut c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Class;
        fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
        fn objc_registerClassPair(class: Class);
        fn class_addMethod(
            class: Class,
            name: Sel,
            imp: unsafe extern "C" fn(),
            types: *const c_char,
        ) -> i8;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {
        static NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification: Id;
    }

    /// Wakes the GPUI task; AppKit delivers the notification on the main thread.
    static CHANGES: OnceLock<async_channel::Sender<()>> = OnceLock::new();

    unsafe fn send_id(receiver: Id, selector: &std::ffi::CStr) -> Id {
        // SAFETY: objc_msgSend must be called through the exact method type;
        // every selector sent here takes no arguments and returns an object.
        unsafe {
            let send = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel) -> Id>(
                objc_msgSend,
            );
            send(receiver, sel_registerName(selector.as_ptr()))
        }
    }

    unsafe fn shared_workspace() -> Option<Id> {
        unsafe {
            let class = objc_getClass(c"NSWorkspace".as_ptr());
            if class.is_null() {
                return None;
            }
            let workspace = send_id(class, c"sharedWorkspace");
            (!workspace.is_null()).then_some(workspace)
        }
    }

    pub fn current() -> Option<bool> {
        // SAFETY: `accessibilityDisplayShouldReduceMotion` (macOS 10.12+) takes
        // no arguments and returns BOOL; only the low byte is meaningful on
        // both x86_64 (signed char) and arm64 (bool).
        unsafe {
            let workspace = shared_workspace()?;
            let get = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel) -> i8>(
                objc_msgSend,
            );
            Some(
                get(
                    workspace,
                    sel_registerName(c"accessibilityDisplayShouldReduceMotion".as_ptr()),
                ) != 0,
            )
        }
    }

    unsafe extern "C" fn display_options_changed(_this: Id, _cmd: Sel, _note: Id) {
        if let Some(changes) = CHANGES.get() {
            // A full channel already holds a pending wake.
            let _ = changes.try_send(());
        }
    }

    /// Registers one observer for the app lifetime. Returns `None` if it is
    /// already installed or the runtime refuses the class.
    pub fn observe() -> Option<async_channel::Receiver<()>> {
        let (send, receive) = async_channel::bounded(1);
        CHANGES.set(send).ok()?;
        // SAFETY: the class is created once (guarded by CHANGES), derives from
        // NSObject and adds one `v@:@` method matching `display_options_changed`.
        // The observer instance is intentionally leaked: it must outlive the
        // notification center registration, which lasts until process exit.
        unsafe {
            let superclass = objc_getClass(c"NSObject".as_ptr());
            if superclass.is_null() {
                return None;
            }
            let name = c"DbunkReduceMotionObserver";
            let mut class = objc_allocateClassPair(superclass, name.as_ptr(), 0);
            if class.is_null() {
                class = objc_getClass(name.as_ptr());
            } else {
                let imp: unsafe extern "C" fn(Id, Sel, Id) = display_options_changed;
                class_addMethod(
                    class,
                    sel_registerName(c"displayOptionsChanged:".as_ptr()),
                    std::mem::transmute::<unsafe extern "C" fn(Id, Sel, Id), unsafe extern "C" fn()>(
                        imp,
                    ),
                    c"v@:@".as_ptr(),
                );
                objc_registerClassPair(class);
            }
            if class.is_null() {
                return None;
            }
            let observer = send_id(send_id(class, c"alloc"), c"init");
            let workspace = shared_workspace()?;
            let center = send_id(workspace, c"notificationCenter");
            if observer.is_null() || center.is_null() {
                return None;
            }
            let add = std::mem::transmute::<
                unsafe extern "C" fn(),
                unsafe extern "C" fn(Id, Sel, Id, Sel, Id, Id),
            >(objc_msgSend);
            add(
                center,
                sel_registerName(c"addObserver:selector:name:object:".as_ptr()),
                observer,
                sel_registerName(c"displayOptionsChanged:".as_ptr()),
                NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
                std::ptr::null_mut(),
            );
        }
        Some(receive)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn current() -> Option<bool> {
        None
    }
    pub fn observe() -> Option<async_channel::Receiver<()>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_skips_the_sidebar_settle_window() {
        assert_eq!(sidebar_settle(true), None);
        let settle = sidebar_settle(false).expect("motion keeps both ends rendered");
        // Must outlast the ~0.3 s spring documented in DESIGN.md §5.
        assert!(settle >= Duration::from_millis(300));
    }
}
