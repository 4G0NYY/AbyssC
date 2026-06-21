//! The Abyssal look: a cold, deep palette and the widget styles that paint it.
//!
//! Skirk dwells in the dark, so the surface is near-black with a faint blue cast;
//! the only warmth is the frost-cyan of her hair (the primary accent) and the
//! abyssal violet beneath it. Nothing glows that does not need to.

use iced::border::Radius;
use iced::widget::{button, container, pick_list, progress_bar, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

// --- Palette ---------------------------------------------------------------

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color { a, ..rgb(r, g, b) }
}

/// Window backdrop — the void.
pub const VOID: Color = rgb(7, 9, 15);
/// Panel background.
pub const ABYSS: Color = rgb(14, 19, 32);
/// Raised surface (cards, inputs).
pub const SURFACE: Color = rgb(22, 29, 46);
/// Hovered/active surface.
pub const SURFACE_HI: Color = rgb(30, 39, 64);
/// Strong border.
pub const BORDER_LINE: Color = rgb(36, 48, 73);
/// Soft, barely-there border.
pub const BORDER_SOFT: Color = rgb(26, 34, 51);

/// Primary text.
pub const TEXT: Color = rgb(213, 222, 239);
/// Muted/secondary text.
pub const MUTED: Color = rgb(122, 132, 153);

/// Frost-cyan — Skirk's hair, the primary accent.
pub const CYAN: Color = rgb(91, 214, 208);
/// Deeper cyan, for fills.
pub const CYAN_DEEP: Color = rgb(43, 168, 164);
/// Abyssal violet — the secondary accent.
pub const VIOLET: Color = rgb(138, 124, 240);
/// Skirk's eyes — danger / destructive.
pub const RED: Color = rgb(224, 86, 107);
/// Success.
pub const GREEN: Color = rgb(95, 212, 155);

// --- Theme -----------------------------------------------------------------

/// The custom dark theme used by the whole app.
pub fn abyss() -> Theme {
    Theme::custom(
        "Abyss".to_string(),
        iced::theme::Palette {
            background: VOID,
            text: TEXT,
            primary: CYAN,
            success: GREEN,
            danger: RED,
        },
    )
}

// --- Containers ------------------------------------------------------------

fn rounded(radius: f32) -> Border {
    Border { color: Color::TRANSPARENT, width: 0.0, radius: Radius::from(radius) }
}

/// The outer window background.
pub fn root(_t: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(VOID)), ..Default::default() }
}

/// A primary panel — the main working area.
pub fn panel(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ABYSS)),
        border: Border { color: BORDER_SOFT, width: 1.0, radius: Radius::from(14.0) },
        ..Default::default()
    }
}

/// A raised card inside a panel.
pub fn card(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        border: Border { color: BORDER_LINE, width: 1.0, radius: Radius::from(10.0) },
        ..Default::default()
    }
}

/// The drop zone, idle.
pub fn dropzone(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgba(43, 168, 164, 0.04))),
        border: Border { color: rgba(91, 214, 208, 0.30), width: 1.5, radius: Radius::from(12.0) },
        ..Default::default()
    }
}

/// The header band.
pub fn header(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ABYSS)),
        border: Border { color: BORDER_SOFT, width: 0.0, radius: Radius::from(0.0) },
        ..Default::default()
    }
}

/// The bottom status bar.
pub fn status_bar(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgba(22, 29, 46, 0.6))),
        border: Border { color: BORDER_SOFT, width: 1.0, radius: Radius::from(10.0) },
        ..Default::default()
    }
}

/// A subtle inset row (e.g. a listed input file).
pub fn row_item(_t: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(rgba(30, 39, 64, 0.5))),
        border: rounded(8.0),
        ..Default::default()
    }
}

/// A tinted info chip.
pub fn chip(_t: &Theme) -> container::Style {
    container::Style {
        text_color: Some(CYAN),
        background: Some(Background::Color(rgba(43, 168, 164, 0.12))),
        border: Border { color: rgba(91, 214, 208, 0.25), width: 1.0, radius: Radius::from(20.0) },
        ..Default::default()
    }
}

/// The "a new version awaits" banner — violet-tinted, cyan-edged.
pub fn update_banner(_t: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT),
        background: Some(Background::Color(rgba(138, 124, 240, 0.12))),
        border: Border { color: rgba(138, 124, 240, 0.5), width: 1.0, radius: Radius::from(10.0) },
        ..Default::default()
    }
}

// --- Buttons ---------------------------------------------------------------

/// The primary call-to-action (Compress / Extract). Frost-cyan with a glow.
pub fn primary(_t: &Theme, status: button::Status) -> button::Style {
    let (fill, glow) = match status {
        button::Status::Hovered => (CYAN, 18.0),
        button::Status::Pressed => (CYAN_DEEP, 6.0),
        button::Status::Disabled => (rgba(43, 168, 164, 0.25), 0.0),
        button::Status::Active => (CYAN_DEEP, 12.0),
    };
    button::Style {
        background: Some(Background::Color(fill)),
        text_color: VOID,
        border: rounded(10.0),
        shadow: Shadow {
            color: rgba(91, 214, 208, 0.45),
            offset: Vector::new(0.0, 0.0),
            blur_radius: glow,
        },
    }
}

/// A quiet, outlined button (Add files, Browse, ...).
pub fn ghost(_t: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => SURFACE_HI,
        button::Status::Pressed => SURFACE,
        _ => SURFACE,
    };
    let border_col = match status {
        button::Status::Hovered => rgba(91, 214, 208, 0.5),
        _ => BORDER_LINE,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT,
        border: Border { color: border_col, width: 1.0, radius: Radius::from(9.0) },
        shadow: Shadow::default(),
    }
}

/// A selected segmented-tab.
pub fn tab_active(_t: &Theme, _s: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(SURFACE_HI)),
        text_color: CYAN,
        border: Border { color: rgba(91, 214, 208, 0.5), width: 1.0, radius: Radius::from(9.0) },
        shadow: Shadow {
            color: rgba(91, 214, 208, 0.25),
            offset: Vector::new(0.0, 0.0),
            blur_radius: 10.0,
        },
    }
}

/// An unselected segmented-tab.
pub fn tab_inactive(_t: &Theme, status: button::Status) -> button::Style {
    let text_color = match status {
        button::Status::Hovered => TEXT,
        _ => MUTED,
    };
    button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color,
        border: rounded(9.0),
        shadow: Shadow::default(),
    }
}

/// A small destructive/ghost action (Remove, Clear).
pub fn danger_ghost(_t: &Theme, status: button::Status) -> button::Style {
    let text_color = match status {
        button::Status::Hovered => RED,
        _ => MUTED,
    };
    button::Style {
        background: Some(Background::Color(Color::TRANSPARENT)),
        text_color,
        border: rounded(8.0),
        shadow: Shadow::default(),
    }
}

/// A Commander list row.
pub fn browse_row(_t: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Background::Color(rgba(91, 214, 208, 0.08)),
        _ => Background::Color(Color::TRANSPARENT),
    };
    button::Style {
        background: Some(bg),
        text_color: TEXT,
        border: rounded(7.0),
        shadow: Shadow::default(),
    }
}

/// The selected Commander row.
pub fn browse_row_selected(_t: &Theme, _s: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(rgba(43, 168, 164, 0.16))),
        text_color: TEXT,
        border: Border { color: rgba(91, 214, 208, 0.4), width: 1.0, radius: Radius::from(7.0) },
        shadow: Shadow::default(),
    }
}

// --- Inputs ----------------------------------------------------------------

pub fn progress(_t: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(SURFACE),
        bar: Background::Color(CYAN),
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: Radius::from(6.0) },
    }
}

pub fn picklist(_t: &Theme, status: pick_list::Status) -> pick_list::Style {
    let border_col = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened => rgba(91, 214, 208, 0.6),
        pick_list::Status::Active => BORDER_LINE,
    };
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: MUTED,
        handle_color: CYAN,
        background: Background::Color(SURFACE),
        border: Border { color: border_col, width: 1.0, radius: Radius::from(9.0) },
    }
}

pub fn field(_t: &Theme, status: text_input::Status) -> text_input::Style {
    let border_col = match status {
        text_input::Status::Focused => rgba(91, 214, 208, 0.6),
        _ => BORDER_LINE,
    };
    text_input::Style {
        background: Background::Color(SURFACE),
        border: Border { color: border_col, width: 1.0, radius: Radius::from(9.0) },
        icon: MUTED,
        placeholder: MUTED,
        value: TEXT,
        selection: rgba(91, 214, 208, 0.30),
    }
}
