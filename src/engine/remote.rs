//! Conexión con el daemon por un socket Unix: un JSON por línea, `Event` del
//! daemon a la UI y `Command` de la UI al daemon.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};

use super::{Backend, Command, Event, Mode};
use crate::prefs;

/// `$XDG_RUNTIME_DIR/tuisentry.sock`, o junto al estado si no hay runtime dir.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(prefs::state_dir)
        .join("tuisentry.sock")
}

/// Envía cada mensaje del canal como una línea JSON.
pub async fn write_lines<T: Serialize>(
    mut out: impl AsyncWrite + Unpin,
    mut rx: mpsc::UnboundedReceiver<T>,
) {
    while let Some(msg) = rx.recv().await {
        let Ok(mut line) = serde_json::to_vec(&msg) else {
            continue;
        };
        line.push(b'\n');
        if out.write_all(&line).await.is_err() {
            break;
        }
    }
}

/// Lee líneas JSON y las pasa al canal; termina al cerrarse la conexión.
pub async fn read_lines<T: DeserializeOwned>(
    input: impl AsyncRead + Unpin,
    tx: mpsc::UnboundedSender<T>,
) {
    let mut lines = BufReader::new(input).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Ok(msg) = serde_json::from_str(&line)
            && tx.send(msg).is_err()
        {
            break;
        }
    }
}

/// Conecta con el daemon si está corriendo.
pub async fn connect() -> Result<Backend> {
    let stream = UnixStream::connect(socket_path()).await?;
    let (read, write) = stream.into_split();
    let (cmd, cmds) = mpsc::unbounded_channel::<Command>();
    let (ev_tx, events) = mpsc::unbounded_channel::<Event>();
    tokio::spawn(write_lines(write, cmds));
    tokio::spawn(async move {
        read_lines(read, ev_tx.clone()).await;
        let _ = ev_tx.send(Event::Toast(
            "se perdió la conexión con el daemon; reabre tuisentry".into(),
        ));
    });
    Ok(Backend {
        cmd,
        events,
        mode: Mode::Remote,
        task: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Sample;

    #[tokio::test]
    async fn protocol_roundtrip() {
        let (a, b) = tokio::io::duplex(4096);
        let (tx, rx) = mpsc::unbounded_channel();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Event>();
        tokio::spawn(write_lines(a, rx));
        tokio::spawn(read_lines(b, out_tx));
        tx.send(Event::Sample(
            3,
            Sample {
                at: 1.5,
                ok: true,
                value: Some(42.0),
                detail: "HTTP 200 · ñ".into(),
                cert_days: Some(30),
            },
        ))
        .unwrap();
        tx.send(Event::Toast("hola".into())).unwrap();
        match out_rx.recv().await.unwrap() {
            Event::Sample(3, s) => {
                assert_eq!(s.value, Some(42.0));
                assert_eq!(s.detail, "HTTP 200 · ñ");
                assert_eq!(s.cert_days, Some(30));
            }
            other => panic!("inesperado: {other:?}"),
        }
        assert!(matches!(out_rx.recv().await, Some(Event::Toast(m)) if m == "hola"));

        // Apagar el daemon no se puede pedir desde un cliente.
        assert!(serde_json::from_str::<Command>("\"Shutdown\"").is_err());

        // Los comandos también viajan.
        let line = serde_json::to_string(&Command::SetPaused(None, true)).unwrap();
        assert!(matches!(
            serde_json::from_str::<Command>(&line).unwrap(),
            Command::SetPaused(None, true)
        ));
    }
}
