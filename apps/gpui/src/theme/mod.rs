use gpui::{rgb, Hsla};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThemePalette {
    pub background: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub surface_active: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_soft: Hsla,
    pub accent: Hsla,
    pub accent_surface: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub terminal_background: Hsla,
    pub terminal_text: Hsla,
}

impl ThemePalette {
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    fn dark() -> Self {
        Self {
            background: rgb(0x151515).into(),
            sidebar: rgb(0x242424).into(),
            surface: rgb(0x1e1e1e).into(),
            surface_hover: rgb(0x303030).into(),
            surface_active: rgb(0x3a3d42).into(),
            border: rgb(0x343434).into(),
            border_strong: rgb(0x4a4a4a).into(),
            text: rgb(0xe7e7e7).into(),
            text_muted: rgb(0xa4a4a4).into(),
            text_soft: rgb(0x8f949d).into(),
            accent: rgb(0x8bbfff).into(),
            accent_surface: rgb(0x294366).into(),
            success: rgb(0x39d98a).into(),
            warning: rgb(0xffcc00).into(),
            danger: rgb(0xff5f57).into(),
            terminal_background: rgb(0x181818).into(),
            terminal_text: rgb(0xe0e0e0).into(),
        }
    }

    fn light() -> Self {
        Self {
            background: rgb(0xf4f5f7).into(),
            sidebar: rgb(0xffffff).into(),
            surface: rgb(0xffffff).into(),
            surface_hover: rgb(0xeceff3).into(),
            surface_active: rgb(0xdde2e8).into(),
            border: rgb(0xd7dce3).into(),
            border_strong: rgb(0xaeb7c2).into(),
            text: rgb(0x1f2933).into(),
            text_muted: rgb(0x66717f).into(),
            text_soft: rgb(0x7b8794).into(),
            accent: rgb(0x4f7cff).into(),
            accent_surface: rgb(0xdbeafe).into(),
            success: rgb(0x168a53).into(),
            warning: rgb(0xb77900).into(),
            danger: rgb(0xc93d3d).into(),
            terminal_background: rgb(0xffffff).into(),
            terminal_text: rgb(0x111827).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_toggle_roundtrips() {
        assert_eq!(ThemeMode::Dark.toggled().toggled(), ThemeMode::Dark);
    }
}
