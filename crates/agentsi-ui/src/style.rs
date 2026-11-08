//! Style helpers for screens. A screen of `agentsi-app` writes no `px(…)`, so the
//! check `rg 'px\(' crates/agentsi-app/src` stays empty.

use gpui_kit::{Pixels, Styled};

pub trait StyleExt: Styled + Sized {
    /// The same padding on the left and on the right.
    fn pad_x(self, v: Pixels) -> Self {
        self.pl(v).pr(v)
    }
}

impl<T: Styled> StyleExt for T {}
