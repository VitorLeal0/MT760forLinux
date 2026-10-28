//! Macros gravadas na memória do mouse (MouseForTelink::setMacro).
//!
//! Cada macro ocupa 0x1000 bytes a partir de 0x1000 + id × 0x1000 (12 espaços) e é
//! gravada em blocos de 24 bytes. Conteúdo:
//!   u16 LE  comprimento total em bytes (cabeçalho incluído)
//!   u16 LE  reprodução: N vezes | 0xFFFE liga/desliga | 0xFFFF enquanto pressionada
//!   eventos de 3 bytes: u16 LE (atraso_ms × 2 | ação) + código
//!     ação 1 = pressionar, 0 = soltar; código = HID usage do teclado ou
//!     0xF0..0xF4 para os botões do mouse.
//! O mouse só grava a macro quando recebe todos os bytes anunciados no
//! cabeçalho, e no máximo 48 bytes (14 eventos de tecla/botão).
//! Um evento "atraso" da interface vira o atraso do evento anterior
//! (limitado a 2..=32767 ms; acima disso o A HUB usa 30000).

use serde::{Deserialize, Serialize};

pub const BASE: u32 = 0x1000;
pub const SLOT_SIZE: u32 = 0x1000;
/// O MT760L responde nos espaços 0x1000..=0xC000; de 0xD000 em diante não.
pub const MAX_MACROS: usize = 12;
/// O firmware só aceita macros de até 48 bytes (testado: 46 grava, 49 é
/// rejeitado mesmo com pausas longas) = cabeçalho + 14 eventos.
pub const MAX_LEN: usize = 48;
pub const MAX_EVENTS: usize = (MAX_LEN - HEADER) / 3;
const HEADER: usize = 4;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Play {
    Times(u16),
    Toggle,
    WhileHeld,
}

impl Default for Play {
    fn default() -> Self {
        Play::Times(1)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Tecla do teclado (HID usage), pressionar ou soltar.
    Key { usage: u8, down: bool },
    /// Botão do mouse: 0xF0 esquerdo, F1 direito, F2 meio, F3 voltar, F4 avançar.
    Mouse { button: u8, down: bool },
    /// Pausa em milissegundos.
    Delay { ms: u32 },
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Macro {
    pub name: String,
    pub play: Play,
    pub ignore_delay: bool,
    pub steps: Vec<Step>,
}

/// Eventos de tecla/botão (os atrasos não ocupam espaço próprio).
pub fn event_count(m: &Macro) -> usize {
    m.steps.iter().filter(|s| !matches!(s, Step::Delay { .. })).count()
}

pub fn slot_addr(id: usize) -> u32 {
    BASE + id as u32 * SLOT_SIZE
}

fn clamp_delay(ms: u32) -> u16 {
    if ms > 0x7FFF {
        30000
    } else {
        ms.max(2) as u16
    }
}

/// Codifica como o A HUB. Atrasos antes do primeiro evento são ignorados.
pub fn encode(m: &Macro) -> Vec<u8> {
    let mut out = vec![0u8; HEADER];
    let repeat: u16 = match m.play {
        Play::Times(n) => n.max(1),
        Play::Toggle => 0xFFFE,
        Play::WhileHeld => 0xFFFF,
    };
    out[2..4].copy_from_slice(&repeat.to_le_bytes());
    let mut last: Option<usize> = None;
    for s in &m.steps {
        match *s {
            Step::Key { usage: code, down } | Step::Mouse { button: code, down } => {
                last = Some(out.len());
                out.extend([down as u8, 0, code]);
            }
            Step::Delay { ms } => {
                if let Some(i) = last {
                    let d = if m.ignore_delay { 2 } else { clamp_delay(ms) };
                    let word = (d << 1) | out[i] as u16 & 1;
                    out[i..i + 2].copy_from_slice(&word.to_le_bytes());
                }
            }
        }
        if out.len() + 3 > MAX_LEN {
            break;
        }
    }
    let len = out.len() as u16;
    out[0..2].copy_from_slice(&len.to_le_bytes());
    out
}

/// Lê de volta o que estiver gravado (só eventos de tecla/botão; outros
/// tipos de registro encerram a leitura).
pub fn decode(data: &[u8]) -> Option<(Play, Vec<Step>)> {
    if data.len() < HEADER {
        return None;
    }
    let len = u16::from_le_bytes([data[0], data[1]]) as usize;
    if len < HEADER || len == 0xFFFF || len > data.len() {
        return None;
    }
    let play = match u16::from_le_bytes([data[2], data[3]]) {
        0xFFFE => Play::Toggle,
        0xFFFF => Play::WhileHeld,
        n => Play::Times(n),
    };
    let mut steps = Vec::new();
    let mut i = HEADER;
    while i + 3 <= len {
        if matches!(data[i + 2], 0xF7 | 0xF8) {
            break; // movimento/roda: não suportado aqui
        }
        let word = u16::from_le_bytes([data[i], data[i + 1]]);
        let (down, code) = (word & 1 == 1, data[i + 2]);
        steps.push(if (0xF0..=0xF4).contains(&code) {
            Step::Mouse { button: code, down }
        } else {
            Step::Key { usage: code, down }
        });
        let ms = (word >> 1) as u32;
        if ms > 0 {
            steps.push(Step::Delay { ms });
        }
        i += 3;
    }
    Some((play, steps))
}

/// Nome de uma tecla HID para mostrar no editor.
pub fn usage_name(u: u8) -> String {
    let named = match u {
        0x04..=0x1D => return ((b'A' + u - 0x04) as char).to_string(),
        0x1E..=0x26 => return ((b'1' + u - 0x1E) as char).to_string(),
        0x27 => "0",
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Backspace",
        0x2B => "Tab",
        0x2C => "Espaço",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps Lock",
        0x3A..=0x45 => return format!("F{}", u - 0x39),
        0x46 => "Print Screen",
        0x49 => "Insert",
        0x4A => "Home",
        0x4B => "Page Up",
        0x4C => "Delete",
        0x4D => "End",
        0x4E => "Page Down",
        0x4F => "→",
        0x50 => "←",
        0x51 => "↓",
        0x52 => "↑",
        0xE0 => "Ctrl",
        0xE1 => "Shift",
        0xE2 => "Alt",
        0xE3 => "Super",
        0xE4 => "Ctrl direito",
        0xE5 => "Shift direito",
        0xE6 => "AltGr",
        0xE7 => "Super direito",
        _ => return format!("Tecla 0x{u:02X}"),
    };
    named.to_string()
}

pub fn mouse_name(b: u8) -> &'static str {
    match b {
        0xF0 => "Botão esquerdo",
        0xF1 => "Botão direito",
        0xF2 => "Botão do meio",
        0xF3 => "Voltar",
        _ => "Avançar",
    }
}

/// Tecla do egui → HID usage.
pub fn egui_key_usage(k: eframe::egui::Key) -> Option<u8> {
    use eframe::egui::Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    if let Some(i) = letters.iter().position(|x| *x == k) {
        return Some(0x04 + i as u8);
    }
    let digits = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
    if let Some(i) = digits.iter().position(|x| *x == k) {
        return Some(0x1E + i as u8);
    }
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(i) = fkeys.iter().position(|x| *x == k) {
        return Some(0x3A + i as u8);
    }
    Some(match k {
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equals => 0x2E,
        OpenBracket => 0x2F,
        CloseBracket => 0x30,
        Backslash => 0x31,
        Semicolon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash => 0x38,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_the_a_hub() {
        // Ctrl+C com 30 ms entre os eventos, repetir 1 vez
        let m = Macro {
            name: "copiar".into(),
            play: Play::Times(1),
            ignore_delay: false,
            steps: vec![
                Step::Key { usage: 0xE0, down: true },
                Step::Delay { ms: 30 },
                Step::Key { usage: 0x06, down: true },
                Step::Delay { ms: 30 },
                Step::Key { usage: 0x06, down: false },
                Step::Delay { ms: 30 },
                Step::Key { usage: 0xE0, down: false },
            ],
        };
        let b = encode(&m);
        assert_eq!(&b[..4], &[16, 0, 1, 0]); // 4 + 4×3 bytes, 1 repetição
        assert_eq!(&b[4..7], &[61, 0, 0xE0]); // 30×2 | 1 = 61
        assert_eq!(&b[7..10], &[61, 0, 0x06]);
        assert_eq!(&b[10..13], &[60, 0, 0x06]); // soltar: bit 0 = 0
        assert_eq!(&b[13..16], &[0, 0, 0xE0]); // último sem atraso
        let (play, steps) = decode(&b).unwrap();
        assert_eq!(play, Play::Times(1));
        assert_eq!(steps, m.steps);
    }

    #[test]
    fn delay_limits_and_modes() {
        let mut m = Macro {
            play: Play::WhileHeld,
            steps: vec![Step::Delay { ms: 500 }, Step::Mouse { button: 0xF0, down: true }, Step::Delay { ms: 1 }, Step::Mouse { button: 0xF0, down: false }, Step::Delay { ms: 99999 }],
            ..Default::default()
        };
        let b = encode(&m);
        assert_eq!(&b[2..4], &[0xFF, 0xFF]);
        assert_eq!(b.len(), 10); // atraso inicial descartado
        assert_eq!(u16::from_le_bytes([b[4], b[5]]), (2 << 1) | 1); // mínimo 2 ms
        assert_eq!(u16::from_le_bytes([b[7], b[8]]), 30000 << 1); // acima do limite: 30 s
        m.ignore_delay = true;
        m.play = Play::Toggle;
        let b = encode(&m);
        assert_eq!(&b[2..4], &[0xFE, 0xFF]);
        assert_eq!(u16::from_le_bytes([b[7], b[8]]), 2 << 1);
    }

    #[test]
    fn respects_the_firmware_limit() {
        let mut m = Macro::default();
        for _ in 0..20 {
            m.steps.push(Step::Key { usage: 0x04, down: true });
        }
        let b = encode(&m);
        assert!(b.len() <= MAX_LEN);
        assert_eq!(b.len(), 4 + 3 * MAX_EVENTS);
        assert_eq!(u16::from_le_bytes([b[0], b[1]]) as usize, b.len());
    }

    #[test]
    fn empty_slots() {
        assert!(decode(&[0xFF; 8]).is_none());
        assert!(decode(&[0, 0, 0, 0]).is_none());
    }
}

#[cfg(test)]
mod hardware {
    use super::*;

    /// Precisa do mouse acordado: `cargo test --release -- --ignored`
    #[test]
    #[ignore]
    fn write_read_delete_on_the_mouse() {
        let mut r = crate::hid::Receiver::open().expect("receptor");
        let id = MAX_MACROS - 1;
        let mut m = Macro::default();
        for i in 0..7u8 {
            m.steps.push(Step::Key { usage: 0x04 + i, down: true });
            m.steps.push(Step::Delay { ms: 10 + i as u32 });
            m.steps.push(Step::Key { usage: 0x04 + i, down: false });
        }
        let data = encode(&m);
        assert_eq!(data.len(), 46, "dois blocos, perto do limite");
        r.write_macro(id, &data).unwrap();
        let mut back = None;
        for _ in 0..5 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            back = r.read_macro(id).unwrap();
            if back.as_deref() == Some(&data[..]) {
                break;
            }
        }
        assert_eq!(back.as_deref(), Some(&data[..]));
        assert_eq!(decode(&back.unwrap()).unwrap().1, m.steps);
        r.delete_macro(id).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(r.read_macro(id).unwrap(), None);
    }
}
