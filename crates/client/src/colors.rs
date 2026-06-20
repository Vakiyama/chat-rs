use iced::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accents {
  pub rosewater: Color,
  pub flamingo: Color,
  pub pink: Color,
  pub mauve: Color,
  pub red: Color,
  pub maroon: Color,
  pub peach: Color,
  pub yellow: Color,
  pub green: Color,
  pub teal: Color,
  pub sky: Color,
  pub sapphire: Color,
  pub blue: Color,
  pub lavender: Color,
}

pub const FRAPPE: Accents = Accents {
  rosewater: Color::from_rgb8(0xf2, 0xd5, 0xcf),
  flamingo: Color::from_rgb8(0xee, 0xbe, 0xbe),
  pink: Color::from_rgb8(0xf4, 0xb8, 0xe4),
  mauve: Color::from_rgb8(0xca, 0x9e, 0xe6),
  red: Color::from_rgb8(0xe7, 0x82, 0x84),
  maroon: Color::from_rgb8(0xea, 0x99, 0x9c),
  peach: Color::from_rgb8(0xef, 0x9f, 0x76),
  yellow: Color::from_rgb8(0xe5, 0xc8, 0x90),
  green: Color::from_rgb8(0xa6, 0xd1, 0x89),
  teal: Color::from_rgb8(0x81, 0xc8, 0xbe),
  sky: Color::from_rgb8(0x99, 0xd1, 0xdb),
  sapphire: Color::from_rgb8(0x85, 0xc1, 0xdc),
  blue: Color::from_rgb8(0x8c, 0xaa, 0xee),
  lavender: Color::from_rgb8(0xba, 0xbb, 0xf1),
};

pub trait AccentsExt {
  fn accents(&self) -> Accents;
}

impl AccentsExt for Theme {
  fn accents(&self) -> Accents {
    match self {
      Theme::CatppuccinFrappe => FRAPPE,
      // Add other flavors here as you adopt them:
      // Theme::CatppuccinLatte => LATTE,
      // Theme::CatppuccinMacchiato => MACCHIATO,
      // Theme::CatppuccinMocha => MOCHA,
      _ => FRAPPE,
    }
  }
}
