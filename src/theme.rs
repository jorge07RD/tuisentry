use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub fg: Color,
    pub dim: Color,
    pub border: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    /// Fondo de popups, botones y pestaña activa; el resto usa el fondo de la terminal.
    pub surface: Color,
    pub series: [Color; 6],
}

const fn hex(c: u32) -> Color {
    Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

impl Theme {
    pub const NAMES: [&str; 3] = ["tokyonight", "catppuccin", "gruvbox"];

    pub fn by_name(name: Option<&str>) -> Self {
        match name.unwrap_or("tokyonight") {
            "catppuccin" => Self {
                fg: hex(0xcdd6f4),
                dim: hex(0x6c7086),
                border: hex(0x45475a),
                accent: hex(0x89b4fa),
                ok: hex(0xa6e3a1),
                warn: hex(0xf9e2af),
                err: hex(0xf38ba8),
                surface: hex(0x313244),
                series: [0x89b4fa, 0xcba6f7, 0x94e2d5, 0xa6e3a1, 0xfab387, 0xf5c2e7].map(hex),
            },
            "gruvbox" => Self {
                fg: hex(0xebdbb2),
                dim: hex(0x928374),
                border: hex(0x504945),
                accent: hex(0x83a598),
                ok: hex(0xb8bb26),
                warn: hex(0xfabd2f),
                err: hex(0xfb4934),
                surface: hex(0x3c3836),
                series: [0x83a598, 0xd3869b, 0x8ec07c, 0xb8bb26, 0xfe8019, 0xfabd2f].map(hex),
            },
            _ => Self {
                fg: hex(0xc0caf5),
                dim: hex(0x565f89),
                border: hex(0x3b4261),
                accent: hex(0x7aa2f7),
                ok: hex(0x9ece6a),
                warn: hex(0xe0af68),
                err: hex(0xf7768e),
                surface: hex(0x292e42),
                series: [0x7aa2f7, 0xbb9af7, 0x7dcfff, 0x9ece6a, 0xe0af68, 0xff9e64].map(hex),
            },
        }
    }

    pub fn series(&self, i: usize) -> Color {
        self.series[i % self.series.len()]
    }
}
