//! Themes: Catppuccin (4 flavors x 14 accents), AMOLED variants, or the system look.
//! Default: Macchiato + Mauve.
use crate::theme_data::{self as d, NAMES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor { Latte, Frappe, Macchiato, Mocha }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Rosewater, Flamingo, Pink, Mauve, Red, Maroon, Peach,
    Yellow, Green, Teal, Sky, Sapphire, Blue, Lavender,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    /// Catppuccin palette. `amoled` swaps backgrounds for true black (Latte uses Mocha as its dark base).
    Catppuccin { flavor: Flavor, accent: Accent, amoled: bool },
    /// Native look: on Android follows the system light/dark setting through Slint's native style.
    System,
}

impl Default for ThemeChoice {
    fn default() -> Self { ThemeChoice::Catppuccin { flavor: Flavor::Macchiato, accent: Accent::Mauve, amoled: false } }
}

impl Accent {
    pub const ALL: [Accent; 14] = [
        Accent::Rosewater, Accent::Flamingo, Accent::Pink, Accent::Mauve, Accent::Red, Accent::Maroon, Accent::Peach,
        Accent::Yellow, Accent::Green, Accent::Teal, Accent::Sky, Accent::Sapphire, Accent::Blue, Accent::Lavender,
    ];
    pub fn name(self) -> &'static str { NAMES[self as usize] }
}

impl Flavor {
    pub const ALL: [Flavor; 4] = [Flavor::Latte, Flavor::Frappe, Flavor::Macchiato, Flavor::Mocha];
    fn table(self) -> &'static [u32; 26] {
        match self { Flavor::Latte => &d::LATTE, Flavor::Frappe => &d::FRAPPE, Flavor::Macchiato => &d::MACCHIATO, Flavor::Mocha => &d::MOCHA }
    }
    pub fn is_dark(self) -> bool { self != Flavor::Latte }
}

/// Resolved colors as 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    pub dark: bool,
    pub accent: u32,
    pub text: u32,
    pub subtext: u32,
    pub overlay: u32,
    pub surface2: u32,
    pub surface1: u32,
    pub surface0: u32,
    pub base: u32,
    pub mantle: u32,
    pub crust: u32,
    /// Semantic colors
    pub ok: u32,
    pub warn: u32,
    pub bad: u32,
}

impl ThemeChoice {
    /// `None` means "use the native/system style" (no custom palette).
    pub fn colors(self) -> Option<Colors> {
        let ThemeChoice::Catppuccin { flavor, accent, amoled } = self else { return None };
        let flavor = if amoled && !flavor.is_dark() { Flavor::Mocha } else { flavor };
        let t = flavor.table();
        let idx = |n: &str| NAMES.iter().position(|x| *x == n).unwrap();
        let mut c = Colors {
            dark: flavor.is_dark(),
            accent: t[accent as usize],
            text: t[idx("text")], subtext: t[idx("subtext0")], overlay: t[idx("overlay0")],
            surface2: t[idx("surface2")], surface1: t[idx("surface1")], surface0: t[idx("surface0")],
            base: t[idx("base")], mantle: t[idx("mantle")], crust: t[idx("crust")],
            ok: t[idx("green")], warn: t[idx("yellow")], bad: t[idx("red")],
        };
        if amoled {
            c.base = 0x000000; c.mantle = 0x000000; c.crust = 0x000000;
            c.surface0 = 0x121212; c.surface1 = 0x1c1c1c; c.surface2 = 0x2a2a2a;
        }
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_macchiato_mauve() {
        let c = ThemeChoice::default().colors().unwrap();
        assert_eq!(c.accent, 0xc6a0f6);
        assert_eq!(c.base, 0x24273a);
        assert!(c.dark);
    }
    #[test]
    fn amoled_is_true_black() {
        let c = ThemeChoice::Catppuccin { flavor: Flavor::Mocha, accent: Accent::Blue, amoled: true }.colors().unwrap();
        assert_eq!((c.base, c.mantle, c.crust), (0, 0, 0));
        assert_eq!(c.accent, 0x89b4fa);
        let l = ThemeChoice::Catppuccin { flavor: Flavor::Latte, accent: Accent::Mauve, amoled: true }.colors().unwrap();
        assert!(l.dark && l.base == 0);
    }
    #[test]
    fn latte_is_light_and_system_has_no_palette() {
        let l = ThemeChoice::Catppuccin { flavor: Flavor::Latte, accent: Accent::Mauve, amoled: false }.colors().unwrap();
        assert!(!l.dark);
        assert_eq!(l.accent, 0x8839ef);
        assert!(ThemeChoice::System.colors().is_none());
    }
    #[test]
    fn all_accents_have_names() {
        assert_eq!(Accent::ALL.len(), 14);
        assert_eq!(Accent::Mauve.name(), "mauve");
        assert_eq!(Accent::Lavender.name(), "lavender");
    }
}
