use std::time::Instant;

use super::{BoxFut, Check, Outcome, ms};

pub struct Tcp {
    pub addr: String,
}

impl Check for Tcp {
    fn run(&self) -> BoxFut<'_, Outcome> {
        Box::pin(async move {
            let t = Instant::now();
            match tokio::net::TcpStream::connect(&self.addr).await {
                Ok(_) => {
                    let lat = ms(t.elapsed());
                    Outcome::up(Some(lat), format!("abierto · {lat:.0} ms"))
                }
                Err(e) => Outcome::down(e.to_string()),
            }
        })
    }
}
