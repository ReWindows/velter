//! Global overlay hotkeys.

use eframe::egui::Rect as EguiRect;
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use std::num::NonZeroIsize;
use windows_sys::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

#[derive(Clone, Copy)]
pub struct Hwnd(NonZeroIsize);

impl Hwnd {
    pub fn from_handle(h: &impl HasWindowHandle) -> Option<Self> {
        match h.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(w) => Some(Self(w.hwnd)),
            _ => None,
        }
    }

    /// Limit the actual native window shape to visible panels. Pixels outside
    /// these rectangles are not part of Velter, so other processes receive clicks.
    pub fn set_panel_region(self, rects: &[EguiRect], pixels_per_point: f32) {
        unsafe {
            let combined = CreateRectRgn(0, 0, 0, 0);
            if combined.is_null() {
                return;
            }
            for rect in rects {
                let r = rect.expand(12.0);
                let part = CreateRectRgn(
                    (r.min.x * pixels_per_point).floor() as i32,
                    (r.min.y * pixels_per_point).floor() as i32,
                    (r.max.x * pixels_per_point).ceil() as i32,
                    (r.max.y * pixels_per_point).ceil() as i32,
                );
                if !part.is_null() {
                    CombineRgn(combined, combined, part, RGN_OR);
                    DeleteObject(part as _);
                }
            }
            // Windows owns `combined` after a successful call.
            if SetWindowRgn(self.0.get() as _, combined, 1) == 0 {
                DeleteObject(combined as _);
            }
        }
    }

    pub fn clear_panel_region(self) {
        unsafe {
            SetWindowRgn(self.0.get() as _, std::ptr::null_mut(), 1);
        }
    }
}

impl HasWindowHandle for Hwnd {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe {
            WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0)))
        })
    }
}

/// Edge-triggered global hotkeys, polled each frame so they work while
/// the overlay is click-through and the game has focus.
pub struct Hotkeys {
    down: [bool; 256],
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { down: [false; 256] }
    }
}

impl Hotkeys {
    pub fn pressed(&mut self, vk: u16) -> bool {
        let now = unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0;
        let was = std::mem::replace(&mut self.down[vk as usize], now);
        now && !was
    }
}
