//! Cross-screen em modo offline: quando o cursor é empurrado contra a borda
//! configurada, o mouse é trocado para o canal do outro computador.
//!
//! No Wayland nenhum cliente consegue ler a posição global do cursor, então a
//! posição horizontal é estimada somando o movimento bruto do sensor
//! (interface hidraw de movimento, lida sem capturar o mouse) e limitando a
//! estimativa a [0, largura]. Encostar o cursor numa borda ressincroniza a
//! estimativa; a troca exige continuar empurrando `push_counts` além da borda.

use crate::{config, hid};
use std::io::Read;
use std::time::{Duration, Instant};

pub struct EdgeTracker {
    x: f64,
    push: f64,
    last_push: Instant,
    cooldown_until: Instant,
}

pub enum Edge {
    Left,
    Right,
}

impl EdgeTracker {
    pub fn new(width: f64) -> Self {
        let now = Instant::now();
        EdgeTracker { x: width / 2.0, push: 0.0, last_push: now, cooldown_until: now }
    }

    /// Processa um deslocamento; devolve a borda quando a troca deve ocorrer.
    pub fn feed(&mut self, dx: f64, cfg: &config::Cross) -> Option<Edge> {
        let w = cfg.width_counts.max(100.0);
        let now = Instant::now();
        if now.duration_since(self.last_push) > Duration::from_millis(300) {
            self.push = 0.0;
        }
        let target = self.x + dx;
        let overflow = if target > w { target - w } else if target < 0.0 { target } else { 0.0 };
        self.x = target.clamp(0.0, w);
        if overflow == 0.0 || now < self.cooldown_until {
            if dx != 0.0 && overflow == 0.0 {
                self.push = 0.0;
            }
            return None;
        }
        if self.push.signum() != overflow.signum() {
            self.push = 0.0;
        }
        self.push += overflow;
        self.last_push = now;
        if self.push.abs() < cfg.push_counts {
            return None;
        }
        let edge = if self.push > 0.0 { Edge::Right } else { Edge::Left };
        self.push = 0.0;
        self.cooldown_until = now + Duration::from_secs(2);
        Some(edge)
    }
}

/// Lê um relatório de movimento: [botões, x i16, y i16, roda, roda lateral].
pub fn read_dx(dev: &mut std::fs::File) -> std::io::Result<i16> {
    let mut buf = [0u8; 16];
    let n = dev.read(&mut buf)?;
    if n < 5 {
        return Ok(0);
    }
    Ok(i16::from_le_bytes([buf[1], buf[2]]))
}

/// Laço do serviço (`mt760 --daemon`).
pub fn run_daemon() -> ! {
    let mut cfg = config::load();
    let mut stamp = config::mtime();
    let mut tracker = EdgeTracker::new(cfg.cross.width_counts);
    eprintln!("[mt760] serviço de cross-screen iniciado ({})", config::path().display());
    let _lan = crate::lan::Lan::start();
    loop {
        let mut dev = match hid::open_motion() {
            Ok(d) => d,
            Err(_) => {
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        eprintln!("[mt760] lendo movimento do receptor");
        loop {
            let dx = match read_dx(&mut dev) {
                Ok(dx) => dx,
                Err(e) => {
                    eprintln!("[mt760] receptor desconectado: {e}");
                    break;
                }
            };
            if config::mtime() != stamp {
                stamp = config::mtime();
                cfg = config::load();
                eprintln!("[mt760] configuração recarregada: {:?}", cfg.cross);
            }
            if !cfg.cross.enabled {
                continue;
            }
            let Some(edge) = tracker.feed(dx as f64, &cfg.cross) else { continue };
            let channel = match edge {
                Edge::Right => cfg.cross.right,
                Edge::Left => cfg.cross.left,
            };
            if let Some(ch) = channel {
                match hid::Receiver::open().and_then(|mut r| r.switch_channel(ch)) {
                    Ok(()) => eprintln!("[mt760] mouse enviado para {}", cfg.channel_label(ch)),
                    Err(e) => eprintln!("[mt760] falha ao enviar para {}: {e}", cfg.channel_label(ch)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> config::Cross {
        config::Cross { enabled: true, right: Some(1), left: None, width_counts: 1000.0, push_counts: 300.0 }
    }

    #[test]
    fn moving_inside_the_screen_never_switches() {
        let mut t = EdgeTracker::new(1000.0);
        for _ in 0..40 {
            assert!(t.feed(10.0, &cfg()).is_none()); // chega à borda direita (x = 900)
        }
        for _ in 0..40 {
            assert!(t.feed(-20.0, &cfg()).is_none());
        }
    }

    #[test]
    fn pushing_past_the_right_edge_switches_once() {
        let mut t = EdgeTracker::new(1000.0);
        let mut hits = 0;
        for _ in 0..200 {
            if let Some(Edge::Right) = t.feed(20.0, &cfg()) {
                hits += 1;
            }
        }
        // 500 até a borda + 300 de empurrão → uma troca; depois, 2 s de espera.
        assert_eq!(hits, 1);
    }

    #[test]
    fn moving_back_resets_the_push() {
        let mut t = EdgeTracker::new(1000.0);
        for _ in 0..25 {
            t.feed(20.0, &cfg()); // x na borda (500) + 0 de empurrão
        }
        for _ in 0..10 {
            assert!(t.feed(25.0, &cfg()).is_none()); // empurra 250 < 300
        }
        t.feed(-5.0, &cfg()); // volta: zera o empurrão
        for _ in 0..10 {
            assert!(t.feed(25.0, &cfg()).is_none());
        }
    }

    #[test]
    fn left_edge_reports_left() {
        let mut t = EdgeTracker::new(1000.0);
        let mut got = None;
        for _ in 0..100 {
            if let Some(e) = t.feed(-20.0, &cfg()) {
                got = Some(e);
                break;
            }
        }
        assert!(matches!(got, Some(Edge::Left)));
    }
}
