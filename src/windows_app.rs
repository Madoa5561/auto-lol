#![allow(unsafe_op_in_unsafe_fn)]

use crate::config::{Settings, format_candidates, parse_candidates};
use crate::worker::{self, SharedState};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::BST_CHECKED;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetCursorPos, GetMessageW, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HMENU,
    IDC_ARROW, IDI_APPLICATION, LoadCursorW, LoadIconW, MB_ICONERROR, MB_OK, MF_STRING, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_RESTORE, SendMessageW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

const WINDOW_CLASS: &str = "LanePilotSettingsWindow";
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const TIMER_ID: usize = 1;

const ID_AUTO_ACCEPT: i32 = 1001;
const ID_AUTO_PICK: i32 = 1002;
const ID_AUTO_LOCK: i32 = 1003;
const ID_TOP: i32 = 1010;
const ID_JUNGLE: i32 = 1011;
const ID_MIDDLE: i32 = 1012;
const ID_BOTTOM: i32 = 1013;
const ID_UTILITY: i32 = 1014;
const ID_SAVE: i32 = 1020;
const ID_HIDE: i32 = 1021;
const ID_TRAY_OPEN: u32 = 2001;
const ID_TRAY_EXIT: u32 = 2002;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;

struct WindowState {
    shared: Arc<SharedState>,
    controls: Controls,
    tray_added: bool,
}

#[derive(Default)]
struct Controls {
    auto_accept: HWND,
    auto_pick: HWND,
    auto_lock: HWND,
    top: HWND,
    jungle: HWND,
    middle: HWND,
    bottom: HWND,
    utility: HWND,
    status: HWND,
}

pub fn run() -> Result<(), String> {
    let (settings, existed) = Settings::load();
    let shared = Arc::new(SharedState::new(settings.clone()));
    let worker = worker::spawn(shared.clone());

    let result = unsafe { run_message_loop(shared, settings, existed) };
    worker.join().ok();
    result
}

pub fn show_fatal_error(error: &str) {
    unsafe {
        let title = wide("LanePilot");
        let message = wide(error);
        MessageBoxW(
            null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

unsafe fn run_message_loop(
    shared: Arc<SharedState>,
    settings: Settings,
    settings_existed: bool,
) -> Result<(), String> {
    let instance = GetModuleHandleW(null());
    if instance.is_null() {
        shared.stop.store(true, Ordering::Relaxed);
        return Err("アプリケーション情報を取得できません".into());
    }

    let class_name = wide(WINDOW_CLASS);
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (windows_sys::Win32::Graphics::Gdi::COLOR_WINDOW + 1) as _,
        lpszClassName: class_name.as_ptr(),
        ..zeroed()
    };
    if RegisterClassW(&window_class) == 0 {
        shared.stop.store(true, Ordering::Relaxed);
        return Err("設定ウィンドウを登録できません".into());
    }

    let title = wide("LanePilot");
    let state = Box::new(WindowState {
        shared: shared.clone(),
        controls: Controls::default(),
        tray_added: false,
    });
    let state_pointer = Box::into_raw(state);
    let window = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        660,
        500,
        null_mut(),
        null_mut(),
        instance,
        state_pointer as *mut _,
    );
    if window.is_null() {
        drop(Box::from_raw(state_pointer));
        shared.stop.store(true, Ordering::Relaxed);
        return Err("設定ウィンドウを作成できません".into());
    }

    populate_controls(window, &settings);
    if !settings_existed {
        ShowWindow(window, SW_RESTORE);
    }

    let mut message: MSG = zeroed();
    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    shared.stop.store(true, Ordering::Relaxed);
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        let state = &mut *(create.lpCreateParams as *mut WindowState);
        add_tray_icon(window);
        state.tray_added = true;
        SetTimer(window, TIMER_ID, 500, None);
    }

    let state_pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut WindowState;
    if state_pointer.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let state = &mut *state_pointer;

    match message {
        WM_COMMAND => {
            let command = (wparam & 0xffff) as i32;
            match command {
                ID_SAVE => save_settings(state),
                ID_HIDE => {
                    ShowWindow(window, SW_HIDE);
                }
                _ => {}
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID
                && let Ok(status) = state.shared.status.lock()
            {
                set_text(state.controls.status, &status);
            }
            0
        }
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONDBLCLK => {
                    ShowWindow(window, SW_RESTORE);
                }
                WM_RBUTTONUP => {
                    show_tray_menu(window);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            ShowWindow(window, SW_HIDE);
            0
        }
        WM_DESTROY => {
            state.shared.stop.store(true, Ordering::Relaxed);
            if state.tray_added {
                delete_tray_icon(window);
            }
            let _ = Box::from_raw(state_pointer);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn populate_controls(window: HWND, settings: &Settings) {
    let state = &mut *(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut WindowState);
    create_label(
        window,
        "ロール別の候補を左から優先順に、カンマ区切りで入力してください。",
        24,
        18,
        590,
        24,
    );
    create_label(
        window,
        "例: Ahri, Lux, Orianna　（BAN・選択済みなら次候補へ移行）",
        24,
        43,
        590,
        24,
    );

    state.controls.top =
        create_role_row(window, "TOP", ID_TOP, 78, &format_candidates(&settings.top));
    state.controls.jungle = create_role_row(
        window,
        "JUNGLE",
        ID_JUNGLE,
        116,
        &format_candidates(&settings.jungle),
    );
    state.controls.middle = create_role_row(
        window,
        "MID",
        ID_MIDDLE,
        154,
        &format_candidates(&settings.middle),
    );
    state.controls.bottom = create_role_row(
        window,
        "ADC",
        ID_BOTTOM,
        192,
        &format_candidates(&settings.bottom),
    );
    state.controls.utility = create_role_row(
        window,
        "SUPPORT",
        ID_UTILITY,
        230,
        &format_candidates(&settings.utility),
    );

    state.controls.auto_accept = create_checkbox(
        window,
        "レディーチェックを自動承認",
        ID_AUTO_ACCEPT,
        24,
        282,
        250,
    );
    state.controls.auto_pick = create_checkbox(
        window,
        "自分の番に候補を自動ホバー",
        ID_AUTO_PICK,
        24,
        316,
        260,
    );
    state.controls.auto_lock = create_checkbox(
        window,
        "自動ロックイン（Riotポリシーを確認して使用）",
        ID_AUTO_LOCK,
        310,
        316,
        310,
    );
    set_checked(state.controls.auto_accept, settings.auto_accept);
    set_checked(state.controls.auto_pick, settings.auto_pick);
    set_checked(state.controls.auto_lock, settings.auto_lock);

    create_button(window, "保存", ID_SAVE, 24, 365, 110);
    create_button(window, "閉じて常駐", ID_HIDE, 146, 365, 130);
    state.controls.status = create_label(
        window,
        "Leagueクライアントを確認しています…",
        24,
        420,
        590,
        24,
    );
}

unsafe fn save_settings(state: &mut WindowState) {
    let settings = Settings {
        auto_accept: is_checked(state.controls.auto_accept),
        auto_pick: is_checked(state.controls.auto_pick),
        auto_lock: is_checked(state.controls.auto_lock),
        top: parse_candidates(&get_text(state.controls.top)),
        jungle: parse_candidates(&get_text(state.controls.jungle)),
        middle: parse_candidates(&get_text(state.controls.middle)),
        bottom: parse_candidates(&get_text(state.controls.bottom)),
        utility: parse_candidates(&get_text(state.controls.utility)),
    };
    match settings.save() {
        Ok(()) => {
            if let Ok(mut current) = state.shared.settings.lock() {
                *current = settings;
            }
            state.shared.set_status("設定を保存しました");
        }
        Err(error) => state
            .shared
            .set_status(format!("設定の保存に失敗: {error}")),
    }
}

unsafe fn add_tray_icon(window: HWND) {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
    copy_wide(&mut data.szTip, "LanePilot");
    Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn delete_tray_icon(window: HWND) {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &data);
}

unsafe fn show_tray_menu(window: HWND) {
    let menu = CreatePopupMenu();
    AppendMenuW(
        menu,
        MF_STRING,
        ID_TRAY_OPEN as usize,
        wide("設定を開く").as_ptr(),
    );
    AppendMenuW(
        menu,
        MF_STRING,
        ID_TRAY_EXIT as usize,
        wide("終了").as_ptr(),
    );
    let mut point = POINT { x: 0, y: 0 };
    GetCursorPos(&mut point);
    SetForegroundWindow(window);
    let command = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
        point.x,
        point.y,
        0,
        window,
        null(),
    );
    DestroyMenu(menu);
    match command as u32 {
        ID_TRAY_OPEN => {
            ShowWindow(window, SW_RESTORE);
        }
        ID_TRAY_EXIT => {
            DestroyWindow(window);
        }
        _ => {}
    }
}

unsafe fn create_role_row(parent: HWND, label: &str, id: i32, y: i32, value: &str) -> HWND {
    create_label(parent, label, 24, y + 4, 80, 24);
    create_edit(parent, value, id, 112, y, 500, 28)
}

unsafe fn create_label(parent: HWND, text: &str, x: i32, y: i32, width: i32, height: i32) -> HWND {
    create_control(
        "STATIC",
        text,
        0,
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        parent,
        0,
    )
}

unsafe fn create_edit(
    parent: HWND,
    text: &str,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> HWND {
    create_control(
        "EDIT",
        text,
        WS_EX_CLIENTEDGE,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER,
        x,
        y,
        width,
        height,
        parent,
        id,
    )
}

unsafe fn create_checkbox(parent: HWND, text: &str, id: i32, x: i32, y: i32, width: i32) -> HWND {
    create_control(
        "BUTTON",
        text,
        0,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | 0x00000003,
        x,
        y,
        width,
        26,
        parent,
        id,
    )
}

unsafe fn create_button(parent: HWND, text: &str, id: i32, x: i32, y: i32, width: i32) -> HWND {
    create_control(
        "BUTTON",
        text,
        0,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        x,
        y,
        width,
        32,
        parent,
        id,
    )
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_control(
    class_name: &str,
    text: &str,
    extended_style: u32,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: HWND,
    id: i32,
) -> HWND {
    let class_name = wide(class_name);
    let text = wide(text);
    let instance: HINSTANCE = GetModuleHandleW(null());
    let control = CreateWindowExW(
        extended_style,
        class_name.as_ptr(),
        text.as_ptr(),
        style,
        x,
        y,
        width,
        height,
        parent,
        id as HMENU,
        instance,
        null_mut(),
    );
    let font = GetStockObject(DEFAULT_GUI_FONT);
    SendMessageW(control, WM_SETFONT, font as usize, 1);
    control
}

unsafe fn get_text(control: HWND) -> String {
    let length = GetWindowTextLengthW(control);
    let mut buffer = vec![0u16; length as usize + 1];
    GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..length as usize])
}

unsafe fn set_text(control: HWND, value: &str) {
    let value = wide(value);
    SetWindowTextW(control, value.as_ptr());
}

unsafe fn set_checked(control: HWND, checked: bool) {
    SendMessageW(
        control,
        BM_SETCHECK,
        if checked { BST_CHECKED as usize } else { 0 },
        0,
    );
}

unsafe fn is_checked(control: HWND) -> bool {
    SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    for (destination, source) in target
        .iter_mut()
        .zip(value.encode_utf16().chain(std::iter::once(0)))
    {
        *destination = source;
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
