#![allow(unsafe_op_in_unsafe_fn)]

use crate::app_icon::load_app_icon;
use crate::champion_picker::{self, Role, SelectionKind};
use crate::config::Settings;
use crate::worker::{self, SharedState};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject,
    DrawTextW, EndPaint, FW_BOLD, FW_NORMAL, FillRect, HBRUSH, HFONT, OPAQUE, PAINTSTRUCT,
    RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{BST_CHECKED, DRAWITEMSTRUCT, ODS_SELECTED, SetWindowTheme};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BS_AUTOCHECKBOX, BS_OWNERDRAW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, LoadCursorW, MB_ICONERROR, MB_OK,
    MF_STRING, MSG, MessageBoxW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_RESTORE,
    SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu, TranslateMessage, WM_APP,
    WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM,
    WM_LBUTTONDBLCLK, WM_PAINT, WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_CAPTION,
    WS_CHILD, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const WINDOW_CLASS: &str = "LanePilotModernSettingsWindow";
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;
const TIMER_ID: usize = 1;

const ID_AUTO_ACCEPT: i32 = 1001;
const ID_AUTO_PICK: i32 = 1002;
const ID_AUTO_BAN: i32 = 1004;
const ID_PICK_BASE: i32 = 1100;
const ID_BAN_BASE: i32 = 1200;
const ID_SAVE: i32 = 1300;
const ID_HIDE: i32 = 1301;
const ID_TRAY_OPEN: u32 = 2001;
const ID_TRAY_EXIT: u32 = 2002;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;

const COLOR_BG: u32 = rgb(13, 16, 23);
const COLOR_SURFACE: u32 = rgb(25, 30, 40);
const COLOR_SURFACE_HOVER: u32 = rgb(37, 44, 58);
const COLOR_TEXT: u32 = rgb(239, 242, 248);
const COLOR_PICK: u32 = rgb(31, 197, 168);
const COLOR_PICK_PRESSED: u32 = rgb(24, 155, 134);
const COLOR_BAN: u32 = rgb(233, 84, 111);
const COLOR_BAN_PRESSED: u32 = rgb(186, 62, 86);

struct WindowState {
    shared: Arc<SharedState>,
    controls: Controls,
    tray_added: bool,
    background_brush: HBRUSH,
    surface_brush: HBRUSH,
    font: HFONT,
    title_font: HFONT,
}

struct Controls {
    auto_accept: HWND,
    auto_pick: HWND,
    auto_ban: HWND,
    role_labels: [HWND; 5],
    pick_summaries: [HWND; 5],
    ban_summaries: [HWND; 5],
    status: HWND,
    icon_status: HWND,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            auto_accept: null_mut(),
            auto_pick: null_mut(),
            auto_ban: null_mut(),
            role_labels: [null_mut(); 5],
            pick_summaries: [null_mut(); 5],
            ban_summaries: [null_mut(); 5],
            status: null_mut(),
            icon_status: null_mut(),
        }
    }
}

pub fn run() -> Result<(), String> {
    let (settings, settings_existed) = Settings::load();
    if !settings_existed {
        settings
            .save()
            .map_err(|error| format!("初期設定を保存できません: {error}"))?;
    }
    let shared = Arc::new(SharedState::new(settings.clone()));
    let monitor = worker::spawn(shared.clone());
    let icon_cache = worker::spawn_icon_cache(shared.clone());

    let result = unsafe { run_message_loop(shared.clone(), settings) };
    shared.stop.store(true, Ordering::Relaxed);
    monitor.join().ok();
    icon_cache.join().ok();
    result
}

pub fn show_fatal_error(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(error).as_ptr(),
            wide("LanePilot").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

unsafe fn run_message_loop(shared: Arc<SharedState>, settings: Settings) -> Result<(), String> {
    let instance = GetModuleHandleW(null());
    if instance.is_null() {
        return Err("アプリケーション情報を取得できません".into());
    }
    champion_picker::register(instance)?;

    let class_name = wide(WINDOW_CLASS);
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hIcon: load_app_icon(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: CreateSolidBrush(COLOR_BG),
        lpszClassName: class_name.as_ptr(),
        ..zeroed()
    };
    if RegisterClassW(&window_class) == 0 {
        return Err("設定ウィンドウを登録できません".into());
    }

    let state = Box::new(WindowState {
        shared,
        controls: Controls::default(),
        tray_added: false,
        background_brush: CreateSolidBrush(COLOR_BG),
        surface_brush: CreateSolidBrush(COLOR_SURFACE),
        font: create_font(16, FW_NORMAL as i32),
        title_font: create_font(28, FW_BOLD as i32),
    });
    let state_pointer = Box::into_raw(state);
    let window = CreateWindowExW(
        0,
        class_name.as_ptr(),
        wide("LanePilot").as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        940,
        735,
        null_mut(),
        null_mut(),
        instance,
        state_pointer as *mut _,
    );
    if window.is_null() {
        drop(Box::from_raw(state_pointer));
        return Err("設定ウィンドウを作成できません".into());
    }

    let dark_mode = 1i32;
    DwmSetWindowAttribute(
        window,
        20,
        &dark_mode as *const i32 as *const _,
        size_of::<i32>() as u32,
    );
    populate_controls(window, &settings);
    ShowWindow(window, SW_RESTORE);

    let mut message: MSG = zeroed();
    while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
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
                ID_AUTO_ACCEPT | ID_AUTO_PICK | ID_AUTO_BAN => save_settings(state),
                ID_HIDE => {
                    ShowWindow(window, SW_HIDE);
                }
                id if (ID_PICK_BASE..ID_PICK_BASE + 5).contains(&id) => {
                    champion_picker::open(
                        window,
                        state.shared.clone(),
                        role_from_index((id - ID_PICK_BASE) as usize),
                        SelectionKind::Pick,
                    );
                }
                id if (ID_BAN_BASE..ID_BAN_BASE + 5).contains(&id) => {
                    champion_picker::open(
                        window,
                        state.shared.clone(),
                        role_from_index((id - ID_BAN_BASE) as usize),
                        SelectionKind::Ban,
                    );
                }
                _ => {}
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID {
                if let Ok(status) = state.shared.status.lock() {
                    set_text(state.controls.status, &status);
                }
                let ready = state.shared.icons_ready.load(Ordering::Relaxed);
                let total = state
                    .shared
                    .champions
                    .lock()
                    .map(|champions| champions.len())
                    .unwrap_or_default();
                let icon_text = if total == 0 {
                    "チャンピオンを読み込み中…".to_owned()
                } else {
                    format!("アイコン {ready}/{total}")
                };
                set_text(state.controls.icon_status, &icon_text);
                refresh_summaries(state);
            }
            0
        }
        WM_PAINT => {
            paint_background(window);
            0
        }
        WM_DRAWITEM => {
            draw_button(&*(lparam as *const DRAWITEMSTRUCT));
            1
        }
        WM_CTLCOLORSTATIC => {
            let device = wparam as *mut _;
            let control = lparam as HWND;
            let on_surface = state.controls.role_labels.contains(&control)
                || state.controls.pick_summaries.contains(&control)
                || state.controls.ban_summaries.contains(&control);
            let (background, brush) = if on_surface {
                (COLOR_SURFACE, state.surface_brush)
            } else {
                (COLOR_BG, state.background_brush)
            };
            SetTextColor(device, COLOR_TEXT);
            SetBkColor(device, background);
            SetBkMode(device, OPAQUE as i32);
            brush as isize
        }
        WM_CTLCOLORBTN => {
            let device = wparam as *mut _;
            SetTextColor(device, COLOR_TEXT);
            SetBkMode(device, TRANSPARENT as i32);
            state.background_brush as isize
        }
        WM_TRAY => {
            match lparam as u32 {
                WM_LBUTTONDBLCLK => {
                    ShowWindow(window, SW_RESTORE);
                }
                WM_RBUTTONUP => show_tray_menu(window),
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
            DeleteObject(state.background_brush as _);
            DeleteObject(state.surface_brush as _);
            DeleteObject(state.font as _);
            DeleteObject(state.title_font as _);
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
    create_label(window, "LanePilot", 28, 20, 300, 42, state.title_font);
    create_label(
        window,
        "軽量なLeagueクライアント自動化",
        30,
        61,
        360,
        24,
        state.font,
    );
    state.controls.icon_status = create_label(
        window,
        "チャンピオンを読み込み中…",
        700,
        30,
        200,
        24,
        state.font,
    );

    state.controls.auto_accept =
        create_checkbox(window, "自動承認", ID_AUTO_ACCEPT, 30, 104, 135, state.font);
    state.controls.auto_pick = create_checkbox(
        window,
        "自動PICK・確定",
        ID_AUTO_PICK,
        178,
        104,
        175,
        state.font,
    );
    state.controls.auto_ban =
        create_checkbox(window, "自動BAN", ID_AUTO_BAN, 370, 104, 140, state.font);
    set_checked(state.controls.auto_accept, settings.auto_accept);
    set_checked(state.controls.auto_pick, settings.auto_pick);
    set_checked(state.controls.auto_ban, settings.auto_ban);

    create_label(window, "ROLE PRESETS", 30, 151, 200, 24, state.font);
    for index in 0..5 {
        let y = 184 + index as i32 * 88;
        state.controls.role_labels[index] =
            create_label(window, role_label(index), 44, y + 14, 90, 24, state.font);
        state.controls.pick_summaries[index] =
            create_label(window, "", 138, y + 8, 530, 24, state.font);
        state.controls.ban_summaries[index] =
            create_label(window, "", 138, y + 40, 530, 24, state.font);
        create_button(
            window,
            "PICK",
            ID_PICK_BASE + index as i32,
            694,
            y + 20,
            88,
            38,
            state.font,
        );
        create_button(
            window,
            "BAN",
            ID_BAN_BASE + index as i32,
            796,
            y + 20,
            88,
            38,
            state.font,
        );
    }

    create_button(window, "設定を保存", ID_SAVE, 30, 638, 150, 40, state.font);
    create_button(window, "閉じて常駐", ID_HIDE, 194, 638, 150, 40, state.font);
    state.controls.status = create_label(
        window,
        "Leagueを確認しています…",
        374,
        647,
        510,
        26,
        state.font,
    );
    refresh_summaries(state);
}

unsafe fn refresh_summaries(state: &WindowState) {
    let Ok(settings) = state.shared.settings.lock() else {
        return;
    };
    let picks = [
        &settings.top,
        &settings.jungle,
        &settings.middle,
        &settings.bottom,
        &settings.utility,
    ];
    let bans = [
        &settings.ban_top,
        &settings.ban_jungle,
        &settings.ban_middle,
        &settings.ban_bottom,
        &settings.ban_utility,
    ];
    for index in 0..5 {
        set_text(
            state.controls.pick_summaries[index],
            &summary("PICK", picks[index]),
        );
        set_text(
            state.controls.ban_summaries[index],
            &summary("BAN", bans[index]),
        );
    }
}

fn summary(label: &str, candidates: &[String]) -> String {
    if candidates.is_empty() {
        format!("{label}  —  未設定")
    } else {
        format!("{label}  {}", candidates.join("  ›  "))
    }
}

unsafe fn save_settings(state: &WindowState) {
    let result = state.shared.settings.lock().map(|mut settings| {
        settings.auto_accept = is_checked(state.controls.auto_accept);
        settings.auto_pick = is_checked(state.controls.auto_pick);
        settings.auto_ban = is_checked(state.controls.auto_ban);
        settings.save()
    });
    match result {
        Ok(Ok(())) => state.shared.set_status("設定を保存しました"),
        Ok(Err(error)) => state
            .shared
            .set_status(format!("設定の保存に失敗: {error}")),
        Err(_) => state.shared.set_status("設定を更新できません"),
    }
}

unsafe fn paint_background(window: HWND) {
    let mut paint: PAINTSTRUCT = zeroed();
    let device = BeginPaint(window, &mut paint);
    let background = CreateSolidBrush(COLOR_BG);
    FillRect(device, &paint.rcPaint, background);
    DeleteObject(background as _);

    let card_brush = CreateSolidBrush(COLOR_SURFACE);
    let previous_brush = SelectObject(device, card_brush as _);
    for index in 0..5 {
        let top = 184 + index * 88;
        RoundRect(device, 28, top, 902, top + 76, 16, 16);
    }
    SelectObject(device, previous_brush);
    DeleteObject(card_brush as _);
    EndPaint(window, &paint);
}

unsafe fn draw_button(item: &DRAWITEMSTRUCT) {
    let pressed = item.itemState & ODS_SELECTED != 0;
    let id = item.CtlID as i32;
    let (normal, active) = if (ID_BAN_BASE..ID_BAN_BASE + 5).contains(&id) {
        (COLOR_BAN, COLOR_BAN_PRESSED)
    } else if (ID_PICK_BASE..ID_PICK_BASE + 5).contains(&id) || id == ID_SAVE {
        (COLOR_PICK, COLOR_PICK_PRESSED)
    } else {
        (COLOR_SURFACE_HOVER, COLOR_SURFACE)
    };
    let brush = CreateSolidBrush(if pressed { active } else { normal });
    let previous = SelectObject(item.hDC, brush as _);
    RoundRect(
        item.hDC,
        item.rcItem.left,
        item.rcItem.top,
        item.rcItem.right,
        item.rcItem.bottom,
        12,
        12,
    );
    SelectObject(item.hDC, previous);
    DeleteObject(brush as _);

    let length = GetWindowTextLengthW(item.hwndItem);
    let mut text = vec![0u16; length as usize + 1];
    GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(item.hDC, if id == ID_HIDE { COLOR_TEXT } else { COLOR_BG });
    let mut rect = item.rcItem;
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        length,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

unsafe fn add_tray_icon(window: HWND) {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = window;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = load_app_icon();
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

unsafe fn create_label(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: HFONT,
) -> HWND {
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
        font,
    )
}

unsafe fn create_checkbox(
    parent: HWND,
    text: &str,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    font: HFONT,
) -> HWND {
    let control = create_control(
        "BUTTON",
        text,
        0,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        x,
        y,
        width,
        30,
        parent,
        id,
        font,
    );
    SetWindowTheme(control, wide("DarkMode_Explorer").as_ptr(), null());
    control
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_button(
    parent: HWND,
    text: &str,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: HFONT,
) -> HWND {
    create_control(
        "BUTTON",
        text,
        0,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        x,
        y,
        width,
        height,
        parent,
        id,
        font,
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
    font: HFONT,
) -> HWND {
    let control = CreateWindowExW(
        extended_style,
        wide(class_name).as_ptr(),
        wide(text).as_ptr(),
        style,
        x,
        y,
        width,
        height,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null_mut(),
    );
    SendMessageW(control, WM_SETFONT, font as usize, 1);
    control
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

unsafe fn set_text(control: HWND, value: &str) {
    let length = GetWindowTextLengthW(control);
    let mut buffer = vec![0u16; length as usize + 1];
    GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
    if String::from_utf16_lossy(&buffer[..length as usize]) == value {
        return;
    }

    SetWindowTextW(control, wide(value).as_ptr());
}

unsafe fn create_font(size: i32, weight: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        1,
        0,
        0,
        5,
        0,
        wide("Segoe UI Variable Text").as_ptr(),
    )
}

fn role_from_index(index: usize) -> Role {
    match index {
        0 => Role::Top,
        1 => Role::Jungle,
        2 => Role::Middle,
        3 => Role::Bottom,
        _ => Role::Utility,
    }
}

fn role_label(index: usize) -> &'static str {
    match index {
        0 => "TOP",
        1 => "JUNGLE",
        2 => "MID",
        3 => "ADC",
        _ => "SUPPORT",
    }
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    for (destination, source) in target
        .iter_mut()
        .zip(value.encode_utf16().chain(std::iter::once(0)))
    {
        *destination = source;
    }
}

const fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    red | (green << 8) | (blue << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
