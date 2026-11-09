//! Style helpers for screens. A screen of `agentsi-app` writes no `px(…)`, so the
//! check `rg 'px\(' crates/agentsi-app/src` stays empty.

use gpui_kit::*;

use crate::tokens::{WorkState, control, motion, radius, status};

/// The peak opacity of the glow shadow in a pulse.
const PULSE_PEAK: f32 = 0.8;

/// The glow of `state` as a child that covers its parent and takes no layout space. `key`
/// names the animation: use a new key for each alert, so that a new alert pulses again.
/// Returns nothing for a state that has no glow.
pub fn glow(key: impl Into<ElementId>, state: WorkState) -> Option<AnimationElement<Div>> {
    let pulses = state.pulses();
    if pulses == 0 {
        return None;
    }
    let color = state.color();
    let mut chain = vec![Animation::new(motion::GLOW_IN)];
    chain
        .extend((0..pulses).map(|_| Animation::new(motion::PULSE).with_max_fps(motion::PULSE_FPS)));
    let overlay = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(radius::md())
        .border(control::GLOW_EDGE);
    Some(overlay.with_animations(key, chain, move |el, ix, delta| {
        // Fade in once, then rise and fall to the resting shadow. The last frame is the resting
        // glow, which is also what reduced motion shows.
        let (edge, shadow) = if ix == 0 {
            (delta, status::BORDER_ALPHA * delta)
        } else {
            let tri = 1. - (2. * delta - 1.).abs();
            (
                1.,
                status::BORDER_ALPHA + (PULSE_PEAK - status::BORDER_ALPHA) * tri,
            )
        };
        el.border_color(color.alpha(edge)).shadow(vec![BoxShadow {
            color: color.alpha(shadow),
            offset: point(px(0.), px(0.)),
            blur_radius: control::GLOW_BLUR,
            spread_radius: px(0.),
            inset: false,
        }])
    }))
}

/// The tint of `state` over its parent. It fades out in [`motion::DIALOG`] and takes no layout
/// space. Use a new `key` for each opening, so that each opening fades again. With reduced
/// motion the last frame shows at once, and that frame is clear.
pub fn tint_fade(key: impl Into<ElementId>, state: WorkState) -> AnimationElement<Div> {
    let tint = state.tint();
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .with_animation(key, Animation::new(motion::DIALOG), move |el, delta| {
            el.bg(tint.alpha(TINT_PEAK * (1. - delta)))
        })
}

/// The tint at the start of a fade.
const TINT_PEAK: f32 = 0.35;

/// A 1 px edge in the color of `state`, over its parent. It fades in during
/// [`motion::HOVER`] and takes no layout space.
pub fn edge(key: impl Into<ElementId>, state: WorkState) -> AnimationElement<Div> {
    let color = state.color();
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .border_1()
        .with_animation(key, Animation::new(motion::HOVER), move |el, delta| {
            el.border_color(color.alpha(delta))
        })
}

pub trait StyleExt: Styled + Sized {
    /// The same padding on the left and on the right.
    fn pad_x(self, v: Pixels) -> Self {
        self.pl(v).pr(v)
    }

    /// The glow of `state` around this element. See [`glow`].
    fn glow(self, key: impl Into<ElementId>, state: Option<WorkState>) -> Self
    where
        Self: ParentElement,
    {
        match state.and_then(|s| glow(key, s)) {
            Some(g) => self.relative().child(g),
            None => self,
        }
    }
}

impl<T: Styled> StyleExt for T {}
