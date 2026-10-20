//! Built-in color themes for the FARMAN terminal and UI.

use crate::types::Rgb;

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub display_name: &'static str,
    pub background: Rgb,
    pub foreground: Rgb,
    pub bright_foreground: Rgb,
    pub dim_foreground: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection: Rgb,
    pub header_bg: Rgb,
    pub header_active: Rgb,
    pub status_bg: Rgb,
    pub panel_bg: Rgb,
    pub panel_alt: Rgb,
    pub border: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub accent_violet: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
    pub ansi: [Rgb; 16],
}

impl Theme {
    pub const NAMES: [&'static str; 3] = ["farman-dark", "midnight", "graphite"];

    pub fn by_name(name: &str) -> Option<&'static Theme> {
        match name {
            "farman-dark" => Some(&FARMAN_DARK),
            "midnight" => Some(&MIDNIGHT),
            "graphite" => Some(&GRAPHITE),
            _ => None,
        }
    }

    pub fn by_name_or_default(name: &str) -> &'static Theme {
        Self::by_name(name).unwrap_or(&FARMAN_DARK)
    }

    pub fn display_name_for(name: &str) -> &'static str {
        Self::by_name(name).map(|t| t.display_name).unwrap_or("FARMAN Dark")
    }

    /// Resolve an ANSI 256-color index. Named colors 0..16 use the theme.
    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => self.ansi[index as usize],
            16..=231 => {
                let i = index as usize - 16;
                const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                let r = LEVELS[i / 36];
                let g = LEVELS[(i / 6) % 6];
                let b = LEVELS[i % 6];
                Rgb::new(r, g, b)
            }
            232..=255 => {
                let v = 8 + (index - 232) * 10;
                Rgb::new(v, v, v)
            }
        }
    }

    pub fn dim(&self, c: Rgb) -> Rgb {
        c.scale(0.66)
    }
}

const fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
    }
}

pub static FARMAN_DARK: Theme = Theme {
    name: "farman-dark",
    display_name: "FARMAN Dark",
    background: rgb(0x080D18),
    foreground: rgb(0xD7E2F0),
    bright_foreground: rgb(0xF2F7FF),
    dim_foreground: rgb(0x8C9AB0),
    cursor: rgb(0x36D7E8),
    cursor_text: rgb(0x061018),
    selection: rgb(0x254B66),
    header_bg: rgb(0x0C1424),
    header_active: rgb(0x142039),
    status_bg: rgb(0x0A111F),
    panel_bg: rgb(0x101A2C),
    panel_alt: rgb(0x162238),
    border: rgb(0x233149),
    muted: rgb(0x7D8CA3),
    accent: rgb(0x35D6E8),
    accent_violet: rgb(0x8B5CF6),
    success: rgb(0x4ADE80),
    warning: rgb(0xFBBF24),
    error: rgb(0xFB7185),
    ansi: [
        rgb(0x1B2436),
        rgb(0xF07178),
        rgb(0x7EE787),
        rgb(0xE3B341),
        rgb(0x6CB6FF),
        rgb(0xC792EA),
        rgb(0x56D4DD),
        rgb(0xC9D1D9),
        rgb(0x4B5A70),
        rgb(0xFF8B94),
        rgb(0x9BF0A7),
        rgb(0xF2CC60),
        rgb(0x8CC8FF),
        rgb(0xDDB0FF),
        rgb(0x7BE8EF),
        rgb(0xF0F6FC),
    ],
};

pub static MIDNIGHT: Theme = Theme {
    name: "midnight",
    display_name: "Midnight Blue",
    background: rgb(0x050914),
    foreground: rgb(0xCFE0F5),
    bright_foreground: rgb(0xECF5FF),
    dim_foreground: rgb(0x8294AC),
    cursor: rgb(0x7C9CFF),
    cursor_text: rgb(0x030812),
    selection: rgb(0x1C3460),
    header_bg: rgb(0x08101F),
    header_active: rgb(0x101C34),
    status_bg: rgb(0x070D19),
    panel_bg: rgb(0x0D1728),
    panel_alt: rgb(0x132038),
    border: rgb(0x1C2B47),
    muted: rgb(0x7487A1),
    accent: rgb(0x7C9CFF),
    accent_violet: rgb(0xA78BFA),
    success: rgb(0x5EEAD4),
    warning: rgb(0xFCD34D),
    error: rgb(0xFB7185),
    ansi: [
        rgb(0x151D2E),
        rgb(0xEE6A7B),
        rgb(0x72E0A8),
        rgb(0xE5C07B),
        rgb(0x61AFEF),
        rgb(0xC678DD),
        rgb(0x56B6C2),
        rgb(0xABB2BF),
        rgb(0x44506A),
        rgb(0xFF8794),
        rgb(0x98F5C6),
        rgb(0xF6D98A),
        rgb(0x8CC8FF),
        rgb(0xE0A4F5),
        rgb(0x7FE3EE),
        rgb(0xE8EDF5),
    ],
};

pub static GRAPHITE: Theme = Theme {
    name: "graphite",
    display_name: "Graphite",
    background: rgb(0x111315),
    foreground: rgb(0xD9DEE5),
    bright_foreground: rgb(0xF1F4F7),
    dim_foreground: rgb(0x8B939D),
    cursor: rgb(0xB7C4D4),
    cursor_text: rgb(0x0D0F11),
    selection: rgb(0x37414C),
    header_bg: rgb(0x171A1E),
    header_active: rgb(0x22272D),
    status_bg: rgb(0x14171A),
    panel_bg: rgb(0x1A1E23),
    panel_alt: rgb(0x23282F),
    border: rgb(0x30363E),
    muted: rgb(0x858D98),
    accent: rgb(0x6FD3C7),
    accent_violet: rgb(0xA78BFA),
    success: rgb(0x77D39A),
    warning: rgb(0xE8C06A),
    error: rgb(0xE8798A),
    ansi: [
        rgb(0x22262B),
        rgb(0xE06C75),
        rgb(0x8CCF7E),
        rgb(0xD9B86C),
        rgb(0x74A9E8),
        rgb(0xC08FE0),
        rgb(0x6CC7C7),
        rgb(0xC8CED6),
        rgb(0x4C545D),
        rgb(0xF08A94),
        rgb(0xA9E3A0),
        rgb(0xEFD08C),
        rgb(0x9CC6F4),
        rgb(0xD8B1F0),
        rgb(0x8FE0E0),
        rgb(0xEEF2F6),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_themes_resolve() {
        for name in Theme::NAMES {
            let theme = Theme::by_name(name).expect("theme");
            assert_eq!(theme.name, name);
            assert!(theme.foreground.luminance() > theme.background.luminance());
        }
    }

    #[test]
    fn indexed_palette_is_stable() {
        let t = &FARMAN_DARK;
        assert_eq!(t.indexed(1), t.ansi[1]);
        assert_eq!(t.indexed(16), Rgb::new(0, 0, 0));
        assert_eq!(t.indexed(231), Rgb::new(255, 255, 255));
        assert_eq!(t.indexed(232), Rgb::new(8, 8, 8));
        assert_eq!(t.indexed(255), Rgb::new(238, 238, 238));
    }
}
