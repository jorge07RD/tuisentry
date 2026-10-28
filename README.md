# tuisentry

TUI ligera para vigilar tus servicios: HTTP/HTTPS, puertos TCP, ping, unidades
systemd (locales o por ssh) y cualquier métrica que saque un comando. Varias
páginas con paneles de gráficas, link directo a cada recurso, y se maneja
entera con el mouse o con el dedo desde el celular.

```
 ◆   Overview   Red                     ● 4 ● 2
╭ Servicios ──────────────────────────────── ⤢ ╮
│ ● example.com              205 ms  100.0%  ↗ │
│                                              │
│ ● Cloudflare DNS            38 ms  100.0%    │
│                                              │
│ ● SSH local                 caído    0.0%    │
│                                              │
│ ● API caída                 caído    0.0%  ↗ │
│                                              │
│ ● NetworkManager           active  100.0%    │
│                                              │
│ ● Disco /                     82%  100.0%    │
╰──────────────────────────────────────────────╯
 ◄         Servicios 1/4          ►   ⟳   ?   ✕
```

- **Ligera:** binario de ~4 MB, ~4 MB de RAM y 0 % de CPU entre checks.
- **Táctil:** todo funciona con clic o toque; el teclado es opcional.
- **Responsiva:** rejilla de paneles en escritorio, un panel a la vez en pantallas chicas.
- **Extensible:** checks y paneles nuevos sin tocar el núcleo, o métricas propias con `cmd`.

## Instalar

Requiere Rust 1.88 o más reciente.

```sh
cargo install --git https://github.com/jorge07RD/tuisentry
tuisentry --init          # crea ~/.config/tuisentry/config.toml de ejemplo
tuisentry                 # o: tuisentry --config otra.toml
tuisentry --once          # ejecuta cada check una vez y sale (útil en scripts/CI)
```

## Uso

| Toque / clic       | Hace                                      |
|--------------------|-------------------------------------------|
| un servicio        | abre el detalle: gráfica, stats y link    |
| `↗`                | abre el recurso monitoreado               |
| título de un panel | zoom a pantalla completa (otra vez: sale) |
| pestañas arriba    | cambia de página                          |
| `◄` `►` abajo      | panel anterior/siguiente (pantalla chica) |
| arrastrar / rueda  | desplaza la lista                         |
| `⟳` `?` `✕`        | re-check de todo · ayuda · salir          |

Teclado opcional: `Tab`/`1-9` páginas, `h`/`l` paneles, `j`/`k` scroll,
`Enter` zoom, `r` re-check, `o` abrir link, `m` mouse libre, `Esc`, `q`.

Con menos de 100 columnas se muestra un panel a la vez, pensado para el celular.

## Desde el celular (navegador)

La TUI corre en tu PC y la sirves con [ttyd](https://github.com/tsl0922/ttyd);
en el cel solo abres una URL. Los toques llegan como clics.

```sh
ttyd -W -p 7681 -t fontSize=15 -t disableLeaveAlert=true tuisentry --web
```

Ábrela desde el cel en `http://<ip-del-pc>:7681`. **No la expongas a internet
sin protección**: úsala dentro de Tailscale/WireGuard o añade `-c usuario:clave`
(y TLS con `--ssl`).

`--web` cambia tres cosas:

- Filas, pestañas y botones más altos para el dedo.
- `↗` no ejecuta `xdg-open` (abriría el navegador del PC): copia la URL
  (OSC 52) y la muestra.
- En el detalle aparece **⇱ Mouse libre**: suelta el mouse 10 s para que
  puedas tocar el link (es un hipervínculo OSC 8) o seleccionar texto.
  Cualquier tecla lo devuelve antes.

## Config

Mira [`config.example.toml`](config.example.toml). Lo esencial:

```toml
[[host]]
name = "vps"
ssh = "jorge@vps.example.com"   # login sin contraseña; se reutiliza la conexión

[[service]]
name = "Blog"
kind = "http"                   # http | tcp | ping | systemd | cmd
target = "https://blog.example.com"
interval = "30s"

[[service]]
name = "nginx"
kind = "systemd"
host = "vps"
unit = "nginx.service"
link = "https://vps.example.com:9090"   # p. ej. Cockpit

[[service]]
name = "RAM vps"
kind = "cmd"
host = "vps"
command = "free | awk '/Mem/ {printf \"%.0f\", $3/$2*100}'"
suffix = "%"

[[page]]
title = "VPS"
  [[page.panel]]
  kind = "status"               # status | chart | spark | gauge
  host = "vps"
  [[page.panel]]
  kind = "chart"
  services = ["Blog"]
  window = "1h"
```

`ping` usa ICMP sin root; si falla con permiso denegado, revisa
`sysctl net.ipv4.ping_group_range`.

## Extender

- **Nuevo tipo de check**: implementa `Check` (`src/checks/mod.rs`) en un módulo
  y añade su `kind` al `match` de `checks::build`.
- **Nuevo tipo de panel**: implementa `Panel` (`src/panels/mod.rs`) y añádelo a
  `panels::build`. Registra en el `HitMap` las áreas tocables con su `Action`.
- **Sin recompilar**: `kind = "cmd"` acepta cualquier comando cuyo stdout empiece
  por un número.

## Licencia

[MIT](LICENSE) © 2026 Jorge Isac Beriguete Mateo
