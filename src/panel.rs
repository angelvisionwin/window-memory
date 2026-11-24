//! Panel emergente dibujado con GDI.
//!
//! Recibe un modelo de vista ya resuelto: no conoce el origen de los datos.

use std::mem::size_of;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE, FillRect, GetMonitorInfoW, HDC,
    HFONT, HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect, SRCCOPY, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetWindowRect, HWND_TOPMOST, SW_SHOW, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow,
};
use windows::core::w;

use crate::platform::registry;

const HEADER: f32 = 54.0;
const ROW: f32 = 62.0;
const FOOTER: f32 = 34.0;
const WIDTH: f32 = 360.0;

/// Acento visual de una fila o de su barra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Neutral,
    Ok,
    Warn,
    Bad,
}

/// Una fila del panel, con los textos ya resueltos por la aplicación.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    pub title: String,
    pub detail: String,
    pub value: String,
    /// Si es `Some`, se dibuja una barra de progreso.
    pub percent: Option<u8>,
    pub accent: Accent,
}

/// Todo lo necesario para pintar el panel.
#[derive(Clone, Debug)]
pub struct PanelView {
    pub title: String,
    pub subtitle: Option<String>,
    pub footer: String,
    /// Título y pista que se muestran cuando no hay filas.
    pub empty: (String, String),
    pub rows: Vec<RowView>,
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn accent_color(accent: Accent) -> COLORREF {
    match accent {
        Accent::Neutral => rgb(0x8A, 0x8A, 0x92),
        Accent::Ok => rgb(0x3F, 0xB9, 0x50),
        Accent::Warn => rgb(0xD2, 0x9E, 0x22),
        Accent::Bad => rgb(0xF8, 0x51, 0x49),
    }
}

struct Theme {
    bg: COLORREF,
    text: COLORREF,
    sub: COLORREF,
    bar: COLORREF,
    border: COLORREF,
}

#[derive(Clone, Copy)]
struct Fonts {
    name: HFONT,
    small: HFONT,
    value: HFONT,
}

fn theme(dark: bool) -> Theme {
    if dark {
        Theme {
            bg: rgb(0x25, 0x25, 0x28),
            text: rgb(0xF2, 0xF2, 0xF4),
            sub: rgb(0x9C, 0x9C, 0xA4),
            bar: rgb(0x3C, 0x3C, 0x42),
            border: rgb(0x3C, 0x3C, 0x42),
        }
    } else {
        Theme {
            bg: rgb(0xFB, 0xFB, 0xFC),
            text: rgb(0x1B, 0x1B, 0x1F),
            sub: rgb(0x6C, 0x6C, 0x74),
            bar: rgb(0xE7, 0xE7, 0xEC),
            border: rgb(0xDC, 0xDC, 0xE2),
        }
    }
}

pub fn preferred_size(count: usize, dpi: u32) -> (i32, i32) {
    let scale = (dpi as f32 / 96.0).max(1.0);
    let rows = count.max(1) as f32;
    (
        (WIDTH * scale).round() as i32,
        ((HEADER + ROW * rows + FOOTER) * scale).round() as i32,
    )
}

pub unsafe fn show(hwnd: HWND, count: usize) {
    let dpi = {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi == 0 { 96 } else { dpi }
    };
    let (width, height) = preferred_size(count, dpi);

    let mut point = POINT::default();
    let _ = unsafe { GetCursorPos(&mut point) };
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = unsafe { GetMonitorInfoW(monitor, &mut monitor_info) };
    let work = monitor_info.rcWork;

    let mut x = point.x - width - 12;
    if x < work.left + 8 {
        x = work.left + 8;
    }
    if x + width > work.right - 8 {
        x = work.right - width - 8;
    }
    let y = (work.bottom - height - 8).max(work.top + 8);

    let _ = unsafe { SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW) };
    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let _ = unsafe { InvalidateRect(hwnd, None, true) };
}

pub unsafe fn resize(hwnd: HWND, count: usize) {
    let dpi = {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi == 0 { 96 } else { dpi }
    };
    let (width, height) = preferred_size(count, dpi);
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    let current_width = rect.right - rect.left;
    let current_height = rect.bottom - rect.top;
    if current_width == width && current_height == height {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            rect.right - width,
            rect.bottom - height,
            width,
            height,
            SWP_NOACTIVATE,
        )
    };
}

pub unsafe fn paint(hwnd: HWND, view: &PanelView) {
    let mut paint_struct = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut paint_struct) };
    unsafe { render(hwnd, hdc, view) };
    let _ = unsafe { windows::Win32::Graphics::Gdi::EndPaint(hwnd, &paint_struct) };
}

pub unsafe fn paint_to(hwnd: HWND, hdc: HDC, view: &PanelView) {
    unsafe { render(hwnd, hdc, view) };
}

unsafe fn render(hwnd: HWND, hdc: HDC, view: &PanelView) {
    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    let (width, height) = (client.right, client.bottom);
    if width <= 0 || height <= 0 {
        return;
    }

    let memory = unsafe { CreateCompatibleDC(hdc) };
    let bitmap = unsafe { CreateCompatibleBitmap(hdc, width, height) };
    let old_bitmap = unsafe { SelectObject(memory, HGDIOBJ(bitmap.0)) };

    let colors = theme(registry::is_dark_mode());
    let dpi = {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi == 0 { 96 } else { dpi }
    };
    let scale = dpi as f32 / 96.0;
    let px = |value: f32| (value * scale).round() as i32;

    unsafe {
        fill_rect(
            memory,
            RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            colors.bg,
        )
    };

    let title_font = unsafe { font(px(16.0), 600) };
    let name_font = unsafe { font(px(13.5), 600) };
    let small_font = unsafe { font(px(10.5), 400) };
    let value_font = unsafe { font(px(15.0), 700) };

    let mut title = RECT {
        left: px(18.0),
        top: px(14.0),
        right: width - px(150.0),
        bottom: px(14.0) + px(22.0),
    };
    unsafe {
        text(
            memory,
            &view.title,
            &mut title,
            title_font,
            colors.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        )
    };

    if let Some(subtitle) = &view.subtitle {
        let mut clock = RECT {
            left: width - px(160.0),
            top: px(18.0),
            right: width - px(18.0),
            bottom: px(18.0) + px(16.0),
        };
        unsafe {
            text(
                memory,
                subtitle,
                &mut clock,
                small_font,
                colors.sub,
                DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            )
        };
    }

    unsafe {
        fill_rect(
            memory,
            RECT {
                left: px(18.0),
                top: px(46.0),
                right: width - px(18.0),
                bottom: px(47.0),
            },
            colors.border,
        )
    };

    let fonts = Fonts {
        name: name_font,
        small: small_font,
        value: value_font,
    };

    let mut y = px(HEADER);
    if view.rows.is_empty() {
        let mut empty = RECT {
            left: px(20.0),
            top: y + px(12.0),
            right: width - px(20.0),
            bottom: y + px(36.0),
        };
        unsafe {
            text(
                memory,
                &view.empty.0,
                &mut empty,
                name_font,
                colors.text,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            )
        };
        let mut hint = RECT {
            left: px(20.0),
            top: y + px(38.0),
            right: width - px(20.0),
            bottom: y + px(60.0),
        };
        unsafe {
            text(
                memory,
                &view.empty.1,
                &mut hint,
                small_font,
                colors.sub,
                DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            )
        };
    } else {
        for row in &view.rows {
            unsafe { draw_row(memory, &colors, scale, width, y, row, fonts) };
            y += px(ROW);
        }
    }

    let mut footer = RECT {
        left: px(18.0),
        top: height - px(26.0),
        right: width - px(18.0),
        bottom: height - px(8.0),
    };
    unsafe {
        text(
            memory,
            &view.footer,
            &mut footer,
            small_font,
            colors.sub,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        )
    };

    let _ = unsafe { BitBlt(hdc, 0, 0, width, height, memory, 0, 0, SRCCOPY) };

    unsafe {
        SelectObject(memory, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory);
        for handle in [title_font, name_font, small_font, value_font] {
            let _ = DeleteObject(HGDIOBJ(handle.0));
        }
    }
}

unsafe fn draw_row(
    dc: HDC,
    colors: &Theme,
    scale: f32,
    width: i32,
    y: i32,
    row: &RowView,
    fonts: Fonts,
) {
    let px = |value: f32| (value * scale).round() as i32;
    let color = accent_color(row.accent);

    let mut title = RECT {
        left: px(18.0),
        top: y + px(7.0),
        right: width - px(86.0),
        bottom: y + px(7.0) + px(20.0),
    };
    unsafe {
        text(
            dc,
            &row.title,
            &mut title,
            fonts.name,
            colors.text,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        )
    };

    let mut value = RECT {
        left: width - px(84.0),
        top: y + px(6.0),
        right: width - px(18.0),
        bottom: y + px(6.0) + px(22.0),
    };
    unsafe {
        text(
            dc,
            &row.value,
            &mut value,
            fonts.value,
            color,
            DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        )
    };

    let mut detail = RECT {
        left: px(18.0),
        top: y + px(28.0),
        right: width - px(18.0),
        bottom: y + px(28.0) + px(15.0),
    };
    unsafe {
        text(
            dc,
            &row.detail,
            &mut detail,
            fonts.small,
            colors.sub,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        )
    };

    if let Some(percent) = row.percent {
        let bar = RECT {
            left: px(18.0),
            top: y + px(47.0),
            right: width - px(18.0),
            bottom: y + px(47.0) + px(6.0),
        };
        unsafe { round_fill(dc, bar, px(3.0), colors.bar) };
        if percent > 0 {
            let total = bar.right - bar.left;
            let filled = ((total as f32) * percent as f32 / 100.0)
                .max(px(4.0) as f32)
                .min(total as f32) as i32;
            unsafe {
                round_fill(
                    dc,
                    RECT {
                        left: bar.left,
                        top: bar.top,
                        right: bar.left + filled,
                        bottom: bar.bottom,
                    },
                    px(3.0),
                    color,
                )
            };
        }
    }
}

unsafe fn fill_rect(dc: HDC, rect: RECT, color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(color) };
    unsafe { FillRect(dc, &rect, brush) };
    let _ = unsafe { DeleteObject(HGDIOBJ(brush.0)) };
}

unsafe fn round_fill(dc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    let brush = unsafe { CreateSolidBrush(color) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, color) };
    let old_brush = unsafe { SelectObject(dc, HGDIOBJ(brush.0)) };
    let old_pen = unsafe { SelectObject(dc, HGDIOBJ(pen.0)) };
    let _ = unsafe {
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        )
    };
    unsafe {
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

unsafe fn text(
    dc: HDC,
    value: &str,
    rect: &mut RECT,
    font: HFONT,
    color: COLORREF,
    flags: DRAW_TEXT_FORMAT,
) {
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    unsafe {
        SelectObject(dc, HGDIOBJ(font.0));
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT);
        DrawTextW(dc, &mut wide, rect, flags);
    }
}

unsafe fn font(size: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            -size,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Segoe UI"),
        )
    }
}
