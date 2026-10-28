mod alerts;
mod app;
mod checks;
mod config;
mod config_edit;
mod engine;
mod form;
mod input;
mod links;
mod menu;
mod panels;
mod prefs;
mod store;
mod text;
mod theme;
mod time;
mod ui;
mod view;

use std::{
    io::stdout,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{
            self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
            EnableMouseCapture,
        },
        execute,
    },
};
use tokio::sync::mpsc;

use crate::{
    app::{App, Effect},
    config::Config,
    input::HitMap,
};

const EXAMPLE: &str = include_str!("../config.example.toml");
const USAGE: &str = "\
tuisentry — monitor de servicios para la terminal

USO:
    tuisentry [--config RUTA] [--web]
    tuisentry --init
    tuisentry --once [--config RUTA]

OPCIONES:
    -c, --config RUTA   config a usar (por defecto ~/.config/tuisentry/config.toml)
        --web           modo celular/navegador (para servirla con ttyd)
        --init          crea la config de ejemplo en la ruta por defecto
        --once          valida la config, ejecuta cada check una vez y sale
                        (código 1 si alguno está caído)
    -h, --help          muestra esta ayuda";

struct Args {
    config: PathBuf,
    web: bool,
    init: bool,
    once: bool,
}

fn parse_args() -> Result<Args> {
    let default = dirs::config_dir()
        .context("no encuentro el directorio de config")?
        .join("tuisentry/config.toml");
    let mut args = Args {
        config: default,
        web: false,
        init: false,
        once: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-c" | "--config" => {
                args.config = it.next().context("--config necesita una ruta")?.into()
            }
            "--web" => args.web = true,
            "--init" => args.init = true,
            "--once" => args.once = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("argumento desconocido: {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

fn init_config(path: &PathBuf) -> Result<()> {
    if path.exists() {
        bail!("{} ya existe; no lo sobrescribo", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, EXAMPLE)?;
    println!(
        "config creada en {} — edítala y ejecuta tuisentry",
        path.display()
    );
    Ok(())
}

async fn once(cfg: &Config) -> Result<()> {
    let results = checks::run_once(cfg).await?;
    let w = results
        .iter()
        .map(|(s, _)| s.name.chars().count())
        .max()
        .unwrap_or(0);
    let mut down = 0;
    for (svc, o) in &results {
        let mark = if o.ok { "✓" } else { "✗" };
        down += usize::from(!o.ok);
        println!("{mark} {:<w$}  {:<8} {}", svc.name, svc.kind, o.detail);
    }
    println!("\n{} servicios · {} caídos", results.len(), down);
    if down > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    if args.init {
        return init_config(&args.config);
    }
    if !args.config.exists() {
        bail!(
            "no existe {}\ncréala con `tuisentry --init` o pasa otra con --config",
            args.config.display()
        );
    }
    let cfg = Config::load(&args.config)?;
    if args.once {
        return once(&cfg).await;
    }

    let mut backend = engine::local(args.config.clone())?;
    let mut app = App::new(cfg, args.config.clone(), args.web, backend.cmd.clone());

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
        hook(info);
    }));

    // crossterm bloquea al leer; un hilo aparte reenvía los eventos. `reading`
    // lo pausa mientras otro programa (el $EDITOR) usa la terminal.
    let reading = Arc::new(AtomicBool::new(true));
    let (ev_tx, mut events) = mpsc::unbounded_channel();
    {
        let reading = reading.clone();
        std::thread::spawn(move || {
            loop {
                if !reading.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                match event::poll(Duration::from_millis(100)) {
                    Ok(true) => {
                        let Ok(ev) = event::read() else { break };
                        if ev_tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        });
    }

    // Refresco para que las gráficas avancen y caduquen los toasts.
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut mouse_on = true;
    let result: Result<()> = loop {
        let mut hits = HitMap::default();
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app, &mut hits)) {
            break Err(e.into());
        }
        let _ = links::emit_hyperlinks(&hits.links);
        app.hits = hits;

        tokio::select! {
            Some(ev) = events.recv() => app.on_event(ev),
            Some(ev) = backend.events.recv() => app.on_engine(ev),
            _ = tick.tick() => {}
        }
        // Agrupa lo que haya llegado mientras tanto en un solo redibujo.
        while let Ok(ev) = events.try_recv() {
            app.on_event(ev);
        }
        while let Ok(ev) = backend.events.try_recv() {
            app.on_engine(ev);
        }
        if app.quit {
            break Ok(());
        }
        for effect in std::mem::take(&mut app.effects) {
            match effect {
                Effect::EditConfig => {
                    if let Err(e) = edit_config(&app.cfg_path, &mut terminal, &reading) {
                        app.toast(format!("no pude abrir el editor: {e}"));
                    }
                    mouse_on = true;
                    app.reload();
                }
            }
        }

        let want = app.mouse_wanted();
        if want != mouse_on {
            mouse_on = want;
            if want {
                execute!(stdout(), EnableMouseCapture)?;
            } else {
                execute!(stdout(), DisableMouseCapture)?;
            }
        }
    };

    execute!(stdout(), DisableMouseCapture, DisableBracketedPaste)?;
    ratatui::restore();
    // Cerrar los canales deja que el motor escriba lo pendiente al disco.
    drop(app);
    drop(backend);
    tokio::time::sleep(Duration::from_millis(100)).await;
    result
}

/// Suspende la TUI, abre la config en `$VISUAL`/`$EDITOR` (o nano) y la restaura.
fn edit_config(path: &Path, terminal: &mut DefaultTerminal, reading: &AtomicBool) -> Result<()> {
    reading.store(false, Ordering::Relaxed);
    // Da tiempo al hilo lector a salir de su `poll`.
    std::thread::sleep(Duration::from_millis(150));
    execute!(stdout(), DisableMouseCapture, DisableBracketedPaste)?;
    ratatui::restore();

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    // Vía sh para admitir editores con argumentos, p. ej. "code --wait".
    let status = std::process::Command::new("sh")
        .args(["-c", &format!("{editor} \"$1\""), "sh"])
        .arg(path)
        .status();

    *terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    reading.store(true, Ordering::Relaxed);
    status?;
    Ok(())
}
