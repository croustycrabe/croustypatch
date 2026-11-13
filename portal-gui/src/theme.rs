//! The CroustyLab colours (tokens.json), light and dark.
use gpui::{rgb, rgba, Hsla};

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub surface: Hsla,
    pub sunken: Hsla,
    pub hover: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub disabled: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub primary: Hsla,
    pub primary_hover: Hsla,
    pub primary_fg: Hsla,
    pub primary_subtle: Hsla,
    pub primary_text: Hsla,
    pub overlay: Hsla,
    pub success: Hsla,
    pub success_subtle: Hsla,
    pub warning_subtle: Hsla,
    pub warning_text: Hsla,
    pub danger: Hsla,
    pub danger_subtle: Hsla,
    pub info: Hsla,
    pub info_subtle: Hsla,
}

fn c(v: u32) -> Hsla {
    rgb(v).into()
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: c(0x14181b), surface: c(0x1b2125), sunken: c(0x101315), hover: c(0x263036), fg: c(0xeeeae0), muted: c(0xa9b6bb), disabled: c(0x5f6f76),
            border: c(0x34424a), border_strong: c(0x7b8b92), primary: c(0xd0786b), primary_hover: c(0xdd8f83), primary_fg: c(0x1b2125), primary_subtle: c(0x3a2420), primary_text: c(0xe08f83),
            overlay: rgba(0x000000a6).into(), success: c(0x4cc3ae), success_subtle: c(0x12332d), warning_subtle: c(0x3a2d0a), warning_text: c(0xf0b429),
            danger: c(0xff7d8f), danger_subtle: c(0x3d1620), info: c(0x6cb1f0), info_subtle: c(0x132a40),
        }
    } else {
        Palette {
            bg: c(0xf5f2ea), surface: c(0xfcfbf8), sunken: c(0xeeeae0), hover: c(0xe7e1d3), fg: c(0x1b2125), muted: c(0x4d6169), disabled: c(0x8a979c),
            border: c(0xd4ccb8), border_strong: c(0x6f7d82), primary: c(0xa44334), primary_hover: c(0x8a3628), primary_fg: c(0xf5f2ea), primary_subtle: c(0xf3e0db), primary_text: c(0xa44334),
            overlay: rgba(0x1b212580).into(), success: c(0x0e6e5e), success_subtle: c(0xd9ece6), warning_subtle: c(0xf8ebc4), warning_text: c(0x7a5200),
            danger: c(0xb4233f), danger_subtle: c(0xf6dfe1), info: c(0x245f9c), info_subtle: c(0xdbe6f3),
        }
    }
}
