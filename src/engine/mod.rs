//! Motor de monitoreo: ejecuta los checks, guarda el historial y emite eventos.
//! La UI solo habla con él por [`Command`] y [`Event`], así funciona igual en el
//! mismo proceso que en un daemon.

pub mod health;
pub mod persist;
pub mod remote;

use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc, task::JoinHandle};

pub use self::persist::Incident;
use self::{
    health::{Tracker, Transition},
    persist::Persist,
};
use crate::{
    alerts::{
        Alert, AlertKind, Evaluator,
        notify::{self, Notifier},
    },
    checks::{self, Check, Ctl, Tagged},
    config::{AlertsCfg, Config, parse_duration},
    prefs::{self, EnginePrefs},
    store::{History, MEMORY_WINDOW, Sample, unix_now},
    time,
};

/// Incidentes cerrados que se cargan del disco y se envían a la UI.
const RECENT_INCIDENTS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// `None` = todos.
    CheckNow(Option<usize>),
    /// `None` = todos.
    SetPaused(Option<usize>, bool),
    SetMuted(usize, bool),
    /// Vuelve a leer la config del disco.
    Reload,
    /// Calcula el uptime de 24h/7d/30d desde el disco.
    Stats(usize),
    /// Exporta a CSV las muestras de los últimos `secs` segundos.
    Export(usize, f64),
    /// Manda una alerta de prueba por todos los canales.
    TestAlert,
    /// Escribe lo pendiente y termina. No viaja por el socket: un cliente
    /// no puede apagar el daemon.
    #[serde(skip)]
    Shutdown,
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
    /// Abiertos primero, luego los cerrados más recientes.
    Incidents(Vec<Incident>),
    /// Uptime de 24h, 7d y 30d.
    Stats {
        idx: usize,
        uptime: [Option<f64>; 3],
    },
    /// Canales de alerta configurados.
    Channels(Vec<String>),
    Toast(String),
}

struct Runtime {
    info: ServiceInfo,
    history: History,
    tracker: Tracker,
    eval: Evaluator,
    slow_ms: Option<u64>,
    ctl: Ctl,
    task: JoinHandle<()>,
}

pub struct Engine {
    path: PathBuf,
    dir: PathBuf,
    generation: u64,
    services: Vec<Runtime>,
    tx: mpsc::UnboundedSender<Tagged>,
    rx: mpsc::UnboundedReceiver<Tagged>,
    /// Resultados de tareas en segundo plano (stats, export).
    aux_tx: mpsc::UnboundedSender<Event>,
    aux_rx: mpsc::UnboundedReceiver<Event>,
    subs: Vec<mpsc::UnboundedSender<Event>>,
    prefs: EnginePrefs,
    persist: Persist,
    open: Vec<Incident>,
    recent: Vec<Incident>,
    alerts: AlertsCfg,
    notifiers: Arc<Vec<Box<dyn Notifier>>>,
    /// Candado que hace de este motor el único que persiste y alerta.
    /// Sin él (otra instancia lo tiene), el motor solo muestra datos.
    lock: Option<File>,
}

/// Intenta tomar el candado del motor primario; `None` si otro lo tiene.
fn take_lock() -> Option<File> {
    let dir = prefs::state_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let f = File::create(dir.join("engine.lock")).ok()?;
    f.try_lock().ok()?;
    Some(f)
}

impl Engine {
    pub fn new(path: PathBuf) -> Result<Self> {
        let cfg = Config::load(&path)?;
        let dir = prefs::state_dir().join("history");
        let (tx, rx) = mpsc::unbounded_channel();
        let (aux_tx, aux_rx) = mpsc::unbounded_channel();
        let mut engine = Self {
            path,
            persist: Persist::new(dir.clone()),
            recent: persist::incidents(&dir, RECENT_INCIDENTS),
            dir,
            generation: 0,
            services: vec![],
            tx,
            rx,
            aux_tx,
            aux_rx,
            subs: vec![],
            prefs: EnginePrefs::load(),
            open: vec![],
            alerts: AlertsCfg::default(),
            notifiers: Arc::new(vec![]),
            lock: take_lock(),
        };
        if engine.is_primary() {
            let retention = parse_duration(&cfg.history.retention)?.as_secs_f64();
            persist::prune(&engine.dir, unix_now() - retention);
        }
        engine.start(&cfg)?;
        Ok(engine)
    }

    /// El primario guarda el historial y manda alertas; un secundario solo muestra.
    pub fn is_primary(&self) -> bool {
        self.lock.is_some()
    }

    /// (Re)lanza los checks de `cfg` conservando el historial por nombre.
    /// Si algún check no se puede construir, deja todo como estaba.
    fn start(&mut self, cfg: &Config) -> Result<()> {
        let built = cfg
            .services
            .iter()
            .map(|s| checks::build(s, cfg))
            .collect::<Result<Vec<_>>>()?;
        let notifiers = cfg
            .alerts
            .channels
            .iter()
            .map(notify::build)
            .collect::<Result<Vec<_>>>()?;
        self.notifiers = Arc::new(notifiers);
        self.alerts = cfg.alerts.clone();
        let mut old: HashMap<String, (History, Tracker, Evaluator)> = self
            .services
            .drain(..)
            .map(|rt| {
                rt.task.abort();
                (rt.info.name, (rt.history, rt.tracker, rt.eval))
            })
            .collect();
        self.persist.close_all();
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
                muted: sc.mute || self.prefs.muted.contains(&sc.name),
                history: vec![],
            };
            let (history, tracker, eval) = old.remove(&sc.name).unwrap_or_else(|| {
                let mut h = History::default();
                for s in persist::load(&self.dir, &sc.name, unix_now() - MEMORY_WINDOW) {
                    h.push(s);
                }
                (h, Tracker::default(), Evaluator::default())
            });
            let task = checks::spawn(
                (self.generation, i),
                check,
                sc,
                ctl.clone(),
                self.tx.clone(),
            );
            self.services.push(Runtime {
                info,
                history,
                tracker,
                eval,
                slow_ms: sc.slow_ms.or(cfg.alerts.slow_ms),
                ctl,
                task,
            });
        }
        // Los incidentes de servicios que ya no existen se descartan.
        self.open
            .retain(|inc| self.services.iter().any(|rt| rt.info.name == inc.service));
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

    fn channels(&self) -> Vec<String> {
        self.notifiers.iter().map(|n| n.label()).collect()
    }

    fn incidents(&self) -> Vec<Incident> {
        self.open.iter().chain(&self.recent).cloned().collect()
    }

    /// Nuevo receptor de eventos; empieza con el estado completo.
    pub fn subscribe(&mut self) -> mpsc::UnboundedReceiver<Event> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(Event::Snapshot(self.snapshot()));
        let _ = tx.send(Event::Incidents(self.incidents()));
        let _ = tx.send(Event::Channels(self.channels()));
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
                    self.broadcast(Event::Incidents(self.incidents()));
                    self.broadcast(Event::Channels(self.channels()));
                    self.broadcast(Event::Toast("config recargada".into()));
                }
                Err(e) => self.broadcast(Event::Toast(format!("config inválida: {e:#}"))),
            },
            Command::Shutdown => {}
            Command::Stats(idx) => self.spawn_stats(idx),
            Command::Export(idx, secs) => self.spawn_export(idx, secs),
            Command::TestAlert => {
                if self.notifiers.is_empty() {
                    self.broadcast(Event::Toast(
                        "no hay canales: añade [[alerts.notify]] a la config".into(),
                    ));
                } else {
                    self.dispatch(Alert {
                        service: "tuisentry".into(),
                        link: None,
                        kind: AlertKind::Test,
                    });
                }
            }
        }
    }

    /// Avisa en la UI y, si el servicio no está silenciado, por los canales.
    fn dispatch(&mut self, alert: Alert) {
        let muted = alert.kind != AlertKind::Test
            && self
                .services
                .iter()
                .any(|rt| rt.info.name == alert.service && rt.info.muted);
        let suffix = if muted { " (silenciado)" } else { "" };
        self.broadcast(Event::Toast(format!("{}{suffix}", alert.title())));
        if muted || self.notifiers.is_empty() || !self.is_primary() {
            return;
        }
        let (notifiers, tx) = (self.notifiers.clone(), self.aux_tx.clone());
        tokio::spawn(async move {
            for n in notifiers.iter() {
                if let Err(e) = n.send(&alert).await {
                    let _ = tx.send(Event::Toast(format!("{}: {e:#}", n.label())));
                }
            }
        });
    }

    fn spawn_stats(&mut self, idx: usize) {
        let Some(rt) = self.services.get(idx) else {
            return;
        };
        self.persist.flush();
        let (dir, name, tx) = (self.dir.clone(), rt.info.name.clone(), self.aux_tx.clone());
        tokio::task::spawn_blocking(move || {
            let now = unix_now();
            let samples = persist::load(&dir, &name, now - 30.0 * 86_400.0);
            let uptime = [1.0, 7.0, 30.0].map(|d| persist::uptime(&samples, now - d * 86_400.0));
            let _ = tx.send(Event::Stats { idx, uptime });
        });
    }

    fn spawn_export(&mut self, idx: usize, secs: f64) {
        let Some(rt) = self.services.get(idx) else {
            return;
        };
        self.persist.flush();
        let (dir, name, tx) = (self.dir.clone(), rt.info.name.clone(), self.aux_tx.clone());
        tokio::task::spawn_blocking(move || {
            let now = unix_now();
            let samples = persist::load(&dir, &name, now - secs);
            let file = dirs::home_dir().unwrap_or_default().join(format!(
                "tuisentry-{}-{}.csv",
                persist::slug(&name),
                time::date(now)
            ));
            let msg = match persist::write_csv(&file, &samples) {
                Ok(()) => format!("⇩ {} muestras en {}", samples.len(), file.display()),
                Err(e) => format!("no pude exportar: {e}"),
            };
            let _ = tx.send(Event::Toast(msg));
        });
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
        let Some(rt) = self.services.get_mut(idx) else {
            return;
        };
        rt.history.push(sample.clone());
        if self.lock.is_some() {
            let _ = self.persist.append(&rt.info.name, &sample);
        }
        let (name, link) = (rt.info.name.clone(), rt.info.link.clone());
        let transition = rt.tracker.observe(&sample, self.alerts.fail_after);
        let extra = rt.eval.observe(&sample, rt.slow_ms, self.alerts.cert_days);
        self.broadcast(Event::Sample(idx, sample));
        if let Some(t) = transition {
            let kind = match &t {
                Transition::Down { cause, .. } => AlertKind::Down {
                    cause: cause.clone(),
                },
                Transition::Up { duration, .. } => AlertKind::Recovered {
                    duration: *duration,
                },
            };
            self.on_transition(&name, t);
            self.dispatch(Alert {
                service: name.clone(),
                link: link.clone(),
                kind,
            });
        }
        for kind in extra {
            self.dispatch(Alert {
                service: name.clone(),
                link: link.clone(),
                kind,
            });
        }
    }

    fn on_transition(&mut self, service: &str, t: Transition) {
        match t {
            Transition::Down { since, cause } => self.open.push(Incident {
                service: service.into(),
                start: since,
                end: None,
                cause,
            }),
            Transition::Up { since, duration } => {
                if let Some(pos) = self
                    .open
                    .iter()
                    .position(|i| i.service == service && i.start == since)
                {
                    let mut inc = self.open.remove(pos);
                    inc.end = Some(since + duration);
                    if self.lock.is_some() {
                        let _ = self.persist.incident(&inc);
                    }
                    self.recent.insert(0, inc);
                    self.recent.truncate(RECENT_INCIDENTS);
                }
            }
        }
        self.broadcast(Event::Incidents(self.incidents()));
    }

    /// Bucle principal. Termina cuando se cierran todos los emisores de comandos.
    pub async fn run(
        mut self,
        mut cmds: mpsc::UnboundedReceiver<Command>,
        mut subs: mpsc::UnboundedReceiver<mpsc::UnboundedSender<Event>>,
    ) {
        let mut flush = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                Some(t) = self.rx.recv() => self.on_sample(t),
                Some(ev) = self.aux_rx.recv() => self.broadcast(ev),
                cmd = cmds.recv() => match cmd {
                    Some(Command::Shutdown) | None => break,
                    Some(c) => self.handle(c),
                },
                Some(sub) = subs.recv() => {
                    let ok = sub.send(Event::Snapshot(self.snapshot())).is_ok()
                        && sub.send(Event::Incidents(self.incidents())).is_ok()
                        && sub.send(Event::Channels(self.channels())).is_ok();
                    if ok {
                        self.subs.push(sub);
                    }
                }
                _ = flush.tick() => self.persist.flush(),
            }
        }
        self.persist.flush();
        for rt in &self.services {
            rt.task.abort();
        }
    }
}

/// De dónde vienen los datos que ve la UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Motor propio que guarda historial y alerta.
    Local,
    /// Motor propio sin persistir ni alertar: otra instancia lo hace.
    Viewer,
    /// Conectado al daemon.
    Remote,
}

/// Canales para hablar con el motor.
pub struct Backend {
    pub cmd: mpsc::UnboundedSender<Command>,
    pub events: mpsc::UnboundedReceiver<Event>,
    pub mode: Mode,
    /// Tarea del motor local, para esperar a que suelte el candado.
    pub task: Option<JoinHandle<()>>,
}

impl Backend {
    /// Detiene el motor local (si lo hay) y espera a que escriba lo pendiente
    /// y suelte el candado. Con el daemon solo se cierra la conexión.
    pub async fn shutdown(self) {
        let Backend { cmd, task, .. } = self;
        if task.is_some() {
            let _ = cmd.send(Command::Shutdown);
        }
        drop(cmd);
        if let Some(task) = task {
            let _ = tokio::time::timeout(Duration::from_secs(3), task).await;
        }
    }
}

/// Motor dentro del mismo proceso que la UI.
pub fn local(path: PathBuf) -> Result<Backend> {
    let mut engine = Engine::new(path)?;
    let mode = if engine.is_primary() {
        Mode::Local
    } else {
        Mode::Viewer
    };
    let events = engine.subscribe();
    let (cmd, cmds) = mpsc::unbounded_channel();
    let (_no_subs, subs) = mpsc::unbounded_channel();
    let task = tokio::spawn(engine.run(cmds, subs));
    Ok(Backend {
        cmd,
        events,
        mode,
        task: Some(task),
    })
}
