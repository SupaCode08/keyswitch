#![windows_subsystem = "windows"]

mod engine;

use engine::{
    extract_embedded_data_to_dir, vk_to_us_char, CorrectionMode, Language, NotificationBanner,
    PendingOsReplacement, RightLangEngine, APP_AUTHOR, APP_BRAND_NAME,
};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use winapi::shared::minwindef::{DWORD, HKL, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HDC, HFONT, HGDIOBJ, HICON, HWND, POINT, RECT};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use winapi::um::wingdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CLEARTYPE_QUALITY, DEFAULT_CHARSET, DIB_RGB_COLORS, FW_BOLD, FW_NORMAL, OUT_DEFAULT_PRECIS,
    RGB, SRCCOPY, TRANSPARENT,
};
use winapi::um::winuser::*;

#[link(name = "shell32")]
extern "system" {
    fn SHChangeNotify(
        wEventId: i32,
        uFlags: u32,
        dwItem1: *const winapi::ctypes::c_void,
        dwItem2: *const winapi::ctypes::c_void,
    );
}

const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
const SHCNF_IDLIST: u32 = 0x0000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const WM_TRAYICON: u32 = WM_APP + 42;
const WM_ENGINE_NOTIFY: u32 = WM_APP + 43;
const TIMER_TOAST_FADE: usize = 102;
const TIMER_TRAY_RETRY: usize = 103;
const TRAY_ICON_UID: u32 = 9901;

static WM_TASKBARCREATED: AtomicU32 = AtomicU32::new(0);

const ICON_16_BGRA: &[u8] = include_bytes!("../assets/icon_16.bgra");
const ICON_24_BGRA: &[u8] = include_bytes!("../assets/icon_24.bgra");
const ICON_32_BGRA: &[u8] = include_bytes!("../assets/icon_32.bgra");
const ICON_48_BGRA: &[u8] = include_bytes!("../assets/icon_48.bgra");
const ICON_64_BGRA: &[u8] = include_bytes!("../assets/icon_64.bgra");

static IS_INJECTING: AtomicBool = AtomicBool::new(false);
static ACTIVE_BUF_SC: AtomicU8 = AtomicU8::new(0);
static ACTIVE_SEL_SC: AtomicU8 = AtomicU8::new(0);
static MAIN_HWND: Mutex<usize> = Mutex::new(0);
static TOAST_HWND: Mutex<usize> = Mutex::new(0);
static HOOK_SENDER: Mutex<Option<Sender<HookEvent>>> = Mutex::new(None);
static GLOBAL_APP_STATE: Mutex<Option<Arc<Mutex<AppUiState>>>> = Mutex::new(None);

#[derive(Debug, Clone)]
struct ConfigDraft {
    learn_new_word: bool,
    show_popup: bool,
    switch_buffer_shortcut: String,
    switch_select_shortcut: String,
    thai_ui: bool,
}

impl ConfigDraft {
    fn from_engine(eng: &RightLangEngine) -> Self {
        Self {
            learn_new_word: eng.learn_new_word,
            show_popup: eng.show_popup,
            switch_buffer_shortcut: eng.switch_buffer_shortcut.clone(),
            switch_select_shortcut: eng.switch_select_shortcut.clone(),
            thai_ui: eng.thai_ui,
        }
    }

    fn defaults() -> Self {
        Self {
            learn_new_word: true,
            show_popup: true,
            switch_buffer_shortcut: "Shift + Backspace".to_string(),
            switch_select_shortcut: "Shift + Caps Lock".to_string(),
            thai_ui: true,
        }
    }

    fn apply_to_engine(&self, eng: &mut RightLangEngine) {
        eng.learn_new_word = self.learn_new_word;
        eng.show_popup = self.show_popup;
        eng.switch_buffer_shortcut = self.switch_buffer_shortcut.clone();
        eng.switch_select_shortcut = self.switch_select_shortcut.clone();
        eng.thai_ui = self.thai_ui;
        sync_active_shortcut_atomics(eng);
    }
}

fn sync_active_shortcut_atomics(eng: &RightLangEngine) {
    let b = match eng.switch_buffer_shortcut.as_str() {
        "Ctrl + Backspace" => 1,
        "Pause" => 2,
        _ => 0,
    };
    let s = match eng.switch_select_shortcut.as_str() {
        "Scroll Lock" => 1,
        _ => 0,
    };
    ACTIVE_BUF_SC.store(b, Ordering::Relaxed);
    ACTIVE_SEL_SC.store(s, Ordering::Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveView {
    InstallerWizard,
    ConfigForm,
}

#[derive(Debug, Clone)]
enum HookEvent {
    PrintableKey { us_char: char, hwnd: usize },
    Boundary { ch: char, hwnd: usize },
    Backspace,
    ClearBuffer,
    SwitchBufferShortcut { hwnd: usize },
    SwitchSelectedShortcut { hwnd: usize },
    ToggleMode,
}

#[derive(Clone, Copy)]
struct ClickRegion {
    rect: RECT,
    action_id: u32,
}

struct AppUiState {
    engine: RightLangEngine,
    draft: ConfigDraft,
    active_view: ActiveView,
    is_installer_process: bool,
    inst_step: u8,
    inst_desktop_shortcut: bool,
    inst_autostart: bool,
    inst_launch_now: bool,
    win_w: i32,
    win_h: i32,
    hover_pos: (i32, i32),
    regions: Vec<ClickRegion>,
    status_banner: Option<String>,
    toast_undo_rect: RECT,
    toast_mute_rect: RECT,
}

const CMD_MODE_ONLINE: usize = 1001;
const CMD_MODE_OFFLINE: usize = 1002;
const CMD_MODE_DISABLED: usize = 1003;
const CMD_TOGGLE_AUTOSTART: usize = 1004;
const CMD_OPEN_CONFIG: usize = 1005;
const CMD_TOGGLE_UI_LANG: usize = 1007;
const CMD_EXIT: usize = 1008;

const ACT_CLOSE_DIALOG: u32 = 20;
const ACT_CFG_TOGGLE_LEARN: u32 = 21;
const ACT_CFG_TOGGLE_POPUP: u32 = 22;
const ACT_CFG_CYCLE_BUF_SC: u32 = 23;
const ACT_CFG_CYCLE_SEL_SC: u32 = 24;
const ACT_CFG_CYCLE_UI_LANG: u32 = 25;
const ACT_CFG_SAVE_OK: u32 = 26;
const ACT_CFG_APPLY: u32 = 27;
const ACT_CFG_RESET: u32 = 28;

const ACT_INST_TOGGLE_DESKTOP: u32 = 70;
const ACT_INST_TOGGLE_AUTOSTART: u32 = 71;
const ACT_INST_TOGGLE_LAUNCH: u32 = 72;
const ACT_INST_RUN_NOW: u32 = 73;
const ACT_INST_EXIT: u32 = 74;
const ACT_INST_NEXT: u32 = 75;
const ACT_INST_BACK: u32 = 76;

fn to_wstring(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn view_dimensions(view: ActiveView) -> (i32, i32) {
    match view {
        ActiveView::InstallerWizard => (500, 360),
        ActiveView::ConfigForm => (430, 294),
    }
}

fn view_title(view: ActiveView, th: bool) -> &'static str {
    match (view, th) {
        (ActiveView::InstallerWizard, true) => "KeySwitch 2.0 Setup Wizard",
        (ActiveView::InstallerWizard, false) => "KeySwitch 2.0 Setup Wizard",
        (ActiveView::ConfigForm, true) => "KeySwitch — ตั้งค่า",
        (ActiveView::ConfigForm, false) => "KeySwitch — Settings",
    }
}

fn center_screen_pos(win_w: i32, win_h: i32) -> (i32, i32) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect as *mut RECT as *mut _, 0) != 0 {
            let x = rect.left + ((rect.right - rect.left - win_w) / 2).max(0);
            let y = rect.top + ((rect.bottom - rect.top - win_h) / 2).max(0);
            return (x, y);
        }
    }
    (200, 200)
}

fn bottom_right_workarea_pos(win_w: i32, win_h: i32) -> (i32, i32) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rect as *mut RECT as *mut _, 0) != 0 {
            let x = (rect.right - win_w - 10).max(0);
            let y = (rect.bottom - win_h - 10).max(0);
            return (x, y);
        }
    }
    (100, 100)
}

fn select_embedded_icon_bgra(target_size: usize) -> (usize, &'static [u8]) {
    if target_size <= 18 {
        (16, ICON_16_BGRA)
    } else if target_size <= 26 {
        (24, ICON_24_BGRA)
    } else if target_size <= 38 {
        (32, ICON_32_BGRA)
    } else if target_size <= 54 {
        (48, ICON_48_BGRA)
    } else {
        (64, ICON_64_BGRA)
    }
}

fn build_keyswitch_icon_dib(size: usize, mode: CorrectionMode) -> Vec<u8> {
    let (src_sz, raw) = select_embedded_icon_bgra(size);
    let row_mask_bytes = ((src_sz + 31) / 32) * 4;
    let image_size = (src_sz * src_sz * 4 + row_mask_bytes * src_sz) as u32;
    let mut dib = Vec::with_capacity(40 + image_size as usize);

    let width = src_sz as i32;
    let height = (src_sz * 2) as i32;
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&width.to_le_bytes());
    dib.extend_from_slice(&height.to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&image_size.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0i32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());

    let mut and_mask = vec![0u8; row_mask_bytes * src_sz];

    for y_inv in 0..src_sz {
        let src_y = src_sz - 1 - y_inv;
        for x in 0..src_sz {
            let i = (src_y * src_sz + x) * 4;
            let mut b = raw[i];
            let mut g = raw[i + 1];
            let mut r = raw[i + 2];
            let a = raw[i + 3];

            match mode {
                CorrectionMode::Online => {}
                CorrectionMode::Offline => {
                    if g > 170 && r < 170 {
                        r = 248;
                        g = 180;
                        b = 35;
                    }
                }
                CorrectionMode::Disabled => {
                    let gray = ((r as u32 * 30 + g as u32 * 59 + b as u32 * 11) / 100) as u8;
                    r = gray;
                    g = gray;
                    b = gray;
                }
            }

            dib.extend_from_slice(&[b, g, r, a]);
            if a < 16 {
                let byte_idx = y_inv * row_mask_bytes + (x / 8);
                let bit_idx = 7 - (x % 8);
                and_mask[byte_idx] |= 1 << bit_idx;
            }
        }
    }

    dib.extend_from_slice(&and_mask);
    dib
}

unsafe fn create_keyswitch_hicon(size: i32, mode: CorrectionMode) -> HICON {
    let (src_sz, _) = select_embedded_icon_bgra(size.max(16) as usize);
    let mut dib = build_keyswitch_icon_dib(src_sz, mode);
    CreateIconFromResourceEx(
        dib.as_mut_ptr(),
        dib.len() as u32,
        1,
        0x0003_0000,
        src_sz as i32,
        src_sz as i32,
        0,
    )
}

fn write_keyswitch_ico_file(path: &Path) {
    let sizes = [16usize, 24, 32, 48, 64];
    let frames: Vec<(usize, Vec<u8>)> = sizes
        .iter()
        .map(|&s| (s, build_keyswitch_icon_dib(s, CorrectionMode::Online)))
        .collect();
    let count = frames.len() as u16;
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6u32 + (frames.len() as u32) * 16;
    for (sz, dib) in &frames {
        ico.push(*sz as u8);
        ico.push(*sz as u8);
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += dib.len() as u32;
    }
    for (_, dib) in &frames {
        ico.extend_from_slice(dib);
    }
    let _ = std::fs::write(path, ico);
}

unsafe fn add_or_update_tray_icon(
    hwnd: HWND,
    mode: CorrectionMode,
    thai_ui: bool,
    is_add: bool,
) -> bool {
    let icon_sz = GetSystemMetrics(SM_CXSMICON).max(16);
    let h_icon = create_keyswitch_hicon(icon_sz, mode);
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_UID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = h_icon;

    let tip_str = format!("{} — {}\0", APP_BRAND_NAME, mode.label(thai_ui));
    let tip: Vec<u16> = tip_str.encode_utf16().collect();
    for (i, &ch) in tip.iter().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = ch;
    }

    let ok = if is_add {
        Shell_NotifyIconW(NIM_DELETE, &mut nid);
        Shell_NotifyIconW(NIM_ADD, &mut nid) != 0
    } else {
        let m = Shell_NotifyIconW(NIM_MODIFY, &mut nid) != 0;
        if !m {
            Shell_NotifyIconW(NIM_ADD, &mut nid) != 0
        } else {
            true
        }
    };
    if !h_icon.is_null() {
        DestroyIcon(h_icon);
    }
    ok
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_UID;
    Shell_NotifyIconW(NIM_DELETE, &mut nid);
}

unsafe fn show_native_tray_popup_menu(
    hwnd: HWND,
    th: bool,
    mode: CorrectionMode,
    auto_start: bool,
) {
    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    let f_online = MF_STRING
        | if mode == CorrectionMode::Online {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
    let f_offline = MF_STRING
        | if mode == CorrectionMode::Offline {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
    let f_disabled = MF_STRING
        | if mode == CorrectionMode::Disabled {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
    let f_autostart = MF_STRING | if auto_start { MF_CHECKED } else { MF_UNCHECKED };

    AppendMenuW(
        hmenu,
        f_online,
        CMD_MODE_ONLINE,
        to_wstring(if th {
            "แก้ไขอัตโนมัติ"
        } else {
            "Auto Mode"
        })
        .as_ptr(),
    );
    AppendMenuW(
        hmenu,
        f_offline,
        CMD_MODE_OFFLINE,
        to_wstring(if th {
            "แก้ไขโดยผู้ใช้"
        } else {
            "Manual Mode"
        })
        .as_ptr(),
    );
    AppendMenuW(
        hmenu,
        f_disabled,
        CMD_MODE_DISABLED,
        to_wstring(if th { "ไม่ใช้งาน" } else { "Disabled" }).as_ptr(),
    );

    AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());

    AppendMenuW(
        hmenu,
        f_autostart,
        CMD_TOGGLE_AUTOSTART,
        to_wstring(if th {
            "เริ่มเมื่อเปิดเครื่อง"
        } else {
            "Auto Start"
        })
        .as_ptr(),
    );

    AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());

    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_OPEN_CONFIG,
        to_wstring(if th { "ตั้งค่า..." } else { "Settings..." }).as_ptr(),
    );

    AppendMenuW(hmenu, MF_SEPARATOR, 0, null_mut());

    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_TOGGLE_UI_LANG,
        to_wstring(if th {
            "ภาษาเมนู: English"
        } else {
            "UI Language: ภาษาไทย"
        })
        .as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_EXIT,
        to_wstring(if th { "จบการทำงาน" } else { "Exit" }).as_ptr(),
    );

    let mut pt: POINT = std::mem::zeroed();
    GetCursorPos(&mut pt);

    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        hmenu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        null_mut(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(hmenu);

    if cmd > 0 {
        PostMessageW(hwnd, WM_COMMAND, cmd as WPARAM, 0);
    }
}

unsafe fn open_dialog_view_safe(hwnd: HWND, view: ActiveView) {
    let (client_w, client_h, title_str) = {
        let arc_opt = GLOBAL_APP_STATE
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        let Some(arc) = arc_opt else { return };
        let Ok(mut st) = arc.lock() else { return };
        st.active_view = view;
        st.status_banner = None;
        if view == ActiveView::ConfigForm {
            st.draft = ConfigDraft::from_engine(&st.engine);
        }
        let (cw, ch) = view_dimensions(view);
        st.win_w = cw;
        st.win_h = ch;
        let t = view_title(view, st.engine.thai_ui);
        (cw, ch, t)
    };

    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    AdjustWindowRectEx(&mut rc, style, 0, 0);
    let outer_w = rc.right - rc.left;
    let outer_h = rc.bottom - rc.top;
    let (x, y) = center_screen_pos(outer_w, outer_h);

    SetWindowTextW(hwnd, to_wstring(title_str).as_ptr());
    SetWindowPos(hwnd, HWND_TOP, x, y, outer_w, outer_h, SWP_SHOWWINDOW);
    SetForegroundWindow(hwnd);
    InvalidateRect(hwnd, null_mut(), 1);
}

unsafe fn close_dialog_to_tray_safe(hwnd: HWND) {
    let is_inst = {
        let arc_opt = GLOBAL_APP_STATE
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        let Some(arc) = arc_opt else { return };
        let Ok(mut st) = arc.lock() else { return };
        st.draft = ConfigDraft::from_engine(&st.engine);
        st.status_banner = None;
        st.is_installer_process
    };
    if is_inst {
        PostQuitMessage(0);
        return;
    }
    ShowWindow(hwnd, SW_HIDE);
}

unsafe fn create_ui_font(px_height: i32, bold: bool) -> HFONT {
    let font_name = to_wstring("Segoe UI");
    CreateFontW(
        -px_height,
        0,
        0,
        0,
        if bold { FW_BOLD } else { FW_NORMAL },
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        0,
        CLEARTYPE_QUALITY,
        0,
        font_name.as_ptr(),
    )
}

unsafe fn draw_solid_rect(hdc: HDC, r: &RECT, color: u32) {
    let br = CreateSolidBrush(color);
    FillRect(hdc, r, br);
    DeleteObject(br as HGDIOBJ);
}

unsafe fn draw_border_rect(hdc: HDC, r: &RECT, fill_col: u32, border_col: u32) {
    let br_fill = CreateSolidBrush(fill_col);
    FillRect(hdc, r, br_fill);
    DeleteObject(br_fill as HGDIOBJ);
    let br_border = CreateSolidBrush(border_col);
    FrameRect(hdc, r, br_border);
    DeleteObject(br_border as HGDIOBJ);
}

unsafe fn draw_text_left(hdc: HDC, x: i32, y: i32, w: i32, h: i32, text: &str, color: u32) {
    SetTextColor(hdc, color);
    let mut rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let ws = to_wstring(text);
    DrawTextW(
        hdc,
        ws.as_ptr(),
        (ws.len() - 1) as i32,
        &mut rc,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
}

unsafe fn draw_text_wrap(hdc: HDC, x: i32, y: i32, w: i32, h: i32, text: &str, color: u32) {
    SetTextColor(hdc, color);
    let mut rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let ws = to_wstring(text);
    DrawTextW(
        hdc,
        ws.as_ptr(),
        (ws.len() - 1) as i32,
        &mut rc,
        DT_LEFT | DT_WORDBREAK | DT_NOPREFIX,
    );
}

unsafe fn draw_text_center(hdc: HDC, r: &RECT, text: &str, color: u32) {
    SetTextColor(hdc, color);
    let mut rc = *r;
    let ws = to_wstring(text);
    DrawTextW(
        hdc,
        ws.as_ptr(),
        (ws.len() - 1) as i32,
        &mut rc,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
}

unsafe fn draw_keyswitch_badge(hdc: HDC, cx: i32, cy: i32, radius: i32, mode: CorrectionMode) {
    let sz = (radius * 2).max(16);
    let h_icon = create_keyswitch_hicon(sz, mode);
    if !h_icon.is_null() {
        DrawIconEx(
            hdc,
            cx - radius,
            cy - radius,
            h_icon,
            sz,
            sz,
            0,
            null_mut(),
            0x0003,
        );
        DestroyIcon(h_icon);
    }
}

fn pt_in_rect(pt: (i32, i32), r: &RECT) -> bool {
    pt.0 >= r.left && pt.0 < r.right && pt.1 >= r.top && pt.1 < r.bottom
}

unsafe fn draw_button(
    hdc: HDC,
    state: &mut AppUiState,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &str,
    action_id: u32,
    primary: bool,
) {
    let rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let hovered = pt_in_rect(state.hover_pos, &rc);
    let (fill, border, txt_col) = if primary {
        if hovered {
            (RGB(18, 128, 145), RGB(12, 95, 110), RGB(255, 255, 255))
        } else {
            (RGB(14, 98, 114), RGB(10, 72, 86), RGB(255, 255, 255))
        }
    } else if hovered {
        (RGB(224, 240, 244), RGB(14, 116, 130), RGB(15, 15, 15))
    } else {
        (RGB(232, 234, 238), RGB(175, 178, 184), RGB(15, 15, 15))
    };
    draw_border_rect(hdc, &rc, fill, border);
    draw_text_center(hdc, &rc, label, txt_col);
    state.regions.push(ClickRegion {
        rect: rc,
        action_id,
    });
}

unsafe fn draw_wizard_button(
    hdc: HDC,
    state: &mut AppUiState,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &str,
    action_id: u32,
    enabled: bool,
    focused: bool,
) {
    let rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    if !enabled {
        draw_border_rect(hdc, &rc, RGB(236, 236, 236), RGB(204, 204, 204));
        draw_text_center(hdc, &rc, label, RGB(150, 150, 150));
        return;
    }
    let hovered = pt_in_rect(state.hover_pos, &rc);
    let (fill, border) = if hovered {
        (RGB(226, 241, 245), RGB(14, 116, 130))
    } else if focused {
        (RGB(225, 234, 238), RGB(14, 108, 124))
    } else {
        (RGB(228, 228, 228), RGB(172, 172, 172))
    };
    draw_border_rect(hdc, &rc, fill, border);
    if focused {
        let inner = RECT {
            left: x + 1,
            top: y + 1,
            right: x + w - 1,
            bottom: y + h - 1,
        };
        let br = CreateSolidBrush(RGB(14, 108, 124));
        FrameRect(hdc, &inner, br);
        DeleteObject(br as HGDIOBJ);
    }
    draw_text_center(hdc, &rc, label, RGB(18, 20, 24));
    state.regions.push(ClickRegion {
        rect: rc,
        action_id,
    });
}

unsafe fn draw_checkbox_row(
    hdc: HDC,
    state: &mut AppUiState,
    x: i32,
    y: i32,
    w: i32,
    checked: bool,
    label: &str,
    action_id: u32,
) {
    let rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + 24,
    };
    let box_rc = RECT {
        left: x + 2,
        top: y + 4,
        right: x + 18,
        bottom: y + 20,
    };
    let hovered = pt_in_rect(state.hover_pos, &rc);
    let fill = if hovered {
        RGB(230, 245, 248)
    } else {
        RGB(255, 255, 255)
    };
    let border = if checked {
        RGB(14, 116, 130)
    } else {
        RGB(145, 148, 155)
    };
    draw_border_rect(hdc, &box_rc, fill, border);
    if checked {
        draw_text_center(hdc, &box_rc, "✓", RGB(14, 116, 130));
    }
    draw_text_left(hdc, x + 24, y, w - 24, 24, label, RGB(18, 20, 25));
    state.regions.push(ClickRegion {
        rect: rc,
        action_id,
    });
}

unsafe fn render_ui_to_dc(hdc: HDC, state: &mut AppUiState) {
    state.regions.clear();
    let w = state.win_w;
    let h = state.win_h;
    SetBkMode(hdc, TRANSPARENT as i32);

    let h_font_reg = create_ui_font(14, false);
    let h_font_bold = create_ui_font(14, true);
    let h_font_small = create_ui_font(13, false);
    let h_font_title = create_ui_font(22, true);

    let old_font = SelectObject(hdc, h_font_reg as HGDIOBJ);
    let full_rc = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    draw_solid_rect(hdc, &full_rc, RGB(243, 244, 246));

    match state.active_view {
        ActiveView::InstallerWizard => {
            render_installer_wizard_dc(
                hdc,
                state,
                h_font_reg,
                h_font_bold,
                h_font_small,
                h_font_title,
            );
        }
        ActiveView::ConfigForm => {
            render_config_dc(hdc, state, h_font_reg, h_font_bold);
        }
    }

    SelectObject(hdc, old_font);
    DeleteObject(h_font_reg as HGDIOBJ);
    DeleteObject(h_font_bold as HGDIOBJ);
    DeleteObject(h_font_small as HGDIOBJ);
    DeleteObject(h_font_title as HGDIOBJ);
}

unsafe fn render_installer_wizard_dc(
    hdc: HDC,
    state: &mut AppUiState,
    h_font_reg: HFONT,
    h_font_bold: HFONT,
    h_font_small: HFONT,
    h_font_title: HFONT,
) {
    let w = state.win_w;
    let h = state.win_h;
    let sidebar_w = 164;
    let bar_y = h - 46;

    let left_rc = RECT {
        left: 0,
        top: 0,
        right: sidebar_w,
        bottom: bar_y,
    };
    draw_solid_rect(hdc, &left_rc, RGB(12, 66, 80));

    let accent1 = RECT {
        left: 0,
        top: bar_y - 120,
        right: sidebar_w,
        bottom: bar_y - 116,
    };
    draw_solid_rect(hdc, &accent1, RGB(22, 108, 122));

    let accent2 = RECT {
        left: 0,
        top: bar_y - 116,
        right: sidebar_w,
        bottom: bar_y,
    };
    draw_solid_rect(hdc, &accent2, RGB(16, 82, 98));

    let frame_rc = RECT {
        left: 52,
        top: 26,
        right: 112,
        bottom: 86,
    };
    draw_border_rect(hdc, &frame_rc, RGB(255, 255, 255), RGB(195, 232, 236));
    draw_keyswitch_badge(hdc, 82, 56, 24, CorrectionMode::Online);

    SelectObject(hdc, h_font_bold as HGDIOBJ);
    let brand_rc = RECT {
        left: 10,
        top: 98,
        right: sidebar_w - 10,
        bottom: 120,
    };
    draw_text_center(hdc, &brand_rc, "KeySwitch 2.0", RGB(255, 255, 255));

    SelectObject(hdc, h_font_small as HGDIOBJ);
    let sub_rc = RECT {
        left: 10,
        top: 120,
        right: sidebar_w - 10,
        bottom: 138,
    };
    draw_text_center(hdc, &sub_rc, "Rust Native Engine", RGB(168, 238, 224));

    let step_txt = match state.inst_step {
        0 => "ขั้นตอน 1 / 3",
        1 => "ขั้นตอน 2 / 3",
        _ => "ขั้นตอน 3 / 3",
    };
    let step_rc = RECT {
        left: 14,
        top: bar_y - 34,
        right: sidebar_w - 14,
        bottom: bar_y - 14,
    };
    draw_text_center(hdc, &step_rc, step_txt, RGB(180, 235, 226));

    let right_rc = RECT {
        left: sidebar_w,
        top: 0,
        right: w,
        bottom: bar_y,
    };
    draw_solid_rect(hdc, &right_rc, RGB(255, 255, 255));

    let cx = sidebar_w + 22;
    let cw = w - sidebar_w - 40;

    match state.inst_step {
        0 => {
            SelectObject(hdc, h_font_title as HGDIOBJ);
            draw_text_wrap(
                hdc,
                cx,
                22,
                cw,
                64,
                "ยินดีต้อนรับสู่ตัวติดตั้ง\nKeySwitch 2.0",
                RGB(15, 20, 28),
            );

            SelectObject(hdc, h_font_reg as HGDIOBJ);
            draw_text_wrap(
                hdc,
                cx,
                96,
                cw,
                130,
                "ตัวช่วยติดตั้งนี้จะติดตั้งโปรแกรม KeySwitch 2.0 ลงในเครื่องคอมพิวเตอร์ของคุณ\n\nโปรแกรมสลับภาษาแป้นพิมพ์ไทย-อังกฤษอัตโนมัติ เขียนด้วย Rust Native ทำงานเร็วและใช้ทรัพยากรเครื่องน้อยมาก",
                RGB(35, 40, 48),
            );

            draw_text_left(
                hdc,
                cx,
                bar_y - 36,
                cw,
                22,
                "คลิก ถัดไป เพื่อดำเนินการต่อ",
                RGB(25, 30, 38),
            );
        }
        1 => {
            SelectObject(hdc, h_font_title as HGDIOBJ);
            draw_text_left(
                hdc,
                cx,
                18,
                cw,
                32,
                "ตัวเลือกการติดตั้ง",
                RGB(15, 20, 28),
            );

            let local_app_data =
                std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\Public".to_string());
            let dest_str = format!(r"{}\Programs\KeySwitch", local_app_data);

            SelectObject(hdc, h_font_bold as HGDIOBJ);
            draw_text_left(hdc, cx, 62, cw, 20, "โฟลเดอร์ปลายทาง:", RGB(30, 38, 48));

            let path_box = RECT {
                left: cx,
                top: 86,
                right: cx + cw,
                bottom: 112,
            };
            draw_border_rect(hdc, &path_box, RGB(248, 250, 252), RGB(195, 202, 210));
            SelectObject(hdc, h_font_small as HGDIOBJ);
            draw_text_left(hdc, cx + 8, 86, cw - 16, 26, &dest_str, RGB(14, 108, 124));

            SelectObject(hdc, h_font_bold as HGDIOBJ);
            draw_text_left(hdc, cx, 128, cw, 20, "การตั้งค่าเพิ่มเติม:", RGB(30, 38, 48));

            SelectObject(hdc, h_font_reg as HGDIOBJ);
            let d_sc = state.inst_desktop_shortcut;
            let a_st = state.inst_autostart;
            let l_nw = state.inst_launch_now;

            draw_checkbox_row(
                hdc,
                state,
                cx,
                154,
                cw,
                d_sc,
                "สร้างไอคอนบนหน้าจอ Desktop",
                ACT_INST_TOGGLE_DESKTOP,
            );
            draw_checkbox_row(
                hdc,
                state,
                cx,
                182,
                cw,
                a_st,
                "เริ่มทำงานอัตโนมัติเมื่อเปิดเครื่อง Windows",
                ACT_INST_TOGGLE_AUTOSTART,
            );
            draw_checkbox_row(
                hdc,
                state,
                cx,
                210,
                cw,
                l_nw,
                "เปิดใช้งานบน System Tray ทันทีหลังติดตั้ง",
                ACT_INST_TOGGLE_LAUNCH,
            );

            if let Some(err_msg) = &state.status_banner {
                SelectObject(hdc, h_font_small as HGDIOBJ);
                draw_text_wrap(hdc, cx, 244, cw, 44, err_msg, RGB(185, 35, 35));
            } else {
                SelectObject(hdc, h_font_reg as HGDIOBJ);
                draw_text_left(
                    hdc,
                    cx,
                    bar_y - 36,
                    cw,
                    22,
                    "คลิก ติดตั้ง เพื่อเริ่มการติดตั้ง",
                    RGB(25, 30, 38),
                );
            }
        }
        _ => {
            SelectObject(hdc, h_font_title as HGDIOBJ);
            draw_text_wrap(
                hdc,
                cx,
                22,
                cw,
                64,
                "ติดตั้ง KeySwitch 2.0\nเสร็จสมบูรณ์",
                RGB(14, 96, 80),
            );

            let pb_bg = RECT {
                left: cx,
                top: 98,
                right: cx + cw,
                bottom: 114,
            };
            draw_border_rect(hdc, &pb_bg, RGB(232, 238, 240), RGB(175, 192, 198));
            let pb_fill = RECT {
                left: cx + 1,
                top: 99,
                right: cx + cw - 1,
                bottom: 113,
            };
            draw_solid_rect(hdc, &pb_fill, RGB(22, 172, 128));

            SelectObject(hdc, h_font_reg as HGDIOBJ);
            draw_text_wrap(
                hdc,
                cx,
                128,
                cw,
                110,
                "ติดตั้งโปรแกรม KeySwitch 2.0 และสร้างไอคอนเรียบร้อยแล้ว\n\nคุณสามารถคลิกไอคอน KeySwitch ที่มุมขวาล่างของ Taskbar (System Tray) เพื่อปรับตั้งค่าได้ตลอดเวลา",
                RGB(35, 40, 48),
            );

            draw_text_left(
                hdc,
                cx,
                bar_y - 36,
                cw,
                22,
                "คลิก เสร็จสิ้น เพื่อปิดตัวติดตั้ง",
                RGB(25, 30, 38),
            );
        }
    }

    let sep_rc = RECT {
        left: 0,
        top: bar_y,
        right: w,
        bottom: bar_y + 1,
    };
    draw_solid_rect(hdc, &sep_rc, RGB(205, 208, 212));

    let bar_rc = RECT {
        left: 0,
        top: bar_y + 1,
        right: w,
        bottom: h,
    };
    draw_solid_rect(hdc, &bar_rc, RGB(240, 240, 240));

    SelectObject(hdc, h_font_reg as HGDIOBJ);
    let btn_y = bar_y + 11;
    let btn_w = 82;
    let btn_h = 24;

    match state.inst_step {
        0 => {
            draw_wizard_button(
                hdc,
                state,
                w - 270,
                btn_y,
                btn_w,
                btn_h,
                "< ย้อนกลับ",
                ACT_INST_BACK,
                false,
                false,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 182,
                btn_y,
                btn_w,
                btn_h,
                "ถัดไป >",
                ACT_INST_NEXT,
                true,
                true,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 94,
                btn_y,
                btn_w,
                btn_h,
                "ยกเลิก",
                ACT_INST_EXIT,
                true,
                false,
            );
        }
        1 => {
            draw_wizard_button(
                hdc,
                state,
                w - 270,
                btn_y,
                btn_w,
                btn_h,
                "< ย้อนกลับ",
                ACT_INST_BACK,
                true,
                false,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 182,
                btn_y,
                btn_w,
                btn_h,
                "ติดตั้ง",
                ACT_INST_RUN_NOW,
                true,
                true,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 94,
                btn_y,
                btn_w,
                btn_h,
                "ยกเลิก",
                ACT_INST_EXIT,
                true,
                false,
            );
        }
        _ => {
            draw_wizard_button(
                hdc,
                state,
                w - 270,
                btn_y,
                btn_w,
                btn_h,
                "< ย้อนกลับ",
                ACT_INST_BACK,
                false,
                false,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 182,
                btn_y,
                btn_w,
                btn_h,
                "เสร็จสิ้น",
                ACT_INST_EXIT,
                true,
                true,
            );
            draw_wizard_button(
                hdc,
                state,
                w - 94,
                btn_y,
                btn_w,
                btn_h,
                "ยกเลิก",
                ACT_INST_EXIT,
                false,
                false,
            );
        }
    }
}

unsafe fn render_config_dc(
    hdc: HDC,
    state: &mut AppUiState,
    h_font_reg: HFONT,
    h_font_bold: HFONT,
) {
    let th = state.draft.thai_ui;
    let w = state.win_w;

    let g1 = RECT {
        left: 12,
        top: 12,
        right: w - 12,
        bottom: 94,
    };
    draw_border_rect(hdc, &g1, RGB(255, 255, 255), RGB(205, 210, 218));
    SelectObject(hdc, h_font_bold as HGDIOBJ);
    draw_text_left(
        hdc,
        22,
        16,
        w - 44,
        22,
        if th { "ทั่วไป" } else { "General" },
        RGB(14, 75, 90),
    );

    SelectObject(hdc, h_font_reg as HGDIOBJ);
    let learn = state.draft.learn_new_word;
    let show_p = state.draft.show_popup;

    draw_checkbox_row(
        hdc,
        state,
        22,
        40,
        w - 44,
        learn,
        if th {
            "เรียนรู้คำใหม่โดยอัตโนมัติ"
        } else {
            "Automatically learn new word"
        },
        ACT_CFG_TOGGLE_LEARN,
    );
    draw_checkbox_row(
        hdc,
        state,
        22,
        64,
        w - 44,
        show_p,
        if th {
            "แสดงหน้าต่างแจ้งเตือนมุมขวาล่าง"
        } else {
            "Show popup notification"
        },
        ACT_CFG_TOGGLE_POPUP,
    );

    let g2 = RECT {
        left: 12,
        top: 104,
        right: w - 12,
        bottom: 218,
    };
    draw_border_rect(hdc, &g2, RGB(255, 255, 255), RGB(205, 210, 218));
    SelectObject(hdc, h_font_bold as HGDIOBJ);
    draw_text_left(
        hdc,
        22,
        108,
        w - 44,
        22,
        if th {
            "ปุ่มลัดและภาษา"
        } else {
            "Shortcuts & Language"
        },
        RGB(14, 75, 90),
    );

    SelectObject(hdc, h_font_reg as HGDIOBJ);
    draw_text_left(
        hdc,
        22,
        134,
        185,
        24,
        if th {
            "เปลี่ยนภาษาคำล่าสุด:"
        } else {
            "Switch input buffer:"
        },
        RGB(20, 24, 32),
    );
    let buf_sc = state.draft.switch_buffer_shortcut.clone();
    draw_button(
        hdc,
        state,
        212,
        133,
        192,
        24,
        &format!("{buf_sc} ▾"),
        ACT_CFG_CYCLE_BUF_SC,
        false,
    );

    draw_text_left(
        hdc,
        22,
        162,
        185,
        24,
        if th {
            "เปลี่ยนภาษาที่เลือก:"
        } else {
            "Switch selected text:"
        },
        RGB(20, 24, 32),
    );
    let sel_sc = state.draft.switch_select_shortcut.clone();
    draw_button(
        hdc,
        state,
        212,
        161,
        192,
        24,
        &format!("{sel_sc} ▾"),
        ACT_CFG_CYCLE_SEL_SC,
        false,
    );

    draw_text_left(
        hdc,
        22,
        190,
        185,
        24,
        if th {
            "ภาษาของโปรแกรม:"
        } else {
            "UI Language:"
        },
        RGB(20, 24, 32),
    );
    draw_button(
        hdc,
        state,
        212,
        189,
        192,
        24,
        if th { "ไทย (Thai) ▾" } else { "English ▾" },
        ACT_CFG_CYCLE_UI_LANG,
        false,
    );

    if let Some(msg) = &state.status_banner {
        draw_text_left(hdc, 14, 224, w - 28, 20, msg, RGB(14, 125, 85));
    }

    draw_button(
        hdc,
        state,
        12,
        252,
        110,
        28,
        if th {
            "คืนค่าเริ่มต้น"
        } else {
            "Reset Default"
        },
        ACT_CFG_RESET,
        false,
    );
    draw_button(
        hdc,
        state,
        w - 266,
        252,
        80,
        28,
        if th { "ตกลง" } else { "OK" },
        ACT_CFG_SAVE_OK,
        true,
    );
    draw_button(
        hdc,
        state,
        w - 180,
        252,
        80,
        28,
        if th { "ยกเลิก" } else { "Cancel" },
        ACT_CLOSE_DIALOG,
        false,
    );
    draw_button(
        hdc,
        state,
        w - 94,
        252,
        80,
        28,
        if th { "นำไปใช้" } else { "Apply" },
        ACT_CFG_APPLY,
        false,
    );
}

unsafe fn show_dropdown_popup(
    hwnd: HWND,
    client_x: i32,
    client_y: i32,
    items: &[&str],
    selected_idx: usize,
) -> Option<usize> {
    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return None;
    }
    for (i, &label) in items.iter().enumerate() {
        let flags = MF_STRING | if i == selected_idx { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(hmenu, flags, i + 1, to_wstring(label).as_ptr());
    }
    let mut pt = POINT {
        x: client_x,
        y: client_y,
    };
    ClientToScreen(hwnd, &mut pt);
    let cmd = TrackPopupMenu(
        hmenu,
        TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        null_mut(),
    );
    DestroyMenu(hmenu);
    if cmd > 0 {
        Some((cmd - 1) as usize)
    } else {
        None
    }
}

unsafe fn handle_dialog_action_safe(hwnd: HWND, action_id: u32) {
    if matches!(
        action_id,
        ACT_CFG_CYCLE_BUF_SC | ACT_CFG_CYCLE_SEL_SC | ACT_CFG_CYCLE_UI_LANG
    ) {
        let cur_vals = {
            GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                g.as_ref().and_then(|arc| {
                    arc.lock().ok().map(|st| {
                        (
                            st.draft.switch_buffer_shortcut.clone(),
                            st.draft.switch_select_shortcut.clone(),
                            st.draft.thai_ui,
                        )
                    })
                })
            })
        };

        let Some((cur_buf, cur_sel, cur_th)) = cur_vals else {
            return;
        };

        match action_id {
            ACT_CFG_CYCLE_BUF_SC => {
                let choices = ["Shift + Backspace", "Ctrl + Backspace", "Pause"];
                let sel = choices.iter().position(|&s| s == cur_buf).unwrap_or(0);
                if let Some(chosen) = show_dropdown_popup(hwnd, 212, 157, &choices, sel) {
                    if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                        if let Some(arc) = guard.as_ref() {
                            if let Ok(mut st) = arc.lock() {
                                st.draft.switch_buffer_shortcut = choices[chosen].to_string();
                                st.status_banner = None;
                            }
                        }
                    }
                    InvalidateRect(hwnd, null_mut(), 1);
                }
            }
            ACT_CFG_CYCLE_SEL_SC => {
                let choices = ["Shift + Caps Lock", "Scroll Lock"];
                let sel = choices.iter().position(|&s| s == cur_sel).unwrap_or(0);
                if let Some(chosen) = show_dropdown_popup(hwnd, 212, 185, &choices, sel) {
                    if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                        if let Some(arc) = guard.as_ref() {
                            if let Ok(mut st) = arc.lock() {
                                st.draft.switch_select_shortcut = choices[chosen].to_string();
                                st.status_banner = None;
                            }
                        }
                    }
                    InvalidateRect(hwnd, null_mut(), 1);
                }
            }
            ACT_CFG_CYCLE_UI_LANG => {
                let choices = ["ไทย (Thai)", "English"];
                let sel = if cur_th { 0 } else { 1 };
                if let Some(chosen) = show_dropdown_popup(hwnd, 212, 213, &choices, sel) {
                    if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                        if let Some(arc) = guard.as_ref() {
                            if let Ok(mut st) = arc.lock() {
                                st.draft.thai_ui = chosen == 0;
                                st.status_banner = None;
                            }
                        }
                    }
                    InvalidateRect(hwnd, null_mut(), 1);
                }
            }
            _ => {}
        }
        return;
    }

    enum PostAction {
        None,
        Repaint,
        CloseDialog,
        UpdateTrayAndTitle(CorrectionMode, bool, &'static str, bool),
        Quit,
    }

    let post = {
        let arc_opt = GLOBAL_APP_STATE
            .lock()
            .ok()
            .and_then(|g| g.as_ref().cloned());
        let Some(arc) = arc_opt else { return };
        let Ok(mut st) = arc.lock() else { return };

        match action_id {
            ACT_CLOSE_DIALOG => {
                st.draft = ConfigDraft::from_engine(&st.engine);
                st.status_banner = None;
                PostAction::CloseDialog
            }
            ACT_INST_EXIT => PostAction::Quit,
            ACT_INST_NEXT => {
                if st.inst_step < 1 {
                    st.inst_step += 1;
                    st.status_banner = None;
                }
                PostAction::Repaint
            }
            ACT_INST_BACK => {
                if st.inst_step == 1 {
                    st.inst_step = 0;
                    st.status_banner = None;
                }
                PostAction::Repaint
            }
            ACT_INST_TOGGLE_DESKTOP => {
                st.inst_desktop_shortcut = !st.inst_desktop_shortcut;
                PostAction::Repaint
            }
            ACT_INST_TOGGLE_AUTOSTART => {
                st.inst_autostart = !st.inst_autostart;
                PostAction::Repaint
            }
            ACT_INST_TOGGLE_LAUNCH => {
                st.inst_launch_now = !st.inst_launch_now;
                PostAction::Repaint
            }
            ACT_INST_RUN_NOW => {
                match perform_self_install_custom(
                    st.inst_desktop_shortcut,
                    st.inst_autostart,
                    st.inst_launch_now,
                ) {
                    Ok(_) => {
                        st.inst_step = 2;
                        st.status_banner = None;
                    }
                    Err(e) => {
                        st.status_banner = Some(format!("เกิดข้อผิดพลาด: {e}"));
                    }
                }
                PostAction::Repaint
            }
            ACT_CFG_TOGGLE_LEARN => {
                st.draft.learn_new_word = !st.draft.learn_new_word;
                st.status_banner = None;
                PostAction::Repaint
            }
            ACT_CFG_TOGGLE_POPUP => {
                st.draft.show_popup = !st.draft.show_popup;
                st.status_banner = None;
                PostAction::Repaint
            }
            ACT_CFG_RESET => {
                st.draft = ConfigDraft::defaults();
                st.status_banner = Some(if st.draft.thai_ui {
                    "↺ คืนค่าเริ่มต้นแล้ว (กด ตกลง หรือ นำไปใช้ เพื่อบันทึก)".to_string()
                } else {
                    "↺ Reset to defaults (click OK or Apply to save)".to_string()
                });
                PostAction::Repaint
            }
            ACT_CFG_APPLY => {
                let d = st.draft.clone();
                d.apply_to_engine(&mut st.engine);
                let _ = st.engine.save_all_user_settings();
                st.status_banner = Some(if st.engine.thai_ui {
                    "✓ บันทึกการตั้งค่าเรียบร้อยแล้ว".to_string()
                } else {
                    "✓ Settings applied and saved.".to_string()
                });
                PostAction::UpdateTrayAndTitle(
                    st.engine.mode,
                    st.engine.thai_ui,
                    view_title(st.active_view, st.engine.thai_ui),
                    false,
                )
            }
            ACT_CFG_SAVE_OK => {
                let d = st.draft.clone();
                d.apply_to_engine(&mut st.engine);
                let _ = st.engine.save_all_user_settings();
                st.status_banner = None;
                PostAction::UpdateTrayAndTitle(
                    st.engine.mode,
                    st.engine.thai_ui,
                    view_title(st.active_view, st.engine.thai_ui),
                    true,
                )
            }
            _ => PostAction::None,
        }
    };

    match post {
        PostAction::None => {}
        PostAction::Repaint => {
            InvalidateRect(hwnd, null_mut(), 1);
        }
        PostAction::CloseDialog => close_dialog_to_tray_safe(hwnd),
        PostAction::UpdateTrayAndTitle(mode, th, title, close_after) => {
            SetWindowTextW(hwnd, to_wstring(title).as_ptr());
            add_or_update_tray_icon(hwnd, mode, th, false);
            if close_after {
                close_dialog_to_tray_safe(hwnd);
            } else {
                InvalidateRect(hwnd, null_mut(), 1);
            }
        }
        PostAction::Quit => {
            PostQuitMessage(0);
        }
    }
}

unsafe fn show_toast_notification(toast_hwnd: HWND, opacity: f32) {
    if toast_hwnd.is_null() {
        return;
    }
    let (x, y) = bottom_right_workarea_pos(250, 90);
    SetLayeredWindowAttributes(toast_hwnd, 0, (opacity * 255.0) as u8, LWA_ALPHA);
    SetWindowPos(
        toast_hwnd,
        HWND_TOPMOST,
        x,
        y,
        250,
        90,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    InvalidateRect(toast_hwnd, null_mut(), 1);
    SetTimer(toast_hwnd, TIMER_TOAST_FADE, 80, None);
}

unsafe extern "system" fn toast_wnd_proc(
    hwnd: HWND,
    msg: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if w_param == TIMER_TOAST_FADE => {
            let (hide, new_alpha) = {
                let mut h_flag = true;
                let mut alpha_opt = None;
                if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                    if let Some(arc) = guard.as_ref() {
                        if let Ok(mut st) = arc.lock() {
                            if let Some(notif) = &mut st.engine.notification {
                                if notif.remaining_ms > 0.0 {
                                    notif.remaining_ms -= 80.0;
                                    h_flag = false;
                                } else {
                                    notif.opacity -= 0.08;
                                    if notif.opacity > 0.08 {
                                        alpha_opt = Some((notif.opacity * 255.0) as u8);
                                        h_flag = false;
                                    }
                                }
                            }
                            if h_flag {
                                st.engine.notification = None;
                            }
                        }
                    }
                }
                (h_flag, alpha_opt)
            };

            if let Some(a) = new_alpha {
                SetLayeredWindowAttributes(hwnd, 0, a, LWA_ALPHA);
            }
            if hide {
                KillTimer(hwnd, TIMER_TOAST_FADE);
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (l_param & 0xFFFF) as i16 as i32;
            let y = ((l_param >> 16) & 0xFFFF) as i16 as i32;
            let clicked_undo = {
                let mut u = false;
                if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                    if let Some(arc) = guard.as_ref() {
                        if let Ok(mut st) = arc.lock() {
                            if pt_in_rect((x, y), &st.toast_undo_rect) {
                                u = true;
                            } else if pt_in_rect((x, y), &st.toast_mute_rect) {
                                st.engine.show_popup = false;
                                st.draft.show_popup = false;
                                let _ = st.engine.save_all_user_settings();
                            }
                        }
                    }
                }
                u
            };
            if clicked_undo {
                if let Ok(guard) = HOOK_SENDER.lock() {
                    if let Some(tx) = guard.as_ref() {
                        let fg = GetForegroundWindow() as usize;
                        let _ = tx.send(HookEvent::SwitchBufferShortcut { hwnd: fg });
                    }
                }
            }
            KillTimer(hwnd, TIMER_TOAST_FADE);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let rc = RECT {
                left: 0,
                top: 0,
                right: 250,
                bottom: 90,
            };
            draw_border_rect(hdc, &rc, RGB(255, 255, 204), RGB(165, 155, 95));
            SetBkMode(hdc, TRANSPARENT as i32);

            let hf_bold = create_ui_font(14, true);
            let hf_reg = create_ui_font(15, false);
            let hf_sm = create_ui_font(12, false);
            let old = SelectObject(hdc, hf_bold as HGDIOBJ);

            let snapshot = {
                GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                    g.as_ref().and_then(|arc| {
                        arc.lock().ok().and_then(|mut st| {
                            st.toast_undo_rect = RECT {
                                left: 14,
                                top: 60,
                                right: 116,
                                bottom: 82,
                            };
                            st.toast_mute_rect = RECT {
                                left: 124,
                                top: 60,
                                right: 238,
                                bottom: 82,
                            };
                            st.engine
                                .notification
                                .clone()
                                .map(|n| (st.engine.thai_ui, st.engine.mode, n))
                        })
                    })
                })
            };

            if let Some((th, mode, notif)) = snapshot {
                draw_keyswitch_badge(hdc, 20, 19, 10, mode);
                draw_text_left(hdc, 36, 8, 206, 22, notif.kind.title(th), RGB(14, 75, 90));
                SelectObject(hdc, hf_reg as HGDIOBJ);
                draw_text_left(hdc, 14, 32, 222, 24, &notif.message, RGB(15, 15, 15));

                SelectObject(hdc, hf_sm as HGDIOBJ);
                let undo_rc = RECT {
                    left: 14,
                    top: 60,
                    right: 116,
                    bottom: 82,
                };
                draw_border_rect(hdc, &undo_rc, RGB(246, 242, 180), RGB(175, 165, 110));
                draw_text_center(
                    hdc,
                    &undo_rc,
                    if th { "ยกเลิก (Undo)" } else { "Undo" },
                    RGB(14, 90, 145),
                );
                let mute_rc = RECT {
                    left: 124,
                    top: 60,
                    right: 238,
                    bottom: 82,
                };
                draw_text_center(
                    hdc,
                    &mute_rc,
                    if th {
                        "ไม่ต้องแจ้งเตือนอีก"
                    } else {
                        "Don't show again"
                    },
                    RGB(110, 102, 70),
                );
            }

            SelectObject(hdc, old);
            DeleteObject(hf_bold as HGDIOBJ);
            DeleteObject(hf_reg as HGDIOBJ);
            DeleteObject(hf_sm as HGDIOBJ);
            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: UINT,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            let ev = (l_param & 0xFFFF) as u32;
            if ev == WM_LBUTTONUP || ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                let snap = {
                    GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                        g.as_ref().and_then(|arc| {
                            arc.lock()
                                .ok()
                                .map(|st| (st.engine.thai_ui, st.engine.mode, st.engine.auto_start))
                        })
                    })
                };
                if let Some((th, mode, auto_start)) = snap {
                    show_native_tray_popup_menu(hwnd, th, mode, auto_start);
                }
            } else if ev == WM_LBUTTONDBLCLK {
                open_dialog_view_safe(hwnd, ActiveView::ConfigForm);
            }
            0
        }
        WM_COMMAND => {
            let cmd_id = w_param & 0xFFFF;
            match cmd_id {
                CMD_MODE_ONLINE | CMD_MODE_OFFLINE | CMD_MODE_DISABLED => {
                    let new_mode = match cmd_id {
                        CMD_MODE_ONLINE => CorrectionMode::Online,
                        CMD_MODE_OFFLINE => CorrectionMode::Offline,
                        _ => CorrectionMode::Disabled,
                    };
                    let th = {
                        let mut thai = true;
                        if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                            if let Some(arc) = guard.as_ref() {
                                if let Ok(mut st) = arc.lock() {
                                    st.engine.mode = new_mode;
                                    thai = st.engine.thai_ui;
                                }
                            }
                        }
                        thai
                    };
                    add_or_update_tray_icon(hwnd, new_mode, th, false);
                }
                CMD_TOGGLE_AUTOSTART => {
                    let new_as = {
                        let mut v = true;
                        if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                            if let Some(arc) = guard.as_ref() {
                                if let Ok(mut st) = arc.lock() {
                                    st.engine.auto_start = !st.engine.auto_start;
                                    v = st.engine.auto_start;
                                }
                            }
                        }
                        v
                    };
                    apply_windows_autostart_registry(new_as);
                }
                CMD_TOGGLE_UI_LANG => {
                    let snap = {
                        GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                            g.as_ref().and_then(|arc| {
                                arc.lock().ok().map(|mut st| {
                                    st.engine.thai_ui = !st.engine.thai_ui;
                                    st.draft.thai_ui = st.engine.thai_ui;
                                    let _ = st.engine.save_all_user_settings();
                                    (
                                        st.engine.mode,
                                        st.engine.thai_ui,
                                        view_title(st.active_view, st.engine.thai_ui),
                                    )
                                })
                            })
                        })
                    };
                    if let Some((mode, th, title)) = snap {
                        add_or_update_tray_icon(hwnd, mode, th, false);
                        SetWindowTextW(hwnd, to_wstring(title).as_ptr());
                        InvalidateRect(hwnd, null_mut(), 1);
                    }
                }
                CMD_OPEN_CONFIG => open_dialog_view_safe(hwnd, ActiveView::ConfigForm),
                CMD_EXIT => {
                    remove_tray_icon(hwnd);
                    PostQuitMessage(0);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            close_dialog_to_tray_safe(hwnd);
            0
        }
        WM_ENGINE_NOTIFY => {
            let snap = {
                GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                    g.as_ref().and_then(|arc| {
                        arc.lock().ok().map(|st| {
                            (
                                st.engine.mode,
                                st.engine.thai_ui,
                                st.engine.notification.as_ref().map(|n| n.opacity),
                            )
                        })
                    })
                })
            };
            if let Some((mode, th, opacity_opt)) = snap {
                add_or_update_tray_icon(hwnd, mode, th, false);
                if let Some(op) = opacity_opt {
                    if let Ok(t_slot) = TOAST_HWND.lock() {
                        if *t_slot != 0 {
                            show_toast_notification(*t_slot as HWND, op);
                        }
                    }
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            let x = (l_param & 0xFFFF) as i16 as i32;
            let y = ((l_param >> 16) & 0xFFFF) as i16 as i32;
            let changed = {
                let mut ch = false;
                if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                    if let Some(arc) = guard.as_ref() {
                        if let Ok(mut st) = arc.lock() {
                            if st.hover_pos != (x, y) {
                                st.hover_pos = (x, y);
                                ch = true;
                            }
                        }
                    }
                }
                ch
            };
            if changed {
                InvalidateRect(hwnd, null_mut(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (l_param & 0xFFFF) as i16 as i32;
            let y = ((l_param >> 16) & 0xFFFF) as i16 as i32;
            let hit_action = {
                let mut hit = None;
                if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                    if let Some(arc) = guard.as_ref() {
                        if let Ok(st) = arc.lock() {
                            for reg in &st.regions {
                                if pt_in_rect((x, y), &reg.rect) {
                                    hit = Some(reg.action_id);
                                    break;
                                }
                            }
                        }
                    }
                }
                hit
            };
            if let Some(act) = hit_action {
                handle_dialog_action_safe(hwnd, act);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Ok(guard) = GLOBAL_APP_STATE.lock() {
                if let Some(arc) = guard.as_ref() {
                    if let Ok(mut st) = arc.lock() {
                        let w = st.win_w;
                        let h = st.win_h;
                        let mem_dc = CreateCompatibleDC(hdc);
                        let mut bmi: BITMAPINFO = std::mem::zeroed();
                        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                        bmi.bmiHeader.biWidth = w;
                        bmi.bmiHeader.biHeight = -h;
                        bmi.bmiHeader.biPlanes = 1;
                        bmi.bmiHeader.biBitCount = 32;
                        bmi.bmiHeader.biCompression = BI_RGB;
                        let mut bits: *mut winapi::ctypes::c_void = null_mut();
                        let h_bm = CreateDIBSection(
                            hdc,
                            &bmi,
                            DIB_RGB_COLORS,
                            &mut bits,
                            null_mut(),
                            0,
                        );
                        let old_bm = SelectObject(mem_dc, h_bm as HGDIOBJ);

                        render_ui_to_dc(mem_dc, &mut st);
                        BitBlt(hdc, 0, 0, w, h, mem_dc, 0, 0, SRCCOPY);

                        SelectObject(mem_dc, old_bm);
                        DeleteObject(h_bm as HGDIOBJ);
                        DeleteDC(mem_dc);
                    }
                }
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_TIMER if w_param == TIMER_TRAY_RETRY => {
            let snap = {
                GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                    g.as_ref().and_then(|arc| {
                        arc.lock()
                            .ok()
                            .map(|st| (st.engine.mode, st.engine.thai_ui, st.is_installer_process))
                    })
                })
            };
            if let Some((mode, th, is_inst)) = snap {
                if is_inst || add_or_update_tray_icon(hwnd, mode, th, true) {
                    KillTimer(hwnd, TIMER_TRAY_RETRY);
                }
            }
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_TRAY_RETRY);
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => {
            let tbc = WM_TASKBARCREATED.load(Ordering::Relaxed);
            if tbc != 0 && msg == tbc {
                let snap = {
                    GLOBAL_APP_STATE.lock().ok().and_then(|g| {
                        g.as_ref().and_then(|arc| {
                            arc.lock().ok().map(|st| {
                                (st.engine.mode, st.engine.thai_ui, st.is_installer_process)
                            })
                        })
                    })
                };
                if let Some((mode, th, is_inst)) = snap {
                    if !is_inst && !add_or_update_tray_icon(hwnd, mode, th, true) {
                        SetTimer(hwnd, TIMER_TRAY_RETRY, 1000, None);
                    }
                }
                return 0;
            }
            DefWindowProcW(hwnd, msg, w_param, l_param)
        }
    }
}

unsafe fn get_focused_hwnd(fg: HWND) -> HWND {
    let tid = if fg.is_null() {
        0
    } else {
        GetWindowThreadProcessId(fg, null_mut())
    };
    let mut gti: GUITHREADINFO = std::mem::zeroed();
    gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if GetGUIThreadInfo(tid, &mut gti) != 0 && !gti.hwndFocus.is_null() {
        return gti.hwndFocus;
    }
    fg
}

unsafe fn query_foreground_process_name(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    let mut pid: DWORD = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 {
        return String::new();
    }
    let h_proc = winapi::um::processthreadsapi::OpenProcess(
        winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION
            | winapi::um::winnt::PROCESS_VM_READ,
        0,
        pid,
    );
    if h_proc.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 260];
    let len = winapi::um::psapi::GetModuleBaseNameW(
        h_proc,
        null_mut(),
        buf.as_mut_ptr(),
        buf.len() as u32,
    );
    winapi::um::handleapi::CloseHandle(h_proc);
    if len > 0 {
        Path::new(&String::from_utf16_lossy(&buf[..len as usize]))
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

unsafe fn detect_window_input_language(hwnd: HWND) -> Language {
    let focus = get_focused_hwnd(hwnd);
    let tid = if !focus.is_null() {
        GetWindowThreadProcessId(focus, null_mut())
    } else if !hwnd.is_null() {
        GetWindowThreadProcessId(hwnd, null_mut())
    } else {
        0
    };
    let hkl = GetKeyboardLayout(tid);
    let lang_id = (hkl as usize) & 0xFFFF;
    let primary_lang = lang_id & 0x3FF;
    if primary_lang == 0x1E {
        Language::Thai
    } else {
        Language::English
    }
}

unsafe fn find_installed_hkl(target_lang: Language) -> LPARAM {
    let target_primary: usize = match target_lang {
        Language::Thai => 0x1E,
        Language::English => 0x09,
    };
    let mut list: [HKL; 16] = [null_mut(); 16];
    let count = GetKeyboardLayoutList(list.len() as i32, list.as_mut_ptr());
    if count > 0 {
        for &hkl in &list[..(count as usize).min(list.len())] {
            let primary = ((hkl as usize) & 0xFFFF) & 0x3FF;
            if primary == target_primary {
                return hkl as LPARAM;
            }
        }
    }
    match target_lang {
        Language::Thai => 0x041E_041E as LPARAM,
        Language::English => 0x0409_0409 as LPARAM,
    }
}

fn read_windows_lang_hotkey() -> u8 {
    static CACHED: OnceLock<u8> = OnceLock::new();
    *CACHED.get_or_init(|| {
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Keyboard Layout\Toggle", "/v", "Language Hotkey"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout);
            for tok in s.split_whitespace().rev() {
                if let Ok(v) = tok.parse::<u8>() {
                    return v;
                }
            }
        }
        4
    })
}

unsafe fn switch_windows_input_language(hwnd: HWND, target_lang: Language) {
    if detect_window_input_language(hwnd) == target_lang {
        return;
    }

    let target_hkl = find_installed_hkl(target_lang);
    let focus = get_focused_hwnd(hwnd);

    if !focus.is_null() {
        PostMessageW(focus, WM_INPUTLANGCHANGEREQUEST, 0, target_hkl);
    }
    if !hwnd.is_null() && hwnd != focus {
        PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, target_hkl);
    }
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        if detect_window_input_language(hwnd) == target_lang {
            return;
        }
    }

    let hotkey = read_windows_lang_hotkey();
    let mut toggle_inputs: Vec<INPUT> = Vec::with_capacity(4);
    match hotkey {
        4 => {
            let mut d: INPUT = std::mem::zeroed();
            d.type_ = INPUT_KEYBOARD;
            d.u.ki_mut().wVk = VK_OEM_3 as u16;
            toggle_inputs.push(d);
            let mut u: INPUT = std::mem::zeroed();
            u.type_ = INPUT_KEYBOARD;
            u.u.ki_mut().wVk = VK_OEM_3 as u16;
            u.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
            toggle_inputs.push(u);
        }
        1 => {
            for (vk, up) in [
                (VK_LMENU, false),
                (VK_LSHIFT, false),
                (VK_LSHIFT, true),
                (VK_LMENU, true),
            ] {
                let mut inp: INPUT = std::mem::zeroed();
                inp.type_ = INPUT_KEYBOARD;
                inp.u.ki_mut().wVk = vk as u16;
                if up {
                    inp.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
                }
                toggle_inputs.push(inp);
            }
        }
        2 => {
            for (vk, up) in [
                (VK_LCONTROL, false),
                (VK_LSHIFT, false),
                (VK_LSHIFT, true),
                (VK_LCONTROL, true),
            ] {
                let mut inp: INPUT = std::mem::zeroed();
                inp.type_ = INPUT_KEYBOARD;
                inp.u.ki_mut().wVk = vk as u16;
                if up {
                    inp.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
                }
                toggle_inputs.push(inp);
            }
        }
        _ => {
            for (vk, up) in [
                (VK_LWIN, false),
                (VK_SPACE, false),
                (VK_SPACE, true),
                (VK_LWIN, true),
            ] {
                let mut inp: INPUT = std::mem::zeroed();
                inp.type_ = INPUT_KEYBOARD;
                inp.u.ki_mut().wVk = vk as u16;
                if up {
                    inp.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
                }
                toggle_inputs.push(inp);
            }
        }
    }
    if !toggle_inputs.is_empty() {
        SendInput(
            toggle_inputs.len() as u32,
            toggle_inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            if detect_window_input_language(hwnd) == target_lang {
                return;
            }
        }
    }
}

unsafe fn execute_os_replacement(hwnd: HWND, rep: &PendingOsReplacement) {
    IS_INJECTING.store(true, Ordering::SeqCst);

    std::thread::sleep(std::time::Duration::from_millis(15));

    let mut inputs: Vec<INPUT> = Vec::with_capacity(rep.backspaces * 2 + 4);
    for vk_mod in [VK_SHIFT, VK_CONTROL] {
        if (GetAsyncKeyState(vk_mod) as u16 & 0x8000) != 0 {
            let mut up: INPUT = std::mem::zeroed();
            up.type_ = INPUT_KEYBOARD;
            up.u.ki_mut().wVk = vk_mod as u16;
            up.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
            inputs.push(up);
        }
    }
    for _ in 0..rep.backspaces {
        let mut down: INPUT = std::mem::zeroed();
        down.type_ = INPUT_KEYBOARD;
        down.u.ki_mut().wVk = VK_BACK as u16;
        inputs.push(down);

        let mut up: INPUT = std::mem::zeroed();
        up.type_ = INPUT_KEYBOARD;
        up.u.ki_mut().wVk = VK_BACK as u16;
        up.u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
        inputs.push(up);
    }
    if !inputs.is_empty() {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }

    switch_windows_input_language(hwnd, rep.target_lang);

    let mut text_inputs: Vec<INPUT> = Vec::with_capacity(rep.replacement_text.len() * 2);
    for u16_unit in rep.replacement_text.encode_utf16() {
        let mut down: INPUT = std::mem::zeroed();
        down.type_ = INPUT_KEYBOARD;
        down.u.ki_mut().wScan = u16_unit;
        down.u.ki_mut().dwFlags = KEYEVENTF_UNICODE;
        text_inputs.push(down);

        let mut up: INPUT = std::mem::zeroed();
        up.type_ = INPUT_KEYBOARD;
        up.u.ki_mut().wScan = u16_unit;
        up.u.ki_mut().dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;
        text_inputs.push(up);
    }
    if !text_inputs.is_empty() {
        SendInput(
            text_inputs.len() as u32,
            text_inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }

    std::thread::sleep(std::time::Duration::from_millis(12));
    IS_INJECTING.store(false, Ordering::SeqCst);
}

unsafe fn set_clipboard_unicode_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_len = wide.len() * 2;
    let h_mem = winapi::um::winbase::GlobalAlloc(winapi::um::winbase::GMEM_MOVEABLE, byte_len);
    if h_mem.is_null() {
        return false;
    }
    let ptr = winapi::um::winbase::GlobalLock(h_mem) as *mut u16;
    if ptr.is_null() {
        winapi::um::winbase::GlobalFree(h_mem);
        return false;
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
    winapi::um::winbase::GlobalUnlock(h_mem);

    if OpenClipboard(null_mut()) != 0 {
        EmptyClipboard();
        SetClipboardData(CF_UNICODETEXT, h_mem);
        CloseClipboard();
        true
    } else {
        winapi::um::winbase::GlobalFree(h_mem);
        false
    }
}

unsafe fn execute_switch_selected_text(hwnd: HWND, state_arc: &Arc<Mutex<AppUiState>>) {
    IS_INJECTING.store(true, Ordering::SeqCst);

    let seq_before = GetClipboardSequenceNumber();

    let mut copy_inputs: [INPUT; 5] = std::mem::zeroed();
    copy_inputs[0].type_ = INPUT_KEYBOARD;
    copy_inputs[0].u.ki_mut().wVk = VK_SHIFT as u16;
    copy_inputs[0].u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
    copy_inputs[1].type_ = INPUT_KEYBOARD;
    copy_inputs[1].u.ki_mut().wVk = VK_CONTROL as u16;
    copy_inputs[2].type_ = INPUT_KEYBOARD;
    copy_inputs[2].u.ki_mut().wVk = b'C' as u16;
    copy_inputs[3].type_ = INPUT_KEYBOARD;
    copy_inputs[3].u.ki_mut().wVk = b'C' as u16;
    copy_inputs[3].u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
    copy_inputs[4].type_ = INPUT_KEYBOARD;
    copy_inputs[4].u.ki_mut().wVk = VK_CONTROL as u16;
    copy_inputs[4].u.ki_mut().dwFlags = KEYEVENTF_KEYUP;

    SendInput(
        copy_inputs.len() as u32,
        copy_inputs.as_mut_ptr(),
        std::mem::size_of::<INPUT>() as i32,
    );

    let t0 = Instant::now();
    while t0.elapsed().as_millis() < 450 {
        if GetClipboardSequenceNumber() != seq_before {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    let mut converted_opt: Option<(String, String)> = None;
    if OpenClipboard(null_mut()) != 0 {
        let h_data = GetClipboardData(CF_UNICODETEXT);
        if !h_data.is_null() {
            let ptr = winapi::um::winbase::GlobalLock(h_data) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 65536 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(ptr, len);
                let orig = String::from_utf16_lossy(slice);
                winapi::um::winbase::GlobalUnlock(h_data);
                if !orig.is_empty() {
                    let conv = engine::convert_string_language(&orig);
                    converted_opt = Some((orig, conv));
                }
            }
        }
        CloseClipboard();
    }

    if let Some((orig, conv)) = converted_opt {
        if set_clipboard_unicode_text(&conv) {
            std::thread::sleep(std::time::Duration::from_millis(40));
            let mut paste_inputs: [INPUT; 4] = std::mem::zeroed();
            paste_inputs[0].type_ = INPUT_KEYBOARD;
            paste_inputs[0].u.ki_mut().wVk = VK_CONTROL as u16;
            paste_inputs[1].type_ = INPUT_KEYBOARD;
            paste_inputs[1].u.ki_mut().wVk = b'V' as u16;
            paste_inputs[2].type_ = INPUT_KEYBOARD;
            paste_inputs[2].u.ki_mut().wVk = b'V' as u16;
            paste_inputs[2].u.ki_mut().dwFlags = KEYEVENTF_KEYUP;
            paste_inputs[3].type_ = INPUT_KEYBOARD;
            paste_inputs[3].u.ki_mut().wVk = VK_CONTROL as u16;
            paste_inputs[3].u.ki_mut().dwFlags = KEYEVENTF_KEYUP;

            SendInput(
                paste_inputs.len() as u32,
                paste_inputs.as_mut_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );

            let target_lang = if conv.chars().any(|c| (c as u32) >= 0x0E00 && (c as u32) <= 0x0E7F) {
                Language::Thai
            } else {
                Language::English
            };
            switch_windows_input_language(hwnd, target_lang);

            if let Ok(mut st) = state_arc.lock() {
                if st.engine.show_popup {
                    st.engine.notification = Some(NotificationBanner {
                        kind: engine::NotificationKind::ManualCorrection,
                        message: format!("{orig} » {conv}"),
                        opacity: 0.92,
                        remaining_ms: 2600.0,
                    });
                }
            }
            if let Ok(m_slot) = MAIN_HWND.lock() {
                if *m_slot != 0 {
                    PostMessageW(*m_slot as HWND, WM_ENGINE_NOTIFY, 0, 0);
                }
            }
        }
    }

    std::thread::sleep(std::time::Duration::from_millis(15));
    IS_INJECTING.store(false, Ordering::SeqCst);
}

fn start_keyboard_worker_and_hook(state_arc: Arc<Mutex<AppUiState>>) {
    let (tx, rx) = mpsc::channel::<HookEvent>();
    if let Ok(mut slot) = HOOK_SENDER.lock() {
        *slot = Some(tx);
    }

    let worker_state = Arc::clone(&state_arc);
    std::thread::spawn(move || unsafe {
        let mut last_fg: usize = 0;
        let mut last_switch_at: Option<(Instant, Language)> = None;

        while let Ok(ev) = rx.recv() {
            match ev {
                HookEvent::PrintableKey { us_char, hwnd } => {
                    let fg = hwnd as HWND;
                    let proc_name = query_foreground_process_name(fg);
                    let os_lang = match last_switch_at {
                        Some((t, lang)) if t.elapsed().as_millis() < 120 => lang,
                        _ => detect_window_input_language(fg),
                    };
                    let rep_opt = if let Ok(mut st) = worker_state.lock() {
                        if hwnd != last_fg {
                            st.engine.clear_buffers();
                            last_fg = hwnd;
                        }
                        st.engine.set_active_process(&proc_name);
                        st.engine.active_language = os_lang;
                        st.engine.process_keystroke(us_char)
                    } else {
                        None
                    };
                    if let Some(rep) = rep_opt {
                        let target = rep.target_lang;
                        execute_os_replacement(fg, &rep);
                        last_switch_at = Some((Instant::now(), target));
                        if let Ok(m_slot) = MAIN_HWND.lock() {
                            if *m_slot != 0 {
                                PostMessageW(*m_slot as HWND, WM_ENGINE_NOTIFY, 0, 0);
                            }
                        }
                    }
                }
                HookEvent::Boundary { ch, hwnd } => {
                    let fg = hwnd as HWND;
                    let rep_opt = if let Ok(mut st) = worker_state.lock() {
                        st.engine.process_keystroke(ch)
                    } else {
                        None
                    };
                    if let Some(rep) = rep_opt {
                        let target = rep.target_lang;
                        execute_os_replacement(fg, &rep);
                        last_switch_at = Some((Instant::now(), target));
                        if let Ok(m_slot) = MAIN_HWND.lock() {
                            if *m_slot != 0 {
                                PostMessageW(*m_slot as HWND, WM_ENGINE_NOTIFY, 0, 0);
                            }
                        }
                    }
                }
                HookEvent::Backspace => {
                    if let Ok(mut st) = worker_state.lock() {
                        st.engine.handle_backspace();
                    }
                }
                HookEvent::ClearBuffer => {
                    if let Ok(mut st) = worker_state.lock() {
                        st.engine.clear_buffers();
                    }
                }
                HookEvent::SwitchBufferShortcut { hwnd } => {
                    let fg = hwnd as HWND;
                    let rep_opt = if let Ok(mut st) = worker_state.lock() {
                        st.engine.trigger_shift_backspace()
                    } else {
                        None
                    };
                    if let Some(rep) = rep_opt {
                        let target = rep.target_lang;
                        execute_os_replacement(fg, &rep);
                        last_switch_at = Some((Instant::now(), target));
                        if let Ok(m_slot) = MAIN_HWND.lock() {
                            if *m_slot != 0 {
                                PostMessageW(*m_slot as HWND, WM_ENGINE_NOTIFY, 0, 0);
                            }
                        }
                    }
                }
                HookEvent::SwitchSelectedShortcut { hwnd } => {
                    execute_switch_selected_text(hwnd as HWND, &worker_state);
                }
                HookEvent::ToggleMode => {
                    if let Ok(mut st) = worker_state.lock() {
                        st.engine.toggle_online_offline_mode();
                    }
                    if let Ok(m_slot) = MAIN_HWND.lock() {
                        if *m_slot != 0 {
                            PostMessageW(*m_slot as HWND, WM_ENGINE_NOTIFY, 0, 0);
                        }
                    }
                }
            }
        }
    });

    std::thread::spawn(|| unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_kbd_proc), null_mut(), 0);
        if !hook.is_null() {
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            UnhookWindowsHookEx(hook);
        }
    });
}

unsafe extern "system" fn low_level_kbd_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0
        && !IS_INJECTING.load(Ordering::Relaxed)
        && (w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize)
    {
        let kb = &*(l_param as *const KBDLLHOOKSTRUCT);
        if (kb.flags & 0x10) == 0 {
            let shift_down = (GetAsyncKeyState(VK_SHIFT) as u16 & 0x8000) != 0;
            let ctrl_down = (GetAsyncKeyState(VK_CONTROL) as u16 & 0x8000) != 0;
            let alt_down = (GetAsyncKeyState(VK_MENU) as u16 & 0x8000) != 0;
            let caps_on = (GetKeyState(VK_CAPITAL) & 1) != 0;
            let vk = kb.vkCode;
            let fg_hwnd = GetForegroundWindow() as usize;

            let mouse_clicked =
                ((GetAsyncKeyState(VK_LBUTTON) as u16) & 1) != 0
                    || ((GetAsyncKeyState(VK_RBUTTON) as u16) & 1) != 0;

            if let Ok(guard) = HOOK_SENDER.lock() {
                if let Some(tx) = guard.as_ref() {
                    if mouse_clicked {
                        let _ = tx.send(HookEvent::ClearBuffer);
                    }

                    if ctrl_down && !shift_down && !alt_down && vk == VK_CAPITAL as u32 {
                        let _ = tx.send(HookEvent::ToggleMode);
                        return 1;
                    }
                    let buf_sc = ACTIVE_BUF_SC.load(Ordering::Relaxed);
                    let sel_sc = ACTIVE_SEL_SC.load(Ordering::Relaxed);
                    let hit_buf = match buf_sc {
                        1 => ctrl_down && !shift_down && vk == VK_BACK as u32,
                        2 => vk == VK_PAUSE as u32,
                        _ => shift_down && !ctrl_down && vk == VK_BACK as u32,
                    };
                    if hit_buf {
                        let _ = tx.send(HookEvent::SwitchBufferShortcut { hwnd: fg_hwnd });
                        return 1;
                    }
                    let hit_sel = match sel_sc {
                        1 => vk == VK_SCROLL as u32,
                        _ => shift_down && !ctrl_down && vk == VK_CAPITAL as u32,
                    };
                    if hit_sel {
                        let _ = tx.send(HookEvent::SwitchSelectedShortcut { hwnd: fg_hwnd });
                        return 1;
                    }

                    if let Ok(m_slot) = MAIN_HWND.lock() {
                        if *m_slot != 0 && fg_hwnd == *m_slot {
                            return CallNextHookEx(null_mut(), n_code, w_param, l_param);
                        }
                    }

                    if ctrl_down || alt_down {
                        let _ = tx.send(HookEvent::ClearBuffer);
                    } else if vk == VK_OEM_3 as u32 && !shift_down && read_windows_lang_hotkey() == 4
                    {
                        let _ = tx.send(HookEvent::ClearBuffer);
                    } else if vk == VK_BACK as u32 {
                        let _ = tx.send(HookEvent::Backspace);
                    } else if vk == VK_SPACE as u32 {
                        let _ = tx.send(HookEvent::Boundary {
                            ch: ' ',
                            hwnd: fg_hwnd,
                        });
                    } else if vk == VK_RETURN as u32 {
                        let _ = tx.send(HookEvent::Boundary {
                            ch: '\n',
                            hwnd: fg_hwnd,
                        });
                    } else if vk == VK_TAB as u32 {
                        let _ = tx.send(HookEvent::Boundary {
                            ch: '\t',
                            hwnd: fg_hwnd,
                        });
                    } else if matches!(
                        vk as i32,
                        VK_LEFT
                            | VK_RIGHT
                            | VK_UP
                            | VK_DOWN
                            | VK_HOME
                            | VK_END
                            | VK_PRIOR
                            | VK_NEXT
                            | VK_DELETE
                            | VK_ESCAPE
                    ) {
                        let _ = tx.send(HookEvent::ClearBuffer);
                    } else if let Some(us_char) = vk_to_us_char(vk, shift_down, caps_on) {
                        let _ = tx.send(HookEvent::PrintableKey {
                            us_char,
                            hwnd: fg_hwnd,
                        });
                    }
                }
            }
        }
    }
    CallNextHookEx(null_mut(), n_code, w_param, l_param)
}

fn startup_shortcut_path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|ad| {
        PathBuf::from(ad)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join("KeySwitch.lnk")
    })
}

fn is_windows_autostart_enabled() -> bool {
    if let Some(p) = startup_shortcut_path() {
        if p.exists() {
            return true;
        }
    }
    if let Ok(out) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "KeySwitch",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        return out.status.success();
    }
    false
}

fn apply_windows_autostart_for_exe(exe_path: &Path, enable: bool) {
    let exe_str = exe_path.to_string_lossy().to_string();
    let work_dir = exe_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let ico_path = exe_path
        .parent()
        .map(|p| p.join("keyswitch_v2.ico").to_string_lossy().to_string())
        .unwrap_or_else(|| exe_str.clone());

    if enable {
        let _ = std::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "KeySwitch",
                "/t",
                "REG_SZ",
                "/d",
                &format!("\"{exe_str}\" --autostart"),
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let _ = std::process::Command::new("reg")
            .args([
                "add",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
                "/v",
                "KeySwitch",
                "/t",
                "REG_BINARY",
                "/d",
                "020000000000000000000000",
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Some(lnk) = startup_shortcut_path() {
            let lnk_str = lnk.to_string_lossy().to_string();
            let ps = format!(
                "$ws = New-Object -ComObject WScript.Shell; \
                 $s = $ws.CreateShortcut('{lnk_str}'); \
                 $s.TargetPath = '{exe_str}'; \
                 $s.Arguments = '--autostart'; \
                 $s.IconLocation = '{ico_path},0'; \
                 $s.WorkingDirectory = '{work_dir}'; \
                 $s.Description = 'KeySwitch — Thai/English Keyboard Auto-Switcher'; \
                 $s.Save()"
            );
            let _ = std::process::Command::new("powershell")
                .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    } else {
        let _ = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "KeySwitch",
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Some(lnk) = startup_shortcut_path() {
            let _ = std::fs::remove_file(lnk);
        }
    }
}

fn apply_windows_autostart_registry(enable: bool) {
    if let Ok(exe_path) = std::env::current_exe() {
        apply_windows_autostart_for_exe(&exe_path, enable);
    }
}

fn perform_self_install_custom(
    create_desktop_shortcut: bool,
    enable_autostart: bool,
    launch_after: bool,
) -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let local_app_data =
        std::env::var("LOCALAPPDATA").unwrap_or_else(|_| r"C:\Users\Public".to_string());
    let install_dir = PathBuf::from(&local_app_data)
        .join("Programs")
        .join("KeySwitch");
    std::fs::create_dir_all(&install_dir).map_err(|e| e.to_string())?;

    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", "KeySwitch.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(120));

    extract_embedded_data_to_dir(&install_dir)?;

    let target_exe = install_dir.join("KeySwitch.exe");
    if current_exe != target_exe {
        std::fs::copy(&current_exe, &target_exe).map_err(|e| e.to_string())?;
    }

    let _ = std::fs::remove_file(install_dir.join("keyswitch.ico"));
    let ico_file = install_dir.join("keyswitch_v2.ico");
    write_keyswitch_ico_file(&ico_file);

    let target_str = target_exe.to_string_lossy().to_string();
    let ico_str = ico_file.to_string_lossy().to_string();

    apply_windows_autostart_for_exe(&target_exe, enable_autostart);

    let uninst_key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\KeySwitch";
    let _ = std::process::Command::new("reg")
        .args(["add", uninst_key, "/v", "DisplayName", "/d", "KeySwitch 2.0", "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::process::Command::new("reg")
        .args(["add", uninst_key, "/v", "Publisher", "/d", APP_AUTHOR, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::process::Command::new("reg")
        .args(["add", uninst_key, "/v", "DisplayVersion", "/d", "2.0", "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::process::Command::new("reg")
        .args(["add", uninst_key, "/v", "DisplayIcon", "/d", &ico_str, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = std::process::Command::new("reg")
        .args([
            "add",
            uninst_key,
            "/v",
            "UninstallString",
            "/d",
            &format!("\"{target_str}\" --uninstall"),
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let dt_part = if create_desktop_shortcut {
        format!(
            "$dt = [Environment]::GetFolderPath('Desktop'); \
             $lnk1 = Join-Path $dt 'KeySwitch.lnk'; \
             Remove-Item $lnk1 -Force -ErrorAction SilentlyContinue; \
             $s1 = $ws.CreateShortcut($lnk1); \
             $s1.TargetPath = '{target_str}'; \
             $s1.IconLocation = '{ico_str},0'; \
             $s1.WorkingDirectory = '{}'; \
             $s1.Description = 'KeySwitch — Thai/English Keyboard Auto-Switcher'; \
             $s1.Save(); ",
            install_dir.display()
        )
    } else {
        String::new()
    };

    let ps_cmd = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         {dt_part}\
         $sm = [Environment]::GetFolderPath('Programs'); \
         $lnk2 = Join-Path $sm 'KeySwitch.lnk'; \
         Remove-Item $lnk2 -Force -ErrorAction SilentlyContinue; \
         $s2 = $ws.CreateShortcut($lnk2); \
         $s2.TargetPath = '{target_str}'; \
         $s2.IconLocation = '{ico_str},0'; \
         $s2.WorkingDirectory = '{}'; \
         $s2.Description = 'KeySwitch — Thai/English Keyboard Auto-Switcher'; \
         $s2.Save()",
        install_dir.display()
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, null_mut(), null_mut());
    }

    if launch_after {
        let _ = std::process::Command::new(&target_exe).spawn();
    }

    Ok(target_exe)
}

fn parse_args() -> (bool, bool, bool, bool) {
    let args: Vec<String> = std::env::args().collect();
    let mut silent_install = false;
    let mut wizard_install = false;
    let mut uninstall_mode = false;
    let mut autostart_launch = false;
    for arg in args.iter().skip(1) {
        if arg == "--install" || arg == "/install" || arg == "/S" {
            silent_install = true;
        } else if arg == "--setup" {
            wizard_install = true;
        } else if arg == "--uninstall" {
            uninstall_mode = true;
        } else if arg == "--autostart" {
            autostart_launch = true;
        }
    }
    if !uninstall_mode && !silent_install {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(stem) = exe.file_stem().map(|s| s.to_string_lossy().to_lowercase()) {
                if stem.contains("install") || stem.contains("setup") {
                    wizard_install = true;
                }
            }
        }
    }
    (silent_install, wizard_install, uninstall_mode, autostart_launch)
}

fn main() {
    unsafe {
        SetProcessDPIAware();
    }

    let (silent_install, wizard_install, uninstall_mode, autostart_launch) = parse_args();

    if uninstall_mode {
        apply_windows_autostart_registry(false);
        let _ = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\KeySwitch",
                "/f",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        return;
    }

    if silent_install {
        let _ = perform_self_install_custom(true, true, true);
        return;
    }

    let _single_instance_mutex = if !wizard_install {
        unsafe {
            let mutex_name = to_wstring("Local\\KeySwitch_SingleInstance_Mutex_v2");
            let h_mutex = winapi::um::synchapi::CreateMutexW(null_mut(), 0, mutex_name.as_ptr());
            if winapi::um::errhandlingapi::GetLastError() == 183 {
                if !autostart_launch {
                    let cls = to_wstring("KeySwitchNativeMainWnd");
                    let existing = FindWindowW(cls.as_ptr(), null_mut());
                    if !existing.is_null() {
                        PostMessageW(existing, WM_COMMAND, CMD_OPEN_CONFIG, 0);
                    }
                }
                if !h_mutex.is_null() {
                    winapi::um::handleapi::CloseHandle(h_mutex);
                }
                return;
            }
            h_mutex
        }
    } else {
        null_mut()
    };

    let mut engine = RightLangEngine::new();
    engine.auto_start = is_windows_autostart_enabled();
    sync_active_shortcut_atomics(&engine);
    let initial_thai_ui = engine.thai_ui;
    let initial_mode = engine.mode;
    let draft = ConfigDraft::from_engine(&engine);
    let initial_view = if wizard_install {
        ActiveView::InstallerWizard
    } else {
        ActiveView::ConfigForm
    };
    let (win_w, win_h) = view_dimensions(initial_view);

    let app_state = Arc::new(Mutex::new(AppUiState {
        engine,
        draft,
        active_view: initial_view,
        is_installer_process: wizard_install,
        inst_step: 0,
        inst_desktop_shortcut: true,
        inst_autostart: true,
        inst_launch_now: true,
        win_w,
        win_h,
        hover_pos: (-1, -1),
        regions: Vec::with_capacity(16),
        status_banner: None,
        toast_undo_rect: RECT {
            left: 14,
            top: 60,
            right: 116,
            bottom: 82,
        },
        toast_mute_rect: RECT {
            left: 124,
            top: 60,
            right: 238,
            bottom: 82,
        },
    }));

    if let Ok(mut slot) = GLOBAL_APP_STATE.lock() {
        *slot = Some(Arc::clone(&app_state));
    }

    unsafe {
        let h_instance = GetModuleHandleW(null_mut());
        let main_cls = to_wstring("KeySwitchNativeMainWnd");
        let toast_cls = to_wstring("KeySwitchNativeToastWnd");
        let h_icon_big = create_keyswitch_hicon(32, CorrectionMode::Online);

        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(main_wnd_proc);
        wc.hInstance = h_instance;
        wc.hIcon = h_icon_big;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        wc.lpszClassName = main_cls.as_ptr();
        RegisterClassW(&wc);

        let mut twc: WNDCLASSW = std::mem::zeroed();
        twc.lpfnWndProc = Some(toast_wnd_proc);
        twc.hInstance = h_instance;
        twc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        twc.lpszClassName = toast_cls.as_ptr();
        RegisterClassW(&twc);

        let base_style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let style = base_style | if wizard_install { WS_VISIBLE } else { 0 };

        let mut rc = RECT {
            left: 0,
            top: 0,
            right: win_w,
            bottom: win_h,
        };
        AdjustWindowRectEx(&mut rc, base_style, 0, 0);
        let outer_w = rc.right - rc.left;
        let outer_h = rc.bottom - rc.top;
        let (pos_x, pos_y) = center_screen_pos(outer_w, outer_h);

        let title_str = view_title(initial_view, initial_thai_ui);
        let hwnd = CreateWindowExW(
            0,
            main_cls.as_ptr(),
            to_wstring(title_str).as_ptr(),
            style,
            pos_x,
            pos_y,
            outer_w,
            outer_h,
            null_mut(),
            null_mut(),
            h_instance,
            null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        if let Ok(mut m_slot) = MAIN_HWND.lock() {
            *m_slot = hwnd as usize;
        }

        let (tx_pos, ty_pos) = bottom_right_workarea_pos(250, 90);
        let toast_hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
            toast_cls.as_ptr(),
            to_wstring("KeySwitchToast").as_ptr(),
            WS_POPUP,
            tx_pos,
            ty_pos,
            250,
            90,
            null_mut(),
            null_mut(),
            h_instance,
            null_mut(),
        );
        if let Ok(mut t_slot) = TOAST_HWND.lock() {
            *t_slot = toast_hwnd as usize;
        }

        if !wizard_install {
            let tbc = RegisterWindowMessageW(to_wstring("TaskbarCreated").as_ptr());
            WM_TASKBARCREATED.store(tbc, Ordering::Relaxed);
            if !add_or_update_tray_icon(hwnd, initial_mode, initial_thai_ui, true) {
                SetTimer(hwnd, TIMER_TRAY_RETRY, 1000, None);
            }
            start_keyboard_worker_and_hook(Arc::clone(&app_state));
        }

        let h_cur = winapi::um::processthreadsapi::GetCurrentProcess();
        let _ = winapi::um::winbase::SetProcessWorkingSetSize(h_cur, usize::MAX, usize::MAX);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
