//! Motor de monitoreo: ejecuta los checks, guarda el historial y emite eventos.
//! La UI solo habla con él por [`Command`] y [`Event`], así funciona igual en el
//! mismo proceso que en un daemon.

use std::{collections::HashMap, path::PathBuf, sync::Arc, sync::atomic::Ordering};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    checks::{self, Check, Ctl, Tagged},
    config::Config,
    prefs::EnginePrefs,
    store::{History, Sample},
};

/// Muestras guardadas en memoria por servicio.
const HISTORY: usize = 600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// `None` = todos.
    CheckNow(Option<usize>),
    /// `None` = todos.
    SetPaused(Option<usize>, bool),
    SetMuted(usize, bool),
    /// Vuelve a leer la config del disco.
    Reload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub kind: String,
    pub target: Option<String>,
    pub host: Option<String>,
    pub link: Option<String>,
    pub suffix: String,
    pub paused: bool,
    pub muted: bool,
    pub history: Vec<Sample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// Estado completo; llega al suscribirse y tras cada recarga.
    Snapshot(Vec<ServiceInfo>),
    Sample(usize, Sample),
    Flags {
        idx: usize,
        paused: bool,
        muted: bool,
    },
    Toast(String),
}

struct Runtime {
    info: ServiceInfo,
    history: History,
    ctl: Ctl,
    task: JoinHandle<()>,
}

pub struct Engine {
    path: PathBuf,
    generation: u64,
    services: Vec<Runtime>,
    tx: mpsc::UnboundedSender<Tagged>,
    rx: mpsc::UnboundedReceiver<Tagged>,
    subs: Vec<mpsc::UnboundedSender<Event>>,
    prefs: EnginePrefs,
}

impl Engine {
    pub fn new(path: PathBuf) -> Result<Self> {
        let cfg = Config::load(&path)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let mut engine = Self {
            path,
            generation: 0,
            services: vec![],
            tx,
            rx,
            subs: vec![],
            prefs: EnginePrefs::load(),
        };
        engine.start(&cfg)?;
        Ok(engine)
    }

    /// (Re)lanza los checks de `cfg` conservando el historial por nombre.
    /// Si algún check no se puede construir, deja todo como estaba.
    fn start(&mut self, cfg: &Config) -> Result<()> {
        let built = cfg
            .services
            .iter()
            .map(|s| checks::build(s, cfg))
            .collect::<Result<Vec<_>>>()?;
        let mut old: HashMap<String, History> = self
            .services
            .drain(..)
            .map(|rt| {
                rt.task.abort();
                (rt.info.name, rt.history)
            })
            .collect();
        self.generation += 1;

        for (i, (sc, check)) in cfg.services.iter().zip(built).enumerate() {
            let check: Arc<dyn Check> = check.into();
            let ctl = Ctl::default();
            let paused = self.prefs.paused.contains(&sc.name);
            ctl.paused.store(paused, Ordering::Relaxed);
            let info = ServiceInfo {
                name: sc.name.clone(),
                kind: sc.kind.clone(),
                target: sc
                    .target
                    .clone()
                    .or_else(|| sc.unit.clone())
                    .or_else(|| sc.command.clone()),
                host: sc.host.clone(),
                link: sc.link.clone().or_else(|| check.default_link()),
                suffix: sc
                    .suffix
                    .clone()
                    .unwrap_or_else(|| check.default_suffix().into()),
                paused,
                muted: self.prefs.muted.contains(&sc.name),
                history: vec![],
            };
            let task = checks::spawn(
                (self.generation, i),
                check,
                sc,
                ctl.clone(),
                self.tx.clone(),
            );
            self.services.push(Runtime {
                history: old
                    .remove(&sc.name)
                    .unwrap_or_else(|| History::new(HISTORY)),
                info,
                ctl,
                task,
            });
        }
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<ServiceInfo> {
        self.services
            .iter()
            .map(|rt| ServiceInfo {
                history: rt.history.iter().cloned().collect(),
                ..rt.info.clone()
            })
            .collect()
    }

    /// Nuevo receptor de eventos; empieza con un `Snapshot`.
    pub fn subscribe(&mut self) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(Event::Snapshot(self.snapshot()));
        self.subs.push(tx);
        rx
    }

    fn broadcast(&mut self, ev: Event) {
        self.subs.retain(|s| s.send(ev.clone()).is_ok());
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::CheckNow(target) => {
                for (i, rt) in self.services.iter().enumerate() {
                    if target.is_none_or(|t| t == i) {
                        rt.ctl.now.notify_one();
                    }
                }
            }
            Command::SetPaused(target, on) => {
                let mut changed = vec![];
                for (i, rt) in self.services.iter_mut().enumerate() {
                    if target.is_none_or(|t| t == i) {
                        rt.ctl.paused.store(on, Ordering::Relaxed);
                        rt.info.paused = on;
                        changed.push(i);
                    }
                }
                self.flags_changed(changed);
            }
            Command::SetMuted(i, on) => {
                if let Some(rt) = self.services.get_mut(i) {
                    rt.info.muted = on;
                    self.flags_changed(vec![i]);
                }
            }
            Command::Reload => match Config::load(&self.path).and_then(|cfg| self.start(&cfg)) {
                Ok(()) => {
                    self.save_prefs();
                    self.broadcast(Event::Snapshot(self.snapshot()));
                    self.broadcast(Event::Toast("config recargada".into()));
                }
                Err(e) => self.broadcast(Event::Toast(format!("config inválida: {e:#}"))),
            },
        }
    }

    fn flags_changed(&mut self, idxs: Vec<usize>) {
        self.save_prefs();
        for idx in idxs {
            let info = &self.services[idx].info;
            let ev = Event::Flags {
                idx,
                paused: info.paused,
                muted: info.muted,
            };
            self.broadcast(ev);
        }
    }

    fn save_prefs(&mut self) {
        let names = |f: fn(&ServiceInfo) -> bool| {
            self.services
                .iter()
                .filter(|rt| f(&rt.info))
                .map(|rt| rt.info.name.clone())
                .collect()
        };
        self.prefs.paused = names(|i| i.paused);
        self.prefs.muted = names(|i| i.muted);
        self.prefs.save();
    }

    fn on_sample(&mut self, (generation, idx, sample): Tagged) {
        if generation != self.generation {
            return;
        }
        if let Some(rt) = self.services.get_mut(idx) {
            rt.history.push(sample.clone());
            self.broadcast(Event::Sample(idx, sample));
        }
    }

    /// Bucle principal. Termina cuando se cierran todos los emisores de comandos.
    pub async fn run(
        mut self,
        mut cmds: mpsc::UnboundedReceiver<Command>,
        mut subs: mpsc::UnboundedReceiver<mpsc::UnboundedSender<Event>>,
    ) {
        loop {
            tokio::select! {
                Some(t) = self.rx.recv() => self.on_sample(t),
                cmd = cmds.recv() => match cmd {
                    Some(c) => self.handle(c),
                    None => break,
                },
                Some(sub) = subs.recv() => {
                    if sub.send(Event::Snapshot(self.snapshot())).is_ok() {
                        self.subs.push(sub);
                    }
                }
            }
        }
        for rt in &self.services {
            rt.task.abort();
        }
    }
}

/// Canales para hablar con el motor.
pub struct Backend {
    pub cmd: mpsc::UnboundedSender<Command>,
    pub events: mpsc::UnboundedReceiver<Event>,
}

/// Motor dentro del mismo proceso que la UI.
pub fn local(path: PathBuf) -> Result<Backend> {
    let mut engine = Engine::new(path)?;
    let events = engine.subscribe();
    let (cmd, cmds) = mpsc::unbounded_channel();
    let (_no_subs, subs) = mpsc::unbounded_channel();
    tokio::spawn(engine.run(cmds, subs));
    Ok(Backend { cmd, events })
}
