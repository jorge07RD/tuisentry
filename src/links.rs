use std::{
    io::{self, Write},
    process::{Command, Stdio},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use ratatui::crossterm::{
    cursor::MoveTo,
    queue,
    style::{Attribute, Print, ResetColor, SetAttribute, SetForegroundColor},
};

use crate::input::LinkSpan;

/// Abre el recurso y devuelve el mensaje para el toast.
///
/// En modo web `xdg-open` abriría el navegador del PC, no el del celular, así que
/// solo copiamos (OSC 52) y mostramos la URL para tocarla con «Mouse libre».
pub fn open(url: &str, web: bool) -> String {
    if web {
        let _ = copy_osc52(url);
        return format!("↗ {url} · copiado — usa «Mouse libre» y tócalo");
    }
    let child = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut c) => {
            std::thread::spawn(move || c.wait());
            format!("↗ abriendo {url}")
        }
        Err(e) => format!("no pude abrir {url}: {e}"),
    }
}

fn copy_osc52(text: &str) -> io::Result<()> {
    let mut out = io::stdout();
    write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
    out.flush()
}

/// Reescribe el texto ya dibujado envuelto en OSC 8 para que la terminal
/// (y xterm.js en ttyd) lo trate como link clicable.
pub fn emit_hyperlinks(spans: &[LinkSpan]) -> io::Result<()> {
    if spans.is_empty() {
        return Ok(());
    }
    let mut out = io::stdout();
    for s in spans {
        queue!(
            out,
            MoveTo(s.pos.x, s.pos.y),
            SetForegroundColor(to_crossterm(s.fg)),
            SetAttribute(Attribute::Underlined),
            Print(format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", s.url, s.text)),
            SetAttribute(Attribute::Reset),
            ResetColor,
        )?;
    }
    out.flush()
}

fn to_crossterm(c: ratatui::style::Color) -> ratatui::crossterm::style::Color {
    use ratatui::crossterm::style::Color as C;
    match c {
        ratatui::style::Color::Rgb(r, g, b) => C::Rgb { r, g, b },
        _ => C::Reset,
    }
}
