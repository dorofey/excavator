//! App-owned semantic colors projected into the pinned GPUI Kit theme.
use crate::persistence::{AppearanceMode, AppearanceSettings, DarkTheme, LightTheme};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Window, WindowAppearance, px, rgb};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub background: u32,
    pub surface: u32,
    pub sidebar: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub selection: u32,
    pub selection_hover: u32,
    pub hover: u32,
    pub dialog: u32,
    pub error: u32,
    pub warning: u32,
    pub dark: bool,
}
pub fn resolve(settings: &AppearanceSettings, appearance: WindowAppearance) -> Tokens {
    let dark = match settings.mode {
        AppearanceMode::Dark => true,
        AppearanceMode::Light => false,
        AppearanceMode::System => matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    };
    if dark {
        match settings.dark_theme {
            DarkTheme::Graphite => Tokens {
                background: 0x1d2025,
                surface: 0x252930,
                sidebar: 0x21252b,
                border: 0x404750,
                text: 0xe4e8ee,
                muted: 0xa0aab7,
                accent: 0x9cbeed,
                selection: 0x34465f,
                selection_hover: 0x405775,
                hover: 0x2a3039,
                dialog: 0x2d3541,
                error: 0xf1aaaa,
                warning: 0xeac68e,
                dark,
            },
            DarkTheme::Midnight => Tokens {
                background: 0x121b29,
                surface: 0x1b2737,
                sidebar: 0x162131,
                border: 0x35485e,
                text: 0xe1ecf8,
                muted: 0x9aafc6,
                accent: 0x8ac6ef,
                selection: 0x274e70,
                selection_hover: 0x326085,
                hover: 0x243348,
                dialog: 0x24374c,
                error: 0xf0a9b4,
                warning: 0xf0ca8e,
                dark,
            },
        }
    } else {
        match settings.light_theme {
            LightTheme::Paper => Tokens {
                background: 0xfaf9f6,
                surface: 0xf0eee8,
                sidebar: 0xf3f1ec,
                border: 0xd3d0c7,
                text: 0x282b30,
                muted: 0x5e6670,
                accent: 0x245e9c,
                selection: 0xd6e5f5,
                selection_hover: 0xc4daf0,
                hover: 0xeceeeF,
                dialog: 0xeef2f7,
                error: 0xa32230,
                warning: 0x805414,
                dark,
            },
            LightTheme::Frost => Tokens {
                background: 0xf6f9fd,
                surface: 0xeaf0f7,
                sidebar: 0xeef3fa,
                border: 0xcbd5e2,
                text: 0x203044,
                muted: 0x53657b,
                accent: 0x155b96,
                selection: 0xcde3f6,
                selection_hover: 0xbdd8ef,
                hover: 0xe2edf8,
                dialog: 0xe6f0fa,
                error: 0xa71e3b,
                warning: 0x785114,
                dark,
            },
        }
    }
}
pub fn apply(settings: &AppearanceSettings, window: &mut Window, cx: &mut App) -> Tokens {
    cx.set_window_appearance(match settings.mode {
        AppearanceMode::Light => Some(WindowAppearance::Light),
        AppearanceMode::Dark => Some(WindowAppearance::Dark),
        AppearanceMode::System => None,
    });
    refresh(settings, window, cx)
}
pub fn refresh(settings: &AppearanceSettings, window: &mut Window, cx: &mut App) -> Tokens {
    let tokens = resolve(settings, cx.window_appearance());
    window.set_rem_size(px(settings.font_size));
    Theme::change(
        if tokens.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_family = settings.font_family.clone().into();
        theme.font_size = px(settings.font_size);
        let colors = &mut theme.colors;
        colors.background = rgb(tokens.background).into();
        colors.foreground = rgb(tokens.text).into();
        colors.border = rgb(tokens.border).into();
        colors.input = rgb(tokens.border).into();
        colors.muted = rgb(tokens.surface).into();
        colors.muted_foreground = rgb(tokens.muted).into();
        colors.accent = rgb(tokens.hover).into();
        colors.accent_foreground = rgb(tokens.text).into();
        colors.ring = rgb(tokens.accent).into();
        colors.caret = rgb(tokens.accent).into();
        colors.primary = rgb(tokens.accent).into();
        colors.primary_foreground = rgb(tokens.background).into();
        colors.secondary = rgb(tokens.surface).into();
        colors.secondary_foreground = rgb(tokens.text).into();
        colors.popover = rgb(tokens.surface).into();
        colors.popover_foreground = rgb(tokens.text).into();
        colors.list = rgb(tokens.background).into();
        colors.list_active = rgb(tokens.selection).into();
        colors.list_hover = rgb(tokens.hover).into();
        colors.list_head = rgb(tokens.surface).into();
        colors.sidebar = rgb(tokens.sidebar).into();
        colors.sidebar_foreground = rgb(tokens.text).into();
        colors.selection = rgb(tokens.selection).into();
        colors.scrollbar = rgb(tokens.background).into();
        colors.scrollbar_thumb = rgb(tokens.border).into();
        colors.scrollbar_thumb_hover = rgb(tokens.muted).into();
    });
    window.refresh();
    tokens
}
