//! Modo daemon: el motor corre sin interfaz, guarda historial y manda alertas
//! aunque la TUI esté cerrada. Las TUI se conectan por el socket.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::{net::UnixListener, sync::mpsc};

use crate::engine::{
    Command, Engine, Event,
    remote::{read_lines, socket_path, write_lines},
};

pub async fn run(config: PathBuf) -> Result<()> {
    let engine = Engine::new(config)?;
    if !engine.is_primary() {
        bail!(
            "otra instancia de tuisentry ya está monitoreando; ciérrala antes de iniciar el daemon"
        );
    }
    let sock = socket_path();
    // Un socket huérfano de una ejecución anterior impediría escuchar.
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).with_context(|| {
        // Los sockets Unix admiten rutas de ~108 bytes como mucho.
        let hint = if sock.as_os_str().len() > 100 {
            " (ruta demasiado larga: define XDG_RUNTIME_DIR con una más corta)"
        } else {
            ""
        };
        format!("no pude escuchar en {}{hint}", sock.display())
    })?;
    eprintln!("tuisentry daemon escuchando en {}", sock.display());

    let (cmd_tx, cmds) = mpsc::unbounded_channel::<Command>();
    let (sub_tx, subs) = mpsc::unbounded_channel::<mpsc::UnboundedSender<Event>>();
    let engine_task = tokio::spawn(engine.run(cmds, subs));

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            conn = listener.accept() => {
                let Ok((stream, _)) = conn else { continue };
                let (read, write) = stream.into_split();
                let (ev_tx, ev_rx) = mpsc::unbounded_channel();
                if sub_tx.send(ev_tx).is_err() {
                    break;
                }
                tokio::spawn(write_lines(write, ev_rx));
                tokio::spawn(read_lines(read, cmd_tx.clone()));
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = term.recv() => break,
        }
    }
    let _ = cmd_tx.send(Command::Shutdown);
    drop(listener);
    let _ = std::fs::remove_file(&sock);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), engine_task).await;
    Ok(())
}

pub fn unit_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("no encuentro ~/.config")?
        .join("systemd/user/tuisentry.service"))
}

pub fn unit_text(exe: &Path, config: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=tuisentry — monitor de servicios (daemon)\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" --daemon --config \"{}\"\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display(),
        config.display()
    )
}

fn systemctl(args: &[&str]) -> Result<()> {
    let out = std::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .context("no pude ejecutar systemctl")?;
    if !out.status.success() {
        bail!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

pub fn is_installed() -> bool {
    unit_path().is_ok_and(|p| p.exists())
}

/// Escribe la unidad de usuario de systemd (sin arrancarla).
pub fn write_unit(config: &Path) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("no encuentro el ejecutable")?;
    let config = config
        .canonicalize()
        .unwrap_or_else(|_| config.to_path_buf());
    let path = unit_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, unit_text(&exe, &config))?;
    systemctl(&["daemon-reload"])?;
    Ok(path)
}

pub fn enable() -> Result<()> {
    systemctl(&["enable", "--now", "tuisentry.service"])
}

pub fn uninstall() -> Result<()> {
    // Si ya estaba parado o no existe, seguimos con el borrado.
    let _ = systemctl(&["disable", "--now", "tuisentry.service"]);
    let path = unit_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    systemctl(&["daemon-reload"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_quotes_paths() {
        let u = unit_text(
            Path::new("/home/a b/.cargo/bin/tuisentry"),
            Path::new("/c/config.toml"),
        );
        assert!(u.contains(
            "ExecStart=\"/home/a b/.cargo/bin/tuisentry\" --daemon --config \"/c/config.toml\""
        ));
        assert!(u.contains("WantedBy=default.target"));
    }
}
