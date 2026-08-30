use gpui::{FontWeight, Rgba, Styled, rgb};

/// 为当前 GPUI 版本补齐常用字重语义，保持 UI 声明式代码易读。
pub(crate) trait FontWeightExt: Styled + Sized {
    fn font_medium(self) -> Self {
        self.font_weight(FontWeight::MEDIUM)
    }

    fn font_semibold(self) -> Self {
        self.font_weight(FontWeight::SEMIBOLD)
    }
}

impl<T: Styled> FontWeightExt for T {}

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) background: Rgba,
    pub(crate) surface: Rgba,
    pub(crate) surface_alt: Rgba,
    pub(crate) border: Rgba,
    pub(crate) text: Rgba,
    pub(crate) muted: Rgba,
    pub(crate) accent: Rgba,
    pub(crate) accent_soft: Rgba,
    pub(crate) success: Rgba,
    pub(crate) success_soft: Rgba,
}

impl Palette {
    pub(crate) fn light() -> Self {
        Self {
            background: rgb(0xf4f6f8),
            surface: rgb(0xffffff),
            surface_alt: rgb(0xf8fafc),
            border: rgb(0xe2e7ec),
            text: rgb(0x17202b),
            muted: rgb(0x687587),
            accent: rgb(0x3468df),
            accent_soft: rgb(0xeaf0ff),
            success: rgb(0x16845c),
            success_soft: rgb(0xe3f5ed),
        }
    }

    pub(crate) fn dark() -> Self {
        Self {
            background: rgb(0x15191e),
            surface: rgb(0x1c222a),
            surface_alt: rgb(0x242c36),
            border: rgb(0x343e4a),
            text: rgb(0xf2f5f7),
            muted: rgb(0x9aa7b5),
            accent: rgb(0x7ea5ff),
            accent_soft: rgb(0x26385d),
            success: rgb(0x5ace9f),
            success_soft: rgb(0x204438),
        }
    }
}
