#![allow(unsafe_op_in_unsafe_fn)]

use std::ptr::{null, null_mut};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{HICON, IDI_APPLICATION, LoadIconW};

const APP_ICON_ID: usize = 101;

pub unsafe fn load_app_icon() -> HICON {
    let icon = LoadIconW(GetModuleHandleW(null()), APP_ICON_ID as *const u16);
    if icon.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}
