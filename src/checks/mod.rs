//! Tipos de check. Para añadir uno nuevo: implementa [`Check`] en un módulo
//! y regístralo por su `kind` en [`build`].

mod cmd;
mod http;
mod ping;
mod systemd;
mod tcp;

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use tokio::{
    sync::{Notify, mpsc},
    task::JoinHandle,
    time::MissedTickBehavior,
};

use crate::{
    config::{Config, ServiceCfg, parse_duration},
    store::{Sample, unix_now},
};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Resultado de una ejecución de un check.
pub struct Outcome {
    pub ok: bool,
    pub value: Option<f64>,
    pub detail: String,
}

impl Outcome {
    pub fn up(value: Option<f64>, detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            value,
            detail: detail.into(),
        }
    }

    pub fn down(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            detail: detail.into(),
        }
    }
}

pub trait Check: Send + Sync {
    /// Ejecuta el check una vez. El timeout lo aplica quien llama.
    fn run(&self) -> BoxFut<'_, Outcome>;

    /// Link al recurso cuando el servicio no define `link`.
    fn default_link(&self) -> Option<String> {
        None
    }

    /// Sufijo del valor cuando el servicio no define `suffix`.
    fn default_suffix(&self) -> &'static str {
        "ms"
    }
}

/// Registro de tipos de check por `kind`.
pub fn build(svc: &ServiceCfg, cfg: &Config) -> Result<Box<dyn Check>> {
    let ssh = match &svc.host {
        Some(h) => cfg.host(h).map(|h| h.ssh.clone()),
        None => None,
    };
    let need = |field: &Option<String>, what: &str| -> Result<String> {
        match field {
            Some(v) => Ok(v.clone()),
            None => bail!("servicio {} ({}): falta `{what}`", svc.name, svc.kind),
        }
    };
    Ok(match svc.kind.as_str() {
        "http" => Box::new(http::Http::new(
            need(&svc.target, "target")?,
            svc.expect_status,
        )?),
        "tcp" => Box::new(tcp::Tcp {
            addr: need(&svc.target, "target")?,
        }),
        "ping" => Box::new(ping::Ping {
            host: need(&svc.target, "target")?,
        }),
        "systemd" => Box::new(systemd::Systemd {
            unit: need(&svc.unit, "unit")?,
            ssh,
        }),
        "cmd" => Box::new(cmd::Cmd {
            command: need(&svc.command, "command")?,
            ssh,
        }),
        other => bail!(
            "servicio {}: kind desconocido {other:?} (http, tcp, ping, systemd, cmd)",
            svc.name
        ),
    })
}

/// Muestra etiquetada con la generación de config que la produjo y el índice
/// del servicio, para descartar las que lleguen tarde tras una recarga.
pub type Tagged = (u64, usize, Sample);

/// Controles compartidos con el bucle de un servicio.
#[derive(Clone, Default)]
pub struct Ctl {
    /// Fuerza un check inmediato (también estando en pausa).
    pub now: Arc<Notify>,
    pub paused: Arc<AtomicBool>,
}

/// Lanza el bucle de un servicio. Abortar el `JoinHandle` lo detiene.
pub fn spawn(
    (generation, idx): (u64, usize),
    check: Arc<dyn Check>,
    svc: &ServiceCfg,
    ctl: Ctl,
    tx: mpsc::UnboundedSender<Tagged>,
) -> JoinHandle<()> {
    // Ya validados en Config::validate.
    let every = parse_duration(&svc.interval).unwrap_or(Duration::from_secs(30));
    let timeout = parse_duration(&svc.timeout).unwrap_or(Duration::from_secs(10));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if ctl.paused.load(Ordering::Relaxed) {
                        continue;
                    }
                }
                _ = ctl.now.notified() => tick.reset(),
            }
            let out = match tokio::time::timeout(timeout, check.run()).await {
                Ok(o) => o,
                Err(_) => Outcome::down(format!("timeout ({}s)", timeout.as_secs())),
            };
            let sample = Sample {
                at: unix_now(),
                ok: out.ok,
                value: out.value,
                detail: out.detail,
            };
            if tx.send((generation, idx, sample)).is_err() {
                break;
            }
        }
    })
}

/// Ejecuta cada check una vez, en paralelo, sin TUI. Devuelve los resultados
/// en el orden de la config.
pub async fn run_once(cfg: &Config) -> Result<Vec<(&ServiceCfg, Outcome)>> {
    let mut tasks = Vec::with_capacity(cfg.services.len());
    for svc in &cfg.services {
        let check = build(svc, cfg)?;
        let timeout = parse_duration(&svc.timeout).unwrap_or(Duration::from_secs(10));
        tasks.push(tokio::spawn(async move {
            match tokio::time::timeout(timeout, check.run()).await {
                Ok(o) => o,
                Err(_) => Outcome::down(format!("timeout ({}s)", timeout.as_secs())),
            }
        }));
    }
    let mut out = Vec::with_capacity(tasks.len());
    for (svc, task) in cfg.services.iter().zip(tasks) {
        out.push((svc, task.await?));
    }
    Ok(out)
}

/// Ejecuta un comando local o por ssh y devuelve (éxito, stdout+stderr recortado).
pub async fn run_shell(command: &str, ssh: Option<&str>) -> std::io::Result<(bool, String)> {
    let mut c = match ssh {
        Some(dest) => {
            let mut c = tokio::process::Command::new("ssh");
            // ControlMaster reutiliza una sola conexión por host entre checks.
            c.args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "ControlMaster=auto",
                "-o",
                "ControlPath=~/.ssh/tuisentry-%r@%h:%p",
                "-o",
                "ControlPersist=120",
                dest,
                command,
            ]);
            c
        }
        None => {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", command]);
            c
        }
    };
    let out = c
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    let mut text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    }
    Ok((out.status.success(), text))
}

pub fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
