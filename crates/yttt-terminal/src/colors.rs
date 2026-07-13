//! Color palette for terminal emulator.
//!
//! This module provides [`ColorPalette`] and [`ColorPaletteBuilder`] for managing
//! terminal colors. It supports:
//!
//! - **16 ANSI colors**: The standard palette (colors 0-15)
//! - **256-color mode**: Extended palette with 6×6×6 RGB cube and grayscale ramp
//! - **True color (24-bit RGB)**: Direct RGB color specification
//!
//! # Default ANSI Colors
//!
//! The default palette uses colors similar to common terminal emulators:
//!
//! | Index | Name | RGB |
//! |-------|------|-----|
//! | 0 | Black | `#000000` |
//! | 1 | Red | `#CC0000` |
//! | 2 | Green | `#4E9A06` |
//! | 3 | Yellow | `#C4A000` |
//! | 4 | Blue | `#3465A4` |
//! | 5 | Magenta | `#75507B` |
//! | 6 | Cyan | `#06989A` |
//! | 7 | White | `#D3D7CF` |
//! | 8 | Bright Black | `#555753` |
//! | 9 | Bright Red | `#EF2929` |
//! | 10 | Bright Green | `#8AE234` |
//! | 11 | Bright Yellow | `#FCE94F` |
//! | 12 | Bright Blue | `#729FCF` |
//! | 13 | Bright Magenta | `#AD7FA8` |
//! | 14 | Bright Cyan | `#34E2E2` |
//! | 15 | Bright White | `#EEEEEC` |
//!
//! # 256-Color Mode
//!
//! Colors 16-255 are calculated:
//!
//! - **16-231**: 6×6×6 RGB cube where each component is `0, 95, 135, 175, 215, 255`
//! - **232-255**: 24-step grayscale from `#080808` to `#EEEEEE`
//!
//! # Example
//!
//! ```
//! use yttt_terminal::ColorPalette;
//!
//! // Use default palette
//! let default = ColorPalette::default();
//!
//! // Or customize with builder
//! let custom = ColorPalette::builder()
//!     .background(0x1a, 0x1b, 0x26)  // Tokyo Night background
//!     .foreground(0xa9, 0xb1, 0xd6)  // Tokyo Night foreground
//!     .red(0xf7, 0x76, 0x8e)
//!     .green(0x9e, 0xce, 0x6a)
//!     .blue(0x7a, 0xa2, 0xf7)
//!     .build();
//! ```

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::Hsla;

/// A color palette that maps ANSI colors to GPUI Hsla colors.
///
/// This struct maintains the 16-color ANSI palette, 256-color extended palette,
/// and special colors (foreground, background, cursor). It provides the
/// [`resolve`](Self::resolve) method to convert any terminal color to GPUI's
/// [`Hsla`] format for rendering.
///
/// # Color Resolution
///
/// The [`resolve`](Self::resolve) method handles all terminal color types:
///
/// 1. **Named colors** (0-15): Looked up in the ANSI palette
/// 2. **Special colors**: Foreground, Background, Cursor, Dim variants
/// 3. **Indexed colors** (16-255): Looked up in the extended palette
/// 4. **True colors**: Converted directly from RGB
///
/// # Creating a Palette
///
/// Use [`ColorPalette::default()`] for standard colors, or
/// [`ColorPalette::builder()`] for customization:
///
/// ```
/// use yttt_terminal::ColorPalette;
///
/// let palette = ColorPalette::builder()
///     .background(0x28, 0x28, 0x28)
///     .foreground(0xeb, 0xdb, 0xb2)
///     .build();
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ColorPalette {
    ansi_colors: [Hsla; 16],
    ansi_rgb: [Rgb; 16],
    extended_colors: [Hsla; 256],
    extended_rgb: [Rgb; 256],
    foreground: Hsla,
    foreground_rgb: Rgb,
    background: Hsla,
    background_rgb: Rgb,
    cursor: Hsla,
    cursor_rgb: Rgb,
    selection_background: Hsla,
    selection_background_rgb: Rgb,
    selection_foreground: Option<Hsla>,
    selection_foreground_rgb: Option<Rgb>,
    cursor_text: Option<Hsla>,
    cursor_text_rgb: Option<Rgb>,
    search_foreground: Hsla,
    search_background: Hsla,
    focused_search_foreground: Hsla,
    focused_search_background: Hsla,
    hint_start_foreground: Hsla,
    hint_start_background: Hsla,
    hint_end_foreground: Hsla,
    hint_end_background: Hsla,
}

impl Default for ColorPalette {
    fn default() -> Self {
        let ansi_rgb = [
            Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
            Rgb {
                r: 0xcc,
                g: 0x00,
                b: 0x00,
            },
            Rgb {
                r: 0x4e,
                g: 0x9a,
                b: 0x06,
            },
            Rgb {
                r: 0xc4,
                g: 0xa0,
                b: 0x00,
            },
            Rgb {
                r: 0x34,
                g: 0x65,
                b: 0xa4,
            },
            Rgb {
                r: 0x75,
                g: 0x50,
                b: 0x7b,
            },
            Rgb {
                r: 0x06,
                g: 0x98,
                b: 0x9a,
            },
            Rgb {
                r: 0xd3,
                g: 0xd7,
                b: 0xcf,
            },
            Rgb {
                r: 0x55,
                g: 0x57,
                b: 0x53,
            },
            Rgb {
                r: 0xef,
                g: 0x29,
                b: 0x29,
            },
            Rgb {
                r: 0x8a,
                g: 0xe2,
                b: 0x34,
            },
            Rgb {
                r: 0xfc,
                g: 0xe9,
                b: 0x4f,
            },
            Rgb {
                r: 0x72,
                g: 0x9f,
                b: 0xcf,
            },
            Rgb {
                r: 0xad,
                g: 0x7f,
                b: 0xa8,
            },
            Rgb {
                r: 0x34,
                g: 0xe2,
                b: 0xe2,
            },
            Rgb {
                r: 0xee,
                g: 0xee,
                b: 0xec,
            },
        ];
        let ansi_colors = ansi_rgb.map(rgb_to_hsla);
        let mut extended_rgb = [Rgb { r: 0, g: 0, b: 0 }; 256];
        extended_rgb[..16].copy_from_slice(&ansi_rgb);

        let mut index = 16;
        for red in 0..6 {
            for green in 0..6 {
                for blue in 0..6 {
                    extended_rgb[index] = Rgb {
                        r: if red == 0 { 0 } else { 55 + red * 40 },
                        g: if green == 0 { 0 } else { 55 + green * 40 },
                        b: if blue == 0 { 0 } else { 55 + blue * 40 },
                    };
                    index += 1;
                }
            }
        }
        for index in 0..24 {
            let gray = (8 + index * 10) as u8;
            extended_rgb[232 + index] = Rgb {
                r: gray,
                g: gray,
                b: gray,
            };
        }
        let extended_colors = extended_rgb.map(rgb_to_hsla);

        let foreground_rgb = Rgb {
            r: 0xd4,
            g: 0xd4,
            b: 0xd4,
        };
        let background_rgb = Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x1e,
        };
        let cursor_rgb = Rgb {
            r: 0xff,
            g: 0xff,
            b: 0xff,
        };
        let selection_background_rgb = Rgb {
            r: 0x32,
            g: 0x3a,
            b: 0x4d,
        };
        let search_rgb = Rgb {
            r: 0x18,
            g: 0x18,
            b: 0x18,
        };
        let search_background_rgb = Rgb {
            r: 0xac,
            g: 0x42,
            b: 0x42,
        };
        let focused_background_rgb = Rgb {
            r: 0xf4,
            g: 0xbf,
            b: 0x75,
        };

        Self {
            ansi_colors,
            ansi_rgb,
            extended_colors,
            extended_rgb,
            foreground: rgb_to_hsla(foreground_rgb),
            foreground_rgb,
            background: rgb_to_hsla(background_rgb),
            background_rgb,
            cursor: rgb_to_hsla(cursor_rgb),
            cursor_rgb,
            selection_background: rgb_to_hsla(selection_background_rgb),
            selection_background_rgb,
            selection_foreground: None,
            selection_foreground_rgb: None,
            cursor_text: None,
            cursor_text_rgb: None,
            search_foreground: rgb_to_hsla(search_rgb),
            search_background: rgb_to_hsla(search_background_rgb),
            focused_search_foreground: rgb_to_hsla(search_rgb),
            focused_search_background: rgb_to_hsla(focused_background_rgb),
            hint_start_foreground: rgb_to_hsla(search_rgb),
            hint_start_background: rgb_to_hsla(focused_background_rgb),
            hint_end_foreground: rgb_to_hsla(search_rgb),
            hint_end_background: rgb_to_hsla(search_background_rgb),
        }
    }
}

impl ColorPalette {
    /// Creates a new color palette with default colors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new color palette builder for customizing colors.
    ///
    /// # Example
    ///
    /// ```
    /// use yttt_terminal::ColorPalette;
    ///
    /// let palette = ColorPalette::builder()
    ///     .background(0x16, 0x16, 0x17)
    ///     .foreground(0xC9, 0xC7, 0xCD)
    ///     .black(0x10, 0x10, 0x10)
    ///     .red(0xEF, 0xA6, 0xA2)
    ///     .build();
    /// ```
    pub fn builder() -> ColorPaletteBuilder {
        ColorPaletteBuilder::new()
    }

    /// Resolves a terminal color to a GPUI Hsla color.
    ///
    /// This method handles all terminal color types:
    /// - Named ANSI colors (0-15)
    /// - 256-color palette colors
    /// - True color (RGB) values
    ///
    /// # Arguments
    ///
    /// * `color` - The terminal color to resolve
    /// * `colors` - Optional color overrides from the terminal configuration
    ///
    /// # Returns
    ///
    /// The resolved Hsla color suitable for use with GPUI
    pub fn resolve(&self, color: Color, colors: &Colors) -> Hsla {
        match color {
            Color::Named(named) => {
                // Check if there's a custom color override first
                if let Some(rgb) = colors[named] {
                    return rgb_to_hsla(rgb);
                }

                // Handle different named color types
                let idx = named as usize;
                if idx < 16 {
                    // Standard ANSI colors (0-15)
                    self.ansi_colors[idx]
                } else {
                    // Special colors (Foreground, Background, Cursor, etc.)
                    match named {
                        NamedColor::Foreground => self.foreground,
                        NamedColor::Background => self.background,
                        NamedColor::Cursor => self.cursor,
                        NamedColor::DimForeground => {
                            // Dimmed version of foreground
                            let mut dim = self.foreground;
                            dim.l *= 0.7;
                            dim
                        }
                        NamedColor::BrightForeground => {
                            // Brighter version of foreground
                            let mut bright = self.foreground;
                            bright.l = (bright.l * 1.2).min(1.0);
                            bright
                        }
                        NamedColor::DimBlack
                        | NamedColor::DimRed
                        | NamedColor::DimGreen
                        | NamedColor::DimYellow
                        | NamedColor::DimBlue
                        | NamedColor::DimMagenta
                        | NamedColor::DimCyan
                        | NamedColor::DimWhite => {
                            // Dim variant - calculate base color index and dim it
                            let base_idx = match named {
                                NamedColor::DimBlack => 0,
                                NamedColor::DimRed => 1,
                                NamedColor::DimGreen => 2,
                                NamedColor::DimYellow => 3,
                                NamedColor::DimBlue => 4,
                                NamedColor::DimMagenta => 5,
                                NamedColor::DimCyan => 6,
                                NamedColor::DimWhite => 7,
                                _ => 7,
                            };
                            let mut dim = self.ansi_colors[base_idx];
                            dim.l *= 0.7;
                            dim
                        }
                        _ => self.foreground, // Fallback for any other special colors
                    }
                }
            }
            Color::Spec(rgb) => {
                // True color (24-bit RGB)
                rgb_to_hsla(rgb)
            }
            Color::Indexed(idx) => {
                // 256-color mode
                self.extended_colors[idx as usize]
            }
        }
    }

    /// Resolve an OSC color query without converting configured RGB through HSL.
    pub fn query_rgb(&self, index: usize, colors: &Colors) -> Option<Rgb> {
        if index <= NamedColor::DimForeground as usize {
            if let Some(rgb) = colors[index] {
                return Some(rgb);
            }
        }

        match index {
            0..=255 => Some(self.extended_rgb[index]),
            index if index == NamedColor::Foreground as usize => Some(self.foreground_rgb),
            index if index == NamedColor::Background as usize => Some(self.background_rgb),
            index if index == NamedColor::Cursor as usize => Some(self.cursor_rgb),
            index if index == NamedColor::BrightForeground as usize => Some(self.foreground_rgb),
            index if index == NamedColor::DimForeground as usize => {
                Some(dim_rgb(self.foreground_rgb))
            }
            index
                if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                    .contains(&index) =>
            {
                Some(dim_rgb(
                    self.ansi_rgb[index - NamedColor::DimBlack as usize],
                ))
            }
            _ => None,
        }
    }

    /// Gets a reference to the ANSI color palette.
    pub fn ansi_colors(&self) -> &[Hsla; 16] {
        &self.ansi_colors
    }

    /// Gets a reference to the full 256-color palette.
    pub fn extended_colors(&self) -> &[Hsla; 256] {
        &self.extended_colors
    }

    /// Gets the default foreground color.
    pub fn foreground(&self) -> Hsla {
        self.foreground
    }

    /// Gets the default background color.
    pub fn background(&self) -> Hsla {
        self.background
    }

    /// Gets the default cursor color.
    pub fn cursor(&self) -> Hsla {
        self.cursor
    }

    /// Gets the selection background color.
    pub fn selection_background(&self) -> Hsla {
        self.selection_background
    }

    pub fn selection_foreground(&self) -> Option<Hsla> {
        self.selection_foreground
    }

    pub fn cursor_text(&self) -> Option<Hsla> {
        self.cursor_text
    }

    pub fn search_colors(&self) -> (Hsla, Hsla) {
        (self.search_foreground, self.search_background)
    }

    pub fn focused_search_colors(&self) -> (Hsla, Hsla) {
        (
            self.focused_search_foreground,
            self.focused_search_background,
        )
    }

    pub fn hint_start_colors(&self) -> (Hsla, Hsla) {
        (self.hint_start_foreground, self.hint_start_background)
    }

    pub fn hint_end_colors(&self) -> (Hsla, Hsla) {
        (self.hint_end_foreground, self.hint_end_background)
    }
}

fn dim_rgb(rgb: Rgb) -> Rgb {
    Rgb {
        r: ((rgb.r as f32) * 0.7).round() as u8,
        g: ((rgb.g as f32) * 0.7).round() as u8,
        b: ((rgb.b as f32) * 0.7).round() as u8,
    }
}
/// Converts an RGB color to GPUI's Hsla color format.
///
/// # Arguments
///
/// * `rgb` - The RGB color to convert (each component is 0-255)
///
/// # Returns
///
/// The converted Hsla color with full opacity
fn rgb_to_hsla(rgb: Rgb) -> Hsla {
    // Normalize RGB values to 0.0-1.0 range
    let r = rgb.r as f32 / 255.0;
    let g = rgb.g as f32 / 255.0;
    let b = rgb.b as f32 / 255.0;

    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    // Calculate lightness
    let l = (max + min) / 2.0;

    // Calculate saturation
    let s = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * l - 1.0).abs())
    };

    // Calculate hue
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };

    // Normalize hue to 0.0-1.0 range (GPUI uses normalized values)
    let h = if h < 0.0 { h + 360.0 } else { h } / 360.0;

    Hsla {
        h,
        s,
        l,
        a: 1.0, // Full opacity
    }
}

/// Builder for creating a customized color palette.
///
/// Start with default colors and override specific ones as needed.
#[derive(Debug, Clone)]
pub struct ColorPaletteBuilder {
    palette: ColorPalette,
}

impl Default for ColorPaletteBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorPaletteBuilder {
    /// Creates a new builder with default colors.
    pub fn new() -> Self {
        Self {
            palette: ColorPalette::default(),
        }
    }

    /// Sets the background color.
    pub fn background(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.background = rgb_to_hsla(rgb);
        self.palette.background_rgb = rgb;
        self
    }

    /// Sets the foreground color.
    pub fn foreground(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.foreground = rgb_to_hsla(rgb);
        self.palette.foreground_rgb = rgb;
        self
    }

    /// Sets the cursor color.
    pub fn cursor(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.cursor = rgb_to_hsla(rgb);
        self.palette.cursor_rgb = rgb;
        self
    }

    /// Sets the selection background color.
    pub fn selection_background(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.selection_background = rgb_to_hsla(rgb);
        self.palette.selection_background_rgb = rgb;
        self
    }

    pub fn selection_foreground(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.selection_foreground = Some(rgb_to_hsla(rgb));
        self.palette.selection_foreground_rgb = Some(rgb);
        self
    }

    pub fn cursor_text(mut self, r: u8, g: u8, b: u8) -> Self {
        let rgb = Rgb { r, g, b };
        self.palette.cursor_text = Some(rgb_to_hsla(rgb));
        self.palette.cursor_text_rgb = Some(rgb);
        self
    }

    pub fn search(mut self, foreground: (u8, u8, u8), background: (u8, u8, u8)) -> Self {
        self.palette.search_foreground = rgb_to_hsla(Rgb {
            r: foreground.0,
            g: foreground.1,
            b: foreground.2,
        });
        self.palette.search_background = rgb_to_hsla(Rgb {
            r: background.0,
            g: background.1,
            b: background.2,
        });
        self
    }

    pub fn focused_search(mut self, foreground: (u8, u8, u8), background: (u8, u8, u8)) -> Self {
        self.palette.focused_search_foreground = rgb_to_hsla(Rgb {
            r: foreground.0,
            g: foreground.1,
            b: foreground.2,
        });
        self.palette.focused_search_background = rgb_to_hsla(Rgb {
            r: background.0,
            g: background.1,
            b: background.2,
        });
        self
    }

    pub fn hint_start(mut self, foreground: (u8, u8, u8), background: (u8, u8, u8)) -> Self {
        self.palette.hint_start_foreground = rgb_to_hsla(Rgb {
            r: foreground.0,
            g: foreground.1,
            b: foreground.2,
        });
        self.palette.hint_start_background = rgb_to_hsla(Rgb {
            r: background.0,
            g: background.1,
            b: background.2,
        });
        self
    }

    pub fn hint_end(mut self, foreground: (u8, u8, u8), background: (u8, u8, u8)) -> Self {
        self.palette.hint_end_foreground = rgb_to_hsla(Rgb {
            r: foreground.0,
            g: foreground.1,
            b: foreground.2,
        });
        self.palette.hint_end_background = rgb_to_hsla(Rgb {
            r: background.0,
            g: background.1,
            b: background.2,
        });
        self
    }

    /// Sets color 0 (black).
    pub fn black(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(0, r, g, b);
        self
    }

    /// Sets color 1 (red).
    pub fn red(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(1, r, g, b);
        self
    }

    /// Sets color 2 (green).
    pub fn green(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(2, r, g, b);
        self
    }

    /// Sets color 3 (yellow).
    pub fn yellow(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(3, r, g, b);
        self
    }

    /// Sets color 4 (blue).
    pub fn blue(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(4, r, g, b);
        self
    }

    /// Sets color 5 (magenta).
    pub fn magenta(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(5, r, g, b);
        self
    }

    /// Sets color 6 (cyan).
    pub fn cyan(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(6, r, g, b);
        self
    }

    /// Sets color 7 (white).
    pub fn white(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(7, r, g, b);
        self
    }

    /// Sets color 8 (bright black).
    pub fn bright_black(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(8, r, g, b);
        self
    }

    /// Sets color 9 (bright red).
    pub fn bright_red(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(9, r, g, b);
        self
    }

    /// Sets color 10 (bright green).
    pub fn bright_green(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(10, r, g, b);
        self
    }

    /// Sets color 11 (bright yellow).
    pub fn bright_yellow(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(11, r, g, b);
        self
    }

    /// Sets color 12 (bright blue).
    pub fn bright_blue(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(12, r, g, b);
        self
    }

    /// Sets color 13 (bright magenta).
    pub fn bright_magenta(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(13, r, g, b);
        self
    }

    /// Sets color 14 (bright cyan).
    pub fn bright_cyan(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(14, r, g, b);
        self
    }

    /// Sets color 15 (bright white).
    pub fn bright_white(mut self, r: u8, g: u8, b: u8) -> Self {
        self.set_ansi_color(15, r, g, b);
        self
    }

    /// Sets an ANSI color by index (0-15).
    fn set_ansi_color(&mut self, idx: usize, r: u8, g: u8, b: u8) {
        let rgb = Rgb { r, g, b };
        let color = rgb_to_hsla(rgb);
        self.palette.ansi_rgb[idx] = rgb;
        self.palette.extended_rgb[idx] = rgb;
        self.palette.ansi_colors[idx] = color;
        self.palette.extended_colors[idx] = color;
    }

    /// Builds the color palette.
    pub fn build(self) -> ColorPalette {
        self.palette
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_to_hsla_black() {
        let rgb = Rgb { r: 0, g: 0, b: 0 };
        let hsla = rgb_to_hsla(rgb);
        assert_eq!(hsla.l, 0.0);
        assert_eq!(hsla.s, 0.0);
        assert_eq!(hsla.a, 1.0);
    }

    #[test]
    fn test_rgb_to_hsla_white() {
        let rgb = Rgb {
            r: 255,
            g: 255,
            b: 255,
        };
        let hsla = rgb_to_hsla(rgb);
        assert_eq!(hsla.l, 1.0);
        assert_eq!(hsla.s, 0.0);
        assert_eq!(hsla.a, 1.0);
    }

    #[test]
    fn test_rgb_to_hsla_red() {
        let rgb = Rgb { r: 255, g: 0, b: 0 };
        let hsla = rgb_to_hsla(rgb);
        assert_eq!(hsla.h, 0.0);
        assert_eq!(hsla.s, 1.0);
        assert_eq!(hsla.a, 1.0);
    }

    #[test]
    fn test_color_palette_default() {
        let palette = ColorPalette::default();
        assert_eq!(palette.ansi_colors.len(), 16);
        assert_eq!(palette.extended_colors.len(), 256);
    }

    #[test]
    fn test_resolve_named_color() {
        use alacritty_terminal::vte::ansi::NamedColor;

        let palette = ColorPalette::new();
        let colors = Colors::default();
        let hsla = palette.resolve(Color::Named(NamedColor::Red), &colors);
        assert!(hsla.a > 0.0); // Should have some opacity
    }

    #[test]
    fn test_resolve_indexed_color() {
        let palette = ColorPalette::new();
        let colors = Colors::default();
        let hsla = palette.resolve(Color::Indexed(42), &colors);
        assert_eq!(hsla.a, 1.0);
    }

    #[test]
    fn test_resolve_spec_color() {
        let palette = ColorPalette::new();
        let colors = Colors::default();
        let rgb = Rgb {
            r: 128,
            g: 64,
            b: 192,
        };
        let hsla = palette.resolve(Color::Spec(rgb), &colors);
        assert_eq!(hsla.a, 1.0);
    }
    #[test]
    fn color_queries_preserve_configured_rgb_and_prefer_dynamic_overrides() {
        let palette = ColorPalette::builder()
            .foreground(1, 2, 3)
            .background(4, 5, 6)
            .red(7, 8, 9)
            .build();
        let mut colors = Colors::default();

        assert_eq!(
            palette.query_rgb(1, &colors),
            Some(Rgb { r: 7, g: 8, b: 9 })
        );
        assert_eq!(
            palette.query_rgb(NamedColor::Foreground as usize, &colors),
            Some(Rgb { r: 1, g: 2, b: 3 })
        );

        colors[NamedColor::Foreground] = Some(Rgb {
            r: 10,
            g: 11,
            b: 12,
        });
        assert_eq!(
            palette.query_rgb(NamedColor::Foreground as usize, &colors),
            Some(Rgb {
                r: 10,
                g: 11,
                b: 12
            })
        );
    }
}
