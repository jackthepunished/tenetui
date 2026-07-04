//! The single source of color for tenetui — the *Tenet* palette.
//!
//! Design (see docs/whitepaper.md "Visual identity — Tenet"):
//! - **red = forward entropy, blue = inverted** — the only two saturated hues.
//! - a cold steel-grey base; red/blue are the only warm/cool signals.
//! - every color is sampled from a ramp at a normalized `t ∈ [0,1]`, interpolated
//!   in Oklab so midtones stay vivid, then quantized to the terminal's capability.
//!
//! No widget hardcodes an RGB literal; they all call into `Theme`.

// The Oklab matrices below are Björn Ottosson's published reference constants,
// kept at full precision on purpose so they stay correct if we ever move the
// math to f64. clippy flags the extra digits for f32; that's expected here.
#![allow(clippy::excessive_precision)]

use ratatui::style::Color;

/// Which end of the timeline a commit sits on, relative to the playhead.
///
/// The playhead is "now"; the future (newer commits, forward entropy) is red,
/// the past (older commits, inverted) is blue, and the playhead itself is the
/// white-hot pivot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pole {
    /// Older than the playhead — inverted / blue.
    Past,
    /// The playhead itself — white-hot pivot.
    Pivot,
    /// Newer than the playhead — forward / red.
    Future,
}

/// Terminal color capability, detected once at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit `Color::Rgb`.
    TrueColor,
    /// xterm 256-color cube.
    Palette256,
    /// Basic ANSI 16.
    Ansi16,
}

impl ColorDepth {
    /// Detect from the environment. `COLORTERM=truecolor|24bit` ⇒ true color;
    /// a `256` in `$TERM` ⇒ 256-color; otherwise assume 16-color.
    ///
    /// Read-only; called once at startup and stored in [`Theme`].
    pub fn detect() -> Self {
        if let Ok(ct) = std::env::var("COLORTERM") {
            let ct = ct.to_ascii_lowercase();
            if ct.contains("truecolor") || ct.contains("24bit") {
                return ColorDepth::TrueColor;
            }
        }
        match std::env::var("TERM") {
            Ok(term) if term.contains("256") => ColorDepth::Palette256,
            Ok(term) if term.is_empty() => ColorDepth::Ansi16,
            Ok(_) => ColorDepth::Palette256,
            Err(_) => ColorDepth::Ansi16,
        }
    }
}

/// The palette. Cheap to copy; construct once and thread through `AppState`.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub depth: ColorDepth,
}

/// Token categories for syntax highlighting. The mapping from `syntect` scopes
/// to these classes lives in `syntax.rs`; the *colors* for each live here, so
/// the palette stays in one module (see docs/decisions.md "Visual identity").
///
/// Every color below is deliberately low-chroma and cold — **never** a
/// saturated red or blue, since those two hues are reserved for time-direction
/// (forward/inverted) and the ghost trails. Syntax reads as quiet structure;
/// the pincer and the comet trail stay the only vivid things on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxClass {
    Text,
    Comment,
    Keyword,
    StringLit,
    Constant,
    Type,
    Function,
    Operator,
}

// --- Anchor colors (sRGB u8), the only literals in the app -------------------

/// Cold neutral base — quiet steel-grey, the resting color of everything.
const STEEL: Rgb = Rgb(96, 105, 120);
/// Dimmer steel for zero-churn ticks and inactive chrome.
const STEEL_DIM: Rgb = Rgb(64, 71, 84);
/// Forward entropy — a warm Tenet red.
const RED: Rgb = Rgb(255, 90, 77);
/// Inverted — a cool Tenet blue.
const BLUE: Rgb = Rgb(77, 158, 255);
/// The white-hot playhead pivot.
const PIVOT: Rgb = Rgb(245, 247, 255);

/// Muted syntax palette — cold, low-chroma, no saturated red/blue.
const SYNTAX_TEXT: Rgb = Rgb(182, 190, 202);
const SYNTAX_COMMENT: Rgb = Rgb(102, 112, 124);
const SYNTAX_KEYWORD: Rgb = Rgb(158, 144, 176);
const SYNTAX_STRING: Rgb = Rgb(170, 162, 130);
const SYNTAX_CONSTANT: Rgb = Rgb(178, 152, 140);
const SYNTAX_TYPE: Rgb = Rgb(150, 172, 158);
const SYNTAX_FUNCTION: Rgb = Rgb(158, 174, 186);
const SYNTAX_OPERATOR: Rgb = Rgb(140, 148, 160);

/// The muted sRGB for a syntax class, for building the highlighter's color
/// table. Kept here so every color literal stays in this one module.
pub fn syntax_rgb(class: SyntaxClass) -> (u8, u8, u8) {
    let Rgb(r, g, b) = match class {
        SyntaxClass::Text => SYNTAX_TEXT,
        SyntaxClass::Comment => SYNTAX_COMMENT,
        SyntaxClass::Keyword => SYNTAX_KEYWORD,
        SyntaxClass::StringLit => SYNTAX_STRING,
        SyntaxClass::Constant => SYNTAX_CONSTANT,
        SyntaxClass::Type => SYNTAX_TYPE,
        SyntaxClass::Function => SYNTAX_FUNCTION,
        SyntaxClass::Operator => SYNTAX_OPERATOR,
    };
    (r, g, b)
}

impl Theme {
    pub fn new() -> Self {
        Theme {
            depth: ColorDepth::detect(),
        }
    }

    /// The base foreground for file text — quiet, so the accents carry meaning.
    pub fn foreground(&self) -> Color {
        self.quantize(STEEL)
    }

    /// Muted chrome (borders, inactive labels).
    pub fn chrome(&self) -> Color {
        self.quantize(STEEL_DIM)
    }

    /// The pure directional accents, for labels and the pincer legend.
    pub fn forward(&self) -> Color {
        self.quantize(RED)
    }
    pub fn inverted(&self) -> Color {
        self.quantize(BLUE)
    }
    pub fn pivot(&self) -> Color {
        self.quantize(PIVOT)
    }

    /// Color for a timeline cell: its `pole` sets the hue, its churn `intensity`
    /// (`0.0..=1.0`) deepens steel → the pole's saturated end. This is the pincer:
    /// blue to the past, red to the future, white at the playhead.
    pub fn timeline_cell(&self, pole: Pole, intensity: f32) -> Color {
        let t = intensity.clamp(0.0, 1.0);
        let end = match pole {
            Pole::Past => BLUE,
            Pole::Future => RED,
            Pole::Pivot => PIVOT,
        };
        let start = if pole == Pole::Pivot {
            STEEL
        } else {
            STEEL_DIM
        };
        self.quantize(oklab_lerp(start, end, t))
    }

    /// Ghost-trail color for a changed line: `Past`/`Future` sets the hue by scrub
    /// direction, `decay` (`1.0` = fresh glow → `0.0` = faded to base) sets how far
    /// it has cooled back toward the foreground. Add/delete is *not* encoded here —
    /// that's luminance + a gutter sign elsewhere (deferred; see docs/decisions.md).
    pub fn ghost(&self, pole: Pole, decay: f32) -> Color {
        let glow = match pole {
            Pole::Past => BLUE,
            Pole::Future => RED,
            Pole::Pivot => PIVOT,
        };
        // decay 1.0 → full glow, 0.0 → base steel foreground.
        self.quantize(oklab_lerp(STEEL, glow, decay.clamp(0.0, 1.0)))
    }

    /// Quantize an arbitrary sRGB triple to the terminal capability. The
    /// highlighter uses this to route `syntect`'s resolved token colors through
    /// the same truecolor→256→16 fallback as every other color in the app.
    pub fn rgb(&self, r: u8, g: u8, b: u8) -> Color {
        self.quantize(Rgb(r, g, b))
    }

    /// Quantize an sRGB triple to the detected terminal capability.
    fn quantize(&self, c: Rgb) -> Color {
        match self.depth {
            ColorDepth::TrueColor => Color::Rgb(c.0, c.1, c.2),
            ColorDepth::Palette256 => Color::Indexed(c.to_xterm256()),
            ColorDepth::Ansi16 => c.to_ansi16(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::new()
    }
}

// --- Color math --------------------------------------------------------------

/// A plain sRGB triple (gamma-encoded, 0..=255).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    /// Nearest color in the xterm 6×6×6 cube (indices 16..=231).
    fn to_xterm256(self) -> u8 {
        fn axis(v: u8) -> u8 {
            // cube levels are 0, 95, 135, 175, 215, 255
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let mut best = 0u8;
            let mut best_d = u16::MAX;
            for (i, &l) in LEVELS.iter().enumerate() {
                let d = (v as i16 - l as i16).unsigned_abs();
                if d < best_d {
                    best_d = d;
                    best = i as u8;
                }
            }
            best
        }
        16 + 36 * axis(self.0) + 6 * axis(self.1) + axis(self.2)
    }

    /// Nearest of the basic ANSI-16 set (used only when direction must survive a
    /// 16-color terminal). We keep the palette meaningful: reds→red, blues→blue.
    fn to_ansi16(self) -> Color {
        let Rgb(r, g, b) = self;
        let (r, g, b) = (r as i32, g as i32, b as i32);
        // Bright vs. dim by overall luma.
        let luma = (r * 30 + g * 59 + b * 11) / 100;
        if r - b > 40 && r - g > 20 {
            return Color::LightRed;
        }
        if b - r > 40 {
            return Color::LightBlue;
        }
        if luma > 200 {
            Color::White
        } else if luma > 110 {
            Color::Gray
        } else {
            Color::DarkGray
        }
    }
}

/// Linear-light interpolation between two sRGB colors *in Oklab space*, so the
/// midpoint stays perceptually vivid instead of graying out.
fn oklab_lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let la = srgb_to_oklab(a);
    let lb = srgb_to_oklab(b);
    let m = [
        la[0] + (lb[0] - la[0]) * t,
        la[1] + (lb[1] - la[1]) * t,
        la[2] + (lb[2] - la[2]) * t,
    ];
    oklab_to_srgb(m)
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// sRGB → Oklab (Björn Ottosson's transform).
fn srgb_to_oklab(c: Rgb) -> [f32; 3] {
    let r = srgb_to_linear(c.0);
    let g = srgb_to_linear(c.1);
    let b = srgb_to_linear(c.2);

    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    [
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    ]
}

/// Oklab → sRGB.
fn oklab_to_srgb(lab: [f32; 3]) -> Rgb {
    let l_ = lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2];
    let m_ = lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2];
    let s_ = lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2];

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;

    Rgb(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_roundtrip_is_stable() {
        for c in [STEEL, RED, BLUE, PIVOT, Rgb(0, 0, 0), Rgb(255, 255, 255)] {
            let back = oklab_to_srgb(srgb_to_oklab(c));
            // Allow ±2 per channel for float/gamma rounding.
            assert!((c.0 as i16 - back.0 as i16).abs() <= 2, "{c:?} -> {back:?}");
            assert!((c.1 as i16 - back.1 as i16).abs() <= 2, "{c:?} -> {back:?}");
            assert!((c.2 as i16 - back.2 as i16).abs() <= 2, "{c:?} -> {back:?}");
        }
    }

    #[test]
    fn lerp_endpoints_match() {
        assert_eq!(oklab_lerp(STEEL, RED, 0.0), oklab_around(STEEL));
        assert_eq!(oklab_lerp(STEEL, RED, 1.0), oklab_around(RED));
    }

    /// Endpoints survive an Oklab round-trip within tolerance; compare to that.
    fn oklab_around(c: Rgb) -> Rgb {
        oklab_to_srgb(srgb_to_oklab(c))
    }

    #[test]
    fn future_is_redder_than_past() {
        let th = Theme {
            depth: ColorDepth::TrueColor,
        };
        let (fwd, past) = (
            th.timeline_cell(Pole::Future, 1.0),
            th.timeline_cell(Pole::Past, 1.0),
        );
        if let (Color::Rgb(fr, _, fb), Color::Rgb(pr, _, pb)) = (fwd, past) {
            assert!(fr > fb, "future should be red-dominant: {fwd:?}");
            assert!(pb > pr, "past should be blue-dominant: {past:?}");
        } else {
            panic!("expected rgb");
        }
    }
}
