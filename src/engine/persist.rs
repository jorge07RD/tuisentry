//! Historial en disco: un JSONL por servicio y mes en
//! `~/.local/state/tuisentry/history/<servicio>/<AAAA-MM>.jsonl`, más
//! `incidents.jsonl` con los incidentes cerrados. Solo se añade al final.

use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{store::Sample, text::fold, time};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Incident {
    pub service: String,
    pub start: f64,
    /// `None` = sigue abierto.
    pub end: Option<f64>,
    pub cause: String,
}

/// Una línea del JSONL; `d` solo se escribe cuando el detalle cambia.
#[derive(Serialize, Deserialize)]
struct Rec {
    t: f64,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    v: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    d: Option<String>,
}

struct Writer {
    month: String,
    out: BufWriter<File>,
    last_detail: String,
}

pub struct Persist {
    dir: PathBuf,
    writers: HashMap<String, Writer>,
}

/// Nombre de servicio apto para carpeta.
pub fn slug(name: &str) -> String {
    let s: String = fold(name)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "servicio".into() } else { s }
}

impl Persist {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            writers: HashMap::new(),
        }
    }

    fn service_dir(&self, service: &str) -> PathBuf {
        self.dir.join(slug(service))
    }

    pub fn append(&mut self, service: &str, s: &Sample) -> io::Result<()> {
        let month = time::month_key(s.at);
        let reopen = self.writers.get(service).is_none_or(|w| w.month != month);
        if reopen {
            let dir = self.service_dir(service);
            fs::create_dir_all(&dir)?;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{month}.jsonl")))?;
            self.writers.insert(
                service.into(),
                Writer {
                    month,
                    out: BufWriter::new(file),
                    // Archivo nuevo o reabierto: repetir el detalle una vez.
                    last_detail: String::new(),
                },
            );
        }
        let w = self.writers.get_mut(service).expect("recién insertado");
        let d = (s.detail != w.last_detail).then(|| s.detail.clone());
        w.last_detail.clone_from(&s.detail);
        let rec = Rec {
            t: s.at,
            ok: s.ok,
            v: s.value,
            d,
        };
        serde_json::to_writer(&mut w.out, &rec)?;
        w.out.write_all(b"\n")
    }

    pub fn flush(&mut self) {
        for w in self.writers.values_mut() {
            let _ = w.out.flush();
        }
    }

    pub fn incident(&mut self, inc: &Incident) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("incidents.jsonl"))?;
        serde_json::to_writer(&mut f, inc)?;
        f.write_all(b"\n")
    }

    /// Se olvidan los archivos abiertos (p. ej. tras renombrar servicios).
    pub fn close_all(&mut self) {
        self.flush();
        self.writers.clear();
    }
}

/// Muestras de `service` con `at >= since`, en orden.
pub fn load(dir: &Path, service: &str, since: f64) -> Vec<Sample> {
    let sdir = dir.join(slug(service));
    let first = time::month_key(since);
    let mut months: Vec<PathBuf> = fs::read_dir(&sdir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|m| m >= first.as_str())
        })
        .collect();
    months.sort();

    let mut out = vec![];
    for path in months {
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let mut detail = String::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(rec) = serde_json::from_str::<Rec>(&line) else {
                continue;
            };
            if let Some(d) = rec.d {
                detail = d;
            }
            if rec.t >= since {
                out.push(Sample {
                    at: rec.t,
                    ok: rec.ok,
                    value: rec.v,
                    detail: detail.clone(),
                    ..Default::default()
                });
            }
        }
    }
    out
}

/// Incidentes cerrados más recientes primero.
pub fn incidents(dir: &Path, limit: usize) -> Vec<Incident> {
    let Ok(file) = File::open(dir.join("incidents.jsonl")) else {
        return vec![];
    };
    let mut all: Vec<Incident> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect();
    all.reverse();
    all.truncate(limit);
    all
}

/// Borra los meses enteramente anteriores a `before`.
pub fn prune(dir: &Path, before: f64) {
    let keep_from = time::month_key(before);
    for sdir in fs::read_dir(dir).into_iter().flatten().flatten() {
        for f in fs::read_dir(sdir.path()).into_iter().flatten().flatten() {
            let p = f.path();
            let old = p.file_stem().and_then(|s| s.to_str()).is_some_and(|m| {
                p.extension().is_some_and(|e| e == "jsonl") && m < keep_from.as_str()
            });
            if old {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// Fracción OK de las muestras desde `since`; `None` sin datos.
pub fn uptime(samples: &[Sample], since: f64) -> Option<f64> {
    let (ok, n) = samples
        .iter()
        .filter(|s| s.at >= since)
        .fold((0usize, 0usize), |(ok, n), s| {
            (ok + usize::from(s.ok), n + 1)
        });
    (n > 0).then(|| ok as f64 / n as f64)
}

/// Escribe las muestras como CSV (hora UTC, ok, valor, detalle).
pub fn write_csv(path: &Path, samples: &[Sample]) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    writeln!(out, "time_utc,ok,value,detail")?;
    for s in samples {
        let value = s.value.map_or(String::new(), |v| format!("{v:.3}"));
        let detail = s.detail.replace('"', "\"\"");
        writeln!(out, "{},{},{},\"{}\"", time::iso(s.at), s.ok, value, detail)?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tuisentry-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn s(at: f64, ok: bool, detail: &str) -> Sample {
        Sample {
            at,
            ok,
            value: ok.then_some(12.5),
            detail: detail.into(),
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip_and_detail_fill() {
        let dir = tmp("roundtrip");
        let mut p = Persist::new(dir.clone());
        // Dos meses distintos: 2026-09-30 y 2026-10-01.
        p.append("Mi API", &s(1_790_726_400.0, true, "HTTP 200"))
            .unwrap();
        p.append("Mi API", &s(1_790_812_800.0, true, "HTTP 200"))
            .unwrap();
        p.append("Mi API", &s(1_790_812_860.0, false, "timeout"))
            .unwrap();
        p.flush();

        let all = load(&dir, "Mi API", 0.0);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].detail, "HTTP 200");
        assert_eq!(all[2].detail, "timeout");
        assert_eq!(all[2].value, None);
        assert_eq!(load(&dir, "Mi API", 1_790_812_800.0).len(), 2);
        assert_eq!(uptime(&all, 0.0), Some(2.0 / 3.0));

        // El mes de septiembre desaparece al podar desde octubre.
        prune(&dir, 1_790_812_800.0);
        assert_eq!(load(&dir, "Mi API", 0.0).len(), 2);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn incidents_newest_first() {
        let dir = tmp("incidents");
        let mut p = Persist::new(dir.clone());
        for i in 0..3 {
            p.incident(&Incident {
                service: "a".into(),
                start: f64::from(i),
                end: Some(f64::from(i) + 1.0),
                cause: "x".into(),
            })
            .unwrap();
        }
        let got = incidents(&dir, 2);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].start, 2.0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("DGII e-CF API"), "dgii-e-cf-api");
        assert_eq!(slug("Disco /"), "disco");
        assert_eq!(slug("///"), "servicio");
        assert_eq!(slug("Caído"), "caido");
    }
}
