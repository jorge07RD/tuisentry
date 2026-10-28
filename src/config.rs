use std::{collections::HashSet, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub theme: ThemeCfg,
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
    #[serde(default, rename = "service")]
    pub services: Vec<ServiceCfg>,
    #[serde(default, rename = "page")]
    pub pages: Vec<PageCfg>,
    #[serde(default)]
    pub history: HistoryCfg,
    #[serde(default)]
    pub alerts: AlertsCfg,
}

#[derive(Debug, Default, Deserialize)]
pub struct ThemeCfg {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCfg {
    /// Cuánto se guarda en disco, p. ej. `90d`.
    #[serde(default = "default_retention")]
    pub retention: String,
}

impl Default for HistoryCfg {
    fn default() -> Self {
        Self {
            retention: default_retention(),
        }
    }
}

fn default_retention() -> String {
    "90d".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertsCfg {
    /// Fallos seguidos antes de declarar un servicio caído.
    #[serde(default = "default_fail_after")]
    pub fail_after: u32,
    /// Latencia (ms) a partir de la cual se avisa de lentitud; sin valor, no se avisa.
    pub slow_ms: Option<u64>,
    /// Avisa cuando al certificado TLS le quedan estos días o menos.
    #[serde(default = "default_cert_days")]
    pub cert_days: i64,
    #[serde(default, rename = "notify")]
    pub channels: Vec<ChannelCfg>,
}

impl Default for AlertsCfg {
    fn default() -> Self {
        Self {
            fail_after: default_fail_after(),
            slow_ms: None,
            cert_days: default_cert_days(),
            channels: vec![],
        }
    }
}

fn default_fail_after() -> u32 {
    2
}

fn default_cert_days() -> i64 {
    14
}

/// Canal de notificación: `desktop` o `ntfy`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelCfg {
    pub kind: String,
    /// ntfy: tema al que te suscribes en la app.
    pub topic: Option<String>,
    /// ntfy: servidor, por defecto `https://ntfy.sh`.
    pub server: Option<String>,
    /// ntfy: token de acceso para temas protegidos.
    pub token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    pub name: String,
    /// Destino ssh, p. ej. `jorge@vps.example.com` o un alias de ~/.ssh/config.
    pub ssh: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceCfg {
    pub name: String,
    pub kind: String,
    /// URL (http), `host:puerto` (tcp) o host/IP (ping).
    pub target: Option<String>,
    /// Host de `[[host]]` donde se ejecuta (systemd, cmd).
    pub host: Option<String>,
    /// Unidad systemd, p. ej. `nginx.service`.
    pub unit: Option<String>,
    /// Comando de shell para `cmd`; su stdout debe empezar por un número.
    pub command: Option<String>,
    /// Sufijo que se muestra junto al valor (por defecto `ms` en checks de latencia).
    pub suffix: Option<String>,
    /// Link directo al recurso; en `http` por defecto es el propio `target`.
    pub link: Option<String>,
    /// Status HTTP esperado; si falta, cualquier código < 400 es OK.
    pub expect_status: Option<u16>,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    /// Umbral de lentitud propio; si falta, se usa `[alerts] slow_ms`.
    pub slow_ms: Option<u64>,
    /// Empieza silenciado (sin alertas); se puede cambiar desde el menú.
    #[serde(default)]
    pub mute: bool,
}

fn default_interval() -> String {
    "30s".into()
}

fn default_timeout() -> String {
    "10s".into()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageCfg {
    pub title: String,
    #[serde(default, rename = "panel")]
    pub panels: Vec<PanelCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelCfg {
    pub kind: String,
    pub title: Option<String>,
    /// Servicios a mostrar por nombre; vacío = todos (o los del `host`).
    #[serde(default)]
    pub services: Vec<String>,
    pub host: Option<String>,
    /// Ventana de tiempo para gráficas, p. ej. `15m`.
    pub window: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("no pude leer {}", path.display()))?;
        let mut cfg: Config = toml::from_str(&raw)
            .with_context(|| format!("config inválida en {}", path.display()))?;
        if cfg.pages.is_empty() {
            cfg.pages.push(PageCfg {
                title: "Overview".into(),
                panels: ["status", "chart"]
                    .map(|kind| PanelCfg {
                        kind: kind.into(),
                        title: None,
                        services: vec![],
                        host: None,
                        window: None,
                    })
                    .to_vec(),
            });
        }
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.services.is_empty() {
            bail!("no hay ningún [[service]] definido");
        }
        parse_duration(&self.history.retention).context("[history] retention")?;
        for c in &self.alerts.channels {
            match c.kind.as_str() {
                "desktop" => {}
                "ntfy" if c.topic.as_deref().is_some_and(|t| !t.is_empty()) => {}
                "ntfy" => bail!("[[alerts.notify]] ntfy necesita `topic`"),
                other => bail!("[[alerts.notify]] kind desconocido {other:?} (desktop, ntfy)"),
            }
        }
        let hosts: HashSet<_> = self.hosts.iter().map(|h| h.name.as_str()).collect();
        let mut names = HashSet::new();
        for s in &self.services {
            if !names.insert(s.name.as_str()) {
                bail!("servicio duplicado: {}", s.name);
            }
            if let Some(h) = &s.host
                && !hosts.contains(h.as_str())
            {
                bail!("servicio {}: host desconocido {h}", s.name);
            }
            parse_duration(&s.interval).with_context(|| format!("servicio {}", s.name))?;
            parse_duration(&s.timeout).with_context(|| format!("servicio {}", s.name))?;
        }
        for p in &self.pages {
            for panel in &p.panels {
                for n in &panel.services {
                    if !names.contains(n.as_str()) {
                        bail!("página {}: servicio desconocido {n}", p.title);
                    }
                }
                if let Some(h) = &panel.host
                    && !hosts.contains(h.as_str())
                {
                    bail!("página {}: host desconocido {h}", p.title);
                }
                if let Some(w) = &panel.window {
                    parse_duration(w).with_context(|| format!("página {}", p.title))?;
                }
            }
        }
        Ok(())
    }

    pub fn host(&self, name: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.name == name)
    }
}

/// Acepta `500ms`, `30s`, `15m`, `2h`.
pub fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: u64 = num
        .parse()
        .with_context(|| format!("duración inválida: {s:?}"))?;
    Ok(match unit {
        "ms" => Duration::from_millis(n),
        "s" | "" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 3600),
        "d" => Duration::from_secs(n * 86_400),
        _ => bail!("unidad de duración inválida en {s:?} (usa ms, s, m, h o d)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("15m").unwrap(), Duration::from_secs(900));
        assert_eq!(
            parse_duration("3d").unwrap(),
            Duration::from_secs(3 * 86_400)
        );
        assert!(parse_duration("3w").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn parses_example_and_validates() {
        let cfg: Config = toml::from_str(include_str!("../config.example.toml")).unwrap();
        cfg.validate().unwrap();
        assert!(!cfg.pages.is_empty());
    }

    #[test]
    fn rejects_unknown_service_in_panel() {
        let cfg: Config = toml::from_str(
            r#"
            [[service]]
            name = "a"
            kind = "tcp"
            target = "localhost:22"
            [[page]]
            title = "x"
            [[page.panel]]
            kind = "chart"
            services = ["b"]
            "#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }
}
