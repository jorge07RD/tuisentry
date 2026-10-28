mod app;
mod checks;
mod config;
mod input;
mod links;
mod panels;
mod store;
mod theme;
mod ui;

use std::{io::stdout, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use ratatui::crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
};
use tokio::sync::mpsc;

use crate::{app::App, config::Config, input::HitMap};

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

    let start = tokio::time::Instant::now();
    let (tx, mut samples) = mpsc::unbounded_channel();
    let mut app = App::new(&cfg, args.web, start, tx)?;

    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        hook(info);
    }));

    // crossterm bloquea al leer; un hilo aparte reenvía los eventos.
    let (ev_tx, mut events) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if ev_tx.send(ev).is_err() {
                break;
            }
        }
    });

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
            Some((i, s)) = samples.recv() => app.record(i, s),
            _ = tick.tick() => {}
        }
        // Agrupa lo que haya llegado mientras tanto en un solo redibujo.
        while let Ok(ev) = events.try_recv() {
            app.on_event(ev);
        }
        while let Ok((i, s)) = samples.try_recv() {
            app.record(i, s);
        }
        if app.quit {
            break Ok(());
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

    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}
