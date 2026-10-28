//! Fechas a partir de segundos unix, en UTC, sin dependencias.

/// (año, mes, día, hora, minuto, segundo) en UTC.
pub fn civil(t: f64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = t.floor() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    // Algoritmo days_from_civil inverso de Howard Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `2026-10` — clave del archivo mensual de historial.
pub fn month_key(t: f64) -> String {
    let (y, m, ..) = civil(t);
    format!("{y:04}-{m:02}")
}

/// `2026-10-27T21:16:05Z`
pub fn iso(t: f64) -> String {
    let (y, mo, d, h, mi, s) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `2026-10-27`
pub fn date(t: f64) -> String {
    let (y, m, d, ..) = civil(t);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Duración legible: `45 s`, `12 min`, `3 h 05 min`, `2 d 4 h`.
pub fn human(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        3600..86_400 => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
        _ => format!("{} d {} h", s / 86_400, s % 86_400 / 3600),
    }
}

/// Hace cuánto, en una sola unidad: `ahora`, `12 min`, `3 h`, `2 d`.
pub fn ago(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..60 => "ahora".into(),
        60..3600 => format!("hace {} min", s / 60),
        3600..86_400 => format!("hace {} h", s / 3600),
        _ => format!("hace {} d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(iso(0.0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_793_134_565.0), "2026-10-27T20:56:05Z");
        assert_eq!(month_key(951_782_400.0), "2000-02"); // 2000-02-29
        assert_eq!(date(951_782_400.0), "2000-02-29");
    }

    #[test]
    fn durations() {
        assert_eq!(human(42.0), "42 s");
        assert_eq!(human(600.0), "10 min");
        assert_eq!(human(3.0 * 3600.0 + 300.0), "3 h 05 min");
        assert_eq!(human(2.0 * 86_400.0 + 7200.0), "2 d 2 h");
        assert_eq!(ago(30.0), "ahora");
        assert_eq!(ago(6840.0), "hace 1 h");
    }
}
