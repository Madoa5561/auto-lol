#![allow(unsafe_op_in_unsafe_fn)]

use crate::app_icon::load_app_icon;
use crate::config::{Settings, champion_icon_path};
use crate::worker::SharedState;
use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FW_NORMAL, HBITMAP, HBRUSH, HFONT, OPAQUE, RoundRect, SelectObject, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::Graphics::GdiPlus::{
    GdipCreateBitmapFromFile, GdipCreateHBITMAPFromBitmap, GdipDisposeImage, GdipGetImageThumbnail,
    GdiplusStartup, GdiplusStartupInput, GpBitmap, GpImage,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, ICC_LISTVIEW_CLASSES, ILC_COLOR32, INITCOMMONCONTROLSEX, ImageList_Add,
    ImageList_Create, ImageList_Destroy, InitCommonControlsEx, LVIF_IMAGE, LVIF_PARAM, LVIF_TEXT,
    LVITEMW, LVM_DELETEALLITEMS, LVM_GETNEXTITEM, LVM_INSERTITEMW, LVM_SETBKCOLOR,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETICONSPACING, LVM_SETIMAGELIST, LVM_SETTEXTBKCOLOR,
    LVM_SETTEXTCOLOR, LVNI_SELECTED, LVS_AUTOARRANGE, LVS_EX_DOUBLEBUFFER, LVS_ICON,
    LVS_SHOWSELALWAYS, LVS_SINGLESEL, LVSIL_NORMAL, NM_DBLCLK, NMHDR, ODS_SELECTED, SetWindowTheme,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW,
    DefWindowProcW, DestroyWindow, EN_CHANGE, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, HMENU, IDC_ARROW, LoadCursorW, RegisterClassW,
    SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM,
    WM_NOTIFY, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE,
    WS_EX_TOOLWINDOW, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const CLASS_NAME: &str = "LanePilotChampionPicker";
const ID_SEARCH: i32 = 4001;
const ID_LIST: i32 = 4002;
const ID_ADD: i32 = 4003;
const ID_UNDO: i32 = 4004;
const ID_CLEAR: i32 = 4005;
const ID_CLOSE: i32 = 4006;
const TIMER_ID: usize = 2;

const COLOR_BG: u32 = rgb(16, 19, 26);
const COLOR_SURFACE: u32 = rgb(29, 34, 45);
const COLOR_EDIT: u32 = rgb(36, 42, 54);
const COLOR_TEXT: u32 = rgb(235, 238, 245);
const COLOR_ACCENT: u32 = rgb(31, 197, 168);
const COLOR_ACCENT_PRESSED: u32 = rgb(24, 155, 134);
const COLOR_BUTTON: u32 = rgb(44, 51, 66);
const COLOR_BUTTON_PRESSED: u32 = rgb(35, 41, 54);

#[derive(Clone, Copy, Debug)]
pub enum Role {
    Top,
    Jungle,
    Middle,
    Bottom,
    Utility,
}

#[derive(Clone, Copy, Debug)]
pub enum SelectionKind {
    Pick,
    Ban,
}

struct PickerState {
    shared: Arc<SharedState>,
    role: Role,
    kind: SelectionKind,
    search: HWND,
    list: HWND,
    selected: HWND,
    progress: HWND,
    filtered: Vec<usize>,
    image_list: isize,
    last_icons_ready: usize,
    last_champion_count: usize,
    background_brush: HBRUSH,
    edit_brush: HBRUSH,
    font: HFONT,
}

pub unsafe fn register(instance: HINSTANCE) -> Result<(), String> {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES,
    };
    InitCommonControlsEx(&controls);
    ensure_gdiplus()?;

    let class_name = wide(CLASS_NAME);
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(picker_proc),
        hInstance: instance,
        hIcon: load_app_icon(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: CreateSolidBrush(COLOR_BG),
        lpszClassName: class_name.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&window_class);
    Ok(())
}

pub unsafe fn open(owner: HWND, shared: Arc<SharedState>, role: Role, kind: SelectionKind) {
    let state = Box::new(PickerState {
        shared,
        role,
        kind,
        search: null_mut(),
        list: null_mut(),
        selected: null_mut(),
        progress: null_mut(),
        filtered: Vec::new(),
        image_list: 0,
        last_icons_ready: usize::MAX,
        last_champion_count: usize::MAX,
        background_brush: CreateSolidBrush(COLOR_BG),
        edit_brush: CreateSolidBrush(COLOR_EDIT),
        font: create_font(16),
    });
    let state_pointer = Box::into_raw(state);
    let title = format!(
        "{} — {}候補",
        role_label(role),
        match kind {
            SelectionKind::Pick => "PICK",
            SelectionKind::Ban => "BAN",
        }
    );
    let window = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        wide(CLASS_NAME).as_ptr(),
        wide(&title).as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        780,
        650,
        owner,
        null_mut(),
        GetModuleHandleW(null()),
        state_pointer as *mut _,
    );
    if window.is_null() {
        drop(Box::from_raw(state_pointer));
    } else {
        let dark_mode = 1i32;
        DwmSetWindowAttribute(
            window,
            20,
            &dark_mode as *const i32 as *const _,
            size_of::<i32>() as u32,
        );
        ShowWindow(window, 5);
    }
}

unsafe extern "system" fn picker_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        let state = &mut *(create.lpCreateParams as *mut PickerState);
        create_picker_controls(window, state);
        rebuild_list(state);
        update_selected_text(state);
        SetTimer(window, TIMER_ID, 500, None);
    }

    let state_pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut PickerState;
    if state_pointer.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let state = &mut *state_pointer;

    match message {
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let notification = ((wparam >> 16) & 0xffff) as u32;
            match id {
                ID_SEARCH if notification == EN_CHANGE => rebuild_list(state),
                ID_ADD => add_selected(state),
                ID_UNDO => mutate_candidates(state, |candidates| {
                    candidates.pop();
                }),
                ID_CLEAR => mutate_candidates(state, Vec::clear),
                ID_CLOSE => {
                    DestroyWindow(window);
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            let header = &*(lparam as *const NMHDR);
            if header.hwndFrom == state.list && header.code == NM_DBLCLK {
                add_selected(state);
            }
            0
        }
        WM_TIMER => {
            if wparam == TIMER_ID {
                let ready = state.shared.icons_ready.load(Ordering::Relaxed);
                let total = state
                    .shared
                    .champions
                    .lock()
                    .map(|champions| champions.len())
                    .unwrap_or_default();
                if list_needs_rebuild(
                    total,
                    ready,
                    state.last_champion_count,
                    state.last_icons_ready,
                ) {
                    rebuild_list(state);
                }
                set_text(state.progress, &format!("アイコン {ready} / {total}"));
            }
            0
        }
        WM_CTLCOLOREDIT => {
            let device = wparam as *mut _;
            SetTextColor(device, COLOR_TEXT);
            SetBkColor(device, COLOR_EDIT);
            state.edit_brush as isize
        }
        WM_CTLCOLORSTATIC => {
            let device = wparam as *mut _;
            SetTextColor(device, COLOR_TEXT);
            SetBkColor(device, COLOR_BG);
            SetBkMode(device, OPAQUE as i32);
            state.background_brush as isize
        }
        WM_CTLCOLORBTN => {
            let device = wparam as *mut _;
            SetTextColor(device, COLOR_TEXT);
            SetBkMode(device, TRANSPARENT as i32);
            state.background_brush as isize
        }
        WM_DRAWITEM => {
            draw_button(&*(lparam as *const DRAWITEMSTRUCT));
            1
        }
        WM_CLOSE => {
            DestroyWindow(window);
            0
        }
        WM_DESTROY => {
            if state.image_list != 0 {
                ImageList_Destroy(state.image_list);
            }
            DeleteObject(state.background_brush as _);
            DeleteObject(state.edit_brush as _);
            DeleteObject(state.font as _);
            let _ = Box::from_raw(state_pointer);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn create_picker_controls(window: HWND, state: &mut PickerState) {
    create_label(
        window,
        "チャンピオンを検索して、優先順に追加",
        24,
        18,
        520,
        26,
        state.font,
    );
    state.progress = create_label(window, "アイコンを準備中…", 575, 20, 170, 24, state.font);
    state.search = create_control(
        "EDIT",
        "",
        WS_EX_CLIENTEDGE,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        24,
        54,
        720,
        34,
        window,
        ID_SEARCH,
        state.font,
    );
    state.list = create_control(
        "SysListView32",
        "",
        WS_EX_CLIENTEDGE,
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | LVS_ICON
            | LVS_SINGLESEL
            | LVS_SHOWSELALWAYS
            | LVS_AUTOARRANGE,
        24,
        102,
        720,
        400,
        window,
        ID_LIST,
        state.font,
    );
    SetWindowTheme(state.list, wide("DarkMode_Explorer").as_ptr(), null());
    SendMessageW(state.list, LVM_SETBKCOLOR, 0, COLOR_SURFACE as isize);
    SendMessageW(state.list, LVM_SETTEXTBKCOLOR, 0, COLOR_SURFACE as isize);
    SendMessageW(state.list, LVM_SETTEXTCOLOR, 0, COLOR_TEXT as isize);
    SendMessageW(
        state.list,
        LVM_SETEXTENDEDLISTVIEWSTYLE,
        LVS_EX_DOUBLEBUFFER as usize,
        LVS_EX_DOUBLEBUFFER as isize,
    );
    SendMessageW(
        state.list,
        LVM_SETICONSPACING,
        0,
        (((94u32) << 16) | 88u32) as isize,
    );
    state.selected = create_label(window, "", 24, 518, 720, 26, state.font);
    create_button(window, "候補に追加", ID_ADD, 24, 558, 150, state.font);
    create_button(window, "1つ戻す", ID_UNDO, 188, 558, 120, state.font);
    create_button(window, "すべて解除", ID_CLEAR, 322, 558, 130, state.font);
    create_button(window, "閉じる", ID_CLOSE, 624, 558, 120, state.font);
}

unsafe fn rebuild_list(state: &mut PickerState) {
    let champions = state
        .shared
        .champions
        .lock()
        .map(|champions| champions.clone())
        .unwrap_or_default();
    let search = get_text(state.search).to_lowercase();
    state.filtered = champions
        .iter()
        .enumerate()
        .filter(|(_, champion)| {
            search.is_empty()
                || champion.name.to_lowercase().contains(&search)
                || champion.alias.to_lowercase().contains(&search)
        })
        .map(|(index, _)| index)
        .collect();

    SendMessageW(state.list, LVM_DELETEALLITEMS, 0, 0);
    if state.image_list != 0 {
        SendMessageW(state.list, LVM_SETIMAGELIST, LVSIL_NORMAL as usize, 0);
        ImageList_Destroy(state.image_list);
    }
    state.image_list = ImageList_Create(56, 56, ILC_COLOR32, champions.len() as i32, 32);
    SendMessageW(
        state.list,
        LVM_SETIMAGELIST,
        LVSIL_NORMAL as usize,
        state.image_list,
    );

    let mut images = HashMap::new();
    for &champion_index in &state.filtered {
        let champion = &champions[champion_index];
        let image_index = *images.entry(champion.id).or_insert_with(|| {
            let bitmap = load_thumbnail(champion.id);
            if bitmap.is_null() {
                -1
            } else {
                let index = ImageList_Add(state.image_list, bitmap, null_mut());
                DeleteObject(bitmap as _);
                index
            }
        });
        let mut text = wide(&champion.name);
        let mut item: LVITEMW = zeroed();
        item.mask = LVIF_TEXT | LVIF_IMAGE | LVIF_PARAM;
        item.iItem = i32::MAX;
        item.pszText = text.as_mut_ptr();
        item.iImage = image_index;
        item.lParam = champion_index as isize;
        SendMessageW(
            state.list,
            LVM_INSERTITEMW,
            0,
            &item as *const LVITEMW as isize,
        );
    }

    state.last_icons_ready = state.shared.icons_ready.load(Ordering::Relaxed);
    state.last_champion_count = champions.len();
}

unsafe fn add_selected(state: &mut PickerState) {
    let selected = SendMessageW(
        state.list,
        LVM_GETNEXTITEM,
        usize::MAX,
        LVNI_SELECTED as isize,
    ) as i32;
    if selected < 0 {
        return;
    }
    let Some(&champion_index) = state.filtered.get(selected as usize) else {
        return;
    };
    let champion = state
        .shared
        .champions
        .lock()
        .ok()
        .and_then(|champions| champions.get(champion_index).cloned());
    let Some(champion) = champion else {
        return;
    };
    mutate_candidates(state, |candidates| {
        if candidates.len() < 8
            && !candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&champion.alias))
        {
            candidates.push(champion.alias.clone());
        }
    });
}

unsafe fn mutate_candidates(state: &mut PickerState, mutate: impl FnOnce(&mut Vec<String>)) {
    if let Ok(mut settings) = state.shared.settings.lock() {
        mutate(candidate_list_mut(&mut settings, state.role, state.kind));
        let _ = settings.save();
    }
    update_selected_text(state);
}

unsafe fn update_selected_text(state: &PickerState) {
    let text = state
        .shared
        .settings
        .lock()
        .map(|settings| {
            let candidates = candidate_list(&settings, state.role, state.kind);
            if candidates.is_empty() {
                "候補: 未設定".to_owned()
            } else {
                format!("候補: {}", candidates.join("  ›  "))
            }
        })
        .unwrap_or_else(|_| "候補を読み込めません".into());
    set_text(state.selected, &text);
}

fn candidate_list(settings: &Settings, role: Role, kind: SelectionKind) -> &[String] {
    match (role, kind) {
        (Role::Top, SelectionKind::Pick) => &settings.top,
        (Role::Jungle, SelectionKind::Pick) => &settings.jungle,
        (Role::Middle, SelectionKind::Pick) => &settings.middle,
        (Role::Bottom, SelectionKind::Pick) => &settings.bottom,
        (Role::Utility, SelectionKind::Pick) => &settings.utility,
        (Role::Top, SelectionKind::Ban) => &settings.ban_top,
        (Role::Jungle, SelectionKind::Ban) => &settings.ban_jungle,
        (Role::Middle, SelectionKind::Ban) => &settings.ban_middle,
        (Role::Bottom, SelectionKind::Ban) => &settings.ban_bottom,
        (Role::Utility, SelectionKind::Ban) => &settings.ban_utility,
    }
}

fn candidate_list_mut(
    settings: &mut Settings,
    role: Role,
    kind: SelectionKind,
) -> &mut Vec<String> {
    match (role, kind) {
        (Role::Top, SelectionKind::Pick) => &mut settings.top,
        (Role::Jungle, SelectionKind::Pick) => &mut settings.jungle,
        (Role::Middle, SelectionKind::Pick) => &mut settings.middle,
        (Role::Bottom, SelectionKind::Pick) => &mut settings.bottom,
        (Role::Utility, SelectionKind::Pick) => &mut settings.utility,
        (Role::Top, SelectionKind::Ban) => &mut settings.ban_top,
        (Role::Jungle, SelectionKind::Ban) => &mut settings.ban_jungle,
        (Role::Middle, SelectionKind::Ban) => &mut settings.ban_middle,
        (Role::Bottom, SelectionKind::Ban) => &mut settings.ban_bottom,
        (Role::Utility, SelectionKind::Ban) => &mut settings.ban_utility,
    }
}

unsafe fn load_thumbnail(champion_id: i64) -> HBITMAP {
    let path = champion_icon_path(champion_id);
    if !path.is_file() {
        return null_mut();
    }
    let Some(path) = path.to_str() else {
        return null_mut();
    };
    let mut image: *mut GpBitmap = null_mut();
    if GdipCreateBitmapFromFile(wide(path).as_ptr(), &mut image) != 0 || image.is_null() {
        return null_mut();
    }
    let mut thumbnail: *mut GpImage = null_mut();
    let status =
        GdipGetImageThumbnail(image as *mut GpImage, 56, 56, &mut thumbnail, 0, null_mut());
    GdipDisposeImage(image as *mut GpImage);
    if status != 0 || thumbnail.is_null() {
        return null_mut();
    }
    let mut bitmap: HBITMAP = null_mut();
    let status = GdipCreateHBITMAPFromBitmap(thumbnail as *mut GpBitmap, &mut bitmap, 0xff20242d);
    GdipDisposeImage(thumbnail);
    if status == 0 { bitmap } else { null_mut() }
}

fn ensure_gdiplus() -> Result<(), String> {
    static TOKEN: OnceLock<usize> = OnceLock::new();
    if TOKEN.get().is_some() {
        return Ok(());
    }
    let mut token = 0usize;
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        DebugEventCallback: 0,
        SuppressBackgroundThread: 0,
        SuppressExternalCodecs: 0,
    };
    let status = unsafe { GdiplusStartup(&mut token, &input, null_mut()) };
    if status == 0 {
        let _ = TOKEN.set(token);
        Ok(())
    } else {
        Err("GDI+を初期化できません".into())
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

unsafe fn create_button(
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
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        x,
        y,
        width,
        34,
        parent,
        id,
        font,
    );
    SetWindowTheme(control, wide("DarkMode_Explorer").as_ptr(), null());
    control
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

unsafe fn get_text(control: HWND) -> String {
    let length = GetWindowTextLengthW(control);
    let mut buffer = vec![0u16; length as usize + 1];
    GetWindowTextW(control, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..length as usize])
}

unsafe fn set_text(control: HWND, value: &str) {
    if get_text(control) == value {
        return;
    }

    SetWindowTextW(control, wide(value).as_ptr());
}

unsafe fn draw_button(item: &DRAWITEMSTRUCT) {
    let pressed = item.itemState & ODS_SELECTED != 0;
    let accent = item.CtlID as i32 == ID_ADD;
    let color = match (accent, pressed) {
        (true, true) => COLOR_ACCENT_PRESSED,
        (true, false) => COLOR_ACCENT,
        (false, true) => COLOR_BUTTON_PRESSED,
        (false, false) => COLOR_BUTTON,
    };
    let brush = CreateSolidBrush(color);
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
    let mut rect = item.rcItem;
    SetTextColor(item.hDC, COLOR_TEXT);
    SetBkMode(item.hDC, TRANSPARENT as i32);
    DrawTextW(
        item.hDC,
        text.as_ptr(),
        length,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

unsafe fn create_font(size: i32) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        FW_NORMAL as i32,
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

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Top => "TOP",
        Role::Jungle => "JUNGLE",
        Role::Middle => "MID",
        Role::Bottom => "ADC",
        Role::Utility => "SUPPORT",
    }
}

fn list_needs_rebuild(
    total: usize,
    ready: usize,
    last_champion_count: usize,
    last_icons_ready: usize,
) -> bool {
    let icons_changed = ready != last_icons_ready;
    total != last_champion_count
        || (icons_changed && (ready == total || ready.saturating_sub(last_icons_ready) >= 40))
}

const fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    red | (green << 8) | (blue << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::list_needs_rebuild;

    #[test]
    fn keeps_completed_icon_list_stable() {
        assert!(!list_needs_rebuild(233, 233, 233, 233));
        assert!(list_needs_rebuild(233, 233, 233, 200));
        assert!(list_needs_rebuild(234, 233, 233, 233));
    }
}
