//! Ações gravadas nos botões do mouse (formato de 4 bytes do MouseForTelink).

/// Botões físicos, na ordem do MT760.json, com o ponto sobre a foto
/// (fração da largura/altura da imagem) e o lado do rótulo (true = direita).
pub struct PhysKey {
    pub name: &'static str,
    pub tip: (f32, f32),
    pub right: bool,
}

pub const PHYS_KEYS: [PhysKey; 11] = [
    PhysKey { name: "Botão esquerdo", tip: (0.30, 0.12), right: false },
    PhysKey { name: "Botão do meio", tip: (0.55, 0.15), right: true },
    PhysKey { name: "Botão direito", tip: (0.80, 0.20), right: true },
    PhysKey { name: "Botão DIY", tip: (0.67, 0.27), right: true },
    PhysKey { name: "Botão DPI", tip: (0.73, 0.32), right: true },
    PhysKey { name: "Lateral avançar", tip: (0.34, 0.46), right: false },
    PhysKey { name: "Lateral voltar", tip: (0.40, 0.61), right: false },
    PhysKey { name: "Roda lateral (cima)", tip: (0.50, 0.49), right: false },
    PhysKey { name: "Roda lateral (baixo)", tip: (0.50, 0.56), right: false },
    PhysKey { name: "Roda para cima", tip: (0.55, 0.10), right: true },
    PhysKey { name: "Roda para baixo", tip: (0.58, 0.19), right: true },
];

/// Configuração de fábrica (lida de um MT760L novo).
pub const FACTORY: [[u8; 4]; 11] = [
    [0x03, 0, 0x01, 0],
    [0x03, 0, 0x04, 0],
    [0x03, 0, 0x02, 0],
    [0x08, 0, 0x04, 0],
    [0x08, 0, 0x03, 0],
    [0x03, 0, 0x10, 0],
    [0x03, 0, 0x08, 0],
    [0x0C, 0, 0x01, 0],
    [0x0C, 0, 0x02, 0],
    [0x0B, 0, 0x01, 0],
    [0x0B, 0, 0x02, 0],
];

pub struct Action {
    pub label: &'static str,
    pub code: [u8; 4],
}

const fn mouse(label: &'static str, mask: u8) -> Action {
    Action { label, code: [0x03, 0, mask, 0] }
}
const fn consumer(label: &'static str, usage: u16) -> Action {
    Action { label, code: [0x04, 0, (usage & 0xFF) as u8, (usage >> 8) as u8] }
}
const fn key(label: &'static str, usage: u8) -> Action {
    Action { label, code: [0x01, 0, usage, 0] }
}
const fn chord(label: &'static str, usage: u8, modifier: u8) -> Action {
    Action { label, code: [0x02, 0, usage, modifier] }
}

// Modificadores HID
const CTRL: u8 = 0x01;
const SHIFT: u8 = 0x02;
const ALT: u8 = 0x04;
const SUPER: u8 = 0x08;

pub struct Category {
    pub title: &'static str,
    pub actions: &'static [Action],
}

pub const CATEGORIES: &[Category] = &[
    Category {
        title: "Mouse",
        actions: &[
            mouse("Clique esquerdo", 0x01),
            mouse("Clique direito", 0x02),
            mouse("Botão do meio", 0x04),
            mouse("Voltar", 0x08),
            mouse("Avançar", 0x10),
        ],
    },
    Category {
        title: "Rolagem",
        actions: &[
            Action { label: "Rolar para cima", code: [0x0B, 0, 0x01, 0] },
            Action { label: "Rolar para baixo", code: [0x0B, 0, 0x02, 0] },
            Action { label: "Rolar para a esquerda", code: [0x0C, 0, 0x01, 0] },
            Action { label: "Rolar para a direita", code: [0x0C, 0, 0x02, 0] },
        ],
    },
    Category {
        title: "DPI",
        actions: &[
            Action { label: "DPI cíclico +", code: [0x08, 0, 0x03, 0] },
            Action { label: "DPI cíclico −", code: [0x08, 0, 0x04, 0] },
            Action { label: "DPI +", code: [0x08, 0, 0x05, 0] },
            Action { label: "DPI −", code: [0x08, 0, 0x06, 0] },
        ],
    },
    Category {
        title: "Mídia",
        actions: &[
            consumer("Reproduzir/Pausar", 0xCD),
            consumer("Próxima faixa", 0xB5),
            consumer("Faixa anterior", 0xB6),
            consumer("Parar", 0xB7),
            consumer("Aumentar volume", 0xE9),
            consumer("Diminuir volume", 0xEA),
            consumer("Mudo", 0xE2),
            consumer("Aumentar brilho", 0x6F),
            consumer("Diminuir brilho", 0x70),
            consumer("Navegador: voltar", 0x224),
            consumer("Navegador: avançar", 0x225),
            consumer("Navegador: recarregar", 0x227),
            consumer("Página inicial", 0x223),
            consumer("E-mail", 0x18A),
            consumer("Calculadora", 0x192),
        ],
    },
    Category {
        title: "Atalhos",
        actions: &[
            chord("Copiar (Ctrl+C)", 0x06, CTRL),
            chord("Colar (Ctrl+V)", 0x19, CTRL),
            chord("Recortar (Ctrl+X)", 0x1B, CTRL),
            chord("Desfazer (Ctrl+Z)", 0x1D, CTRL),
            chord("Refazer (Ctrl+Shift+Z)", 0x1D, CTRL | SHIFT),
            chord("Selecionar tudo (Ctrl+A)", 0x04, CTRL),
            chord("Localizar (Ctrl+F)", 0x09, CTRL),
            chord("Salvar (Ctrl+S)", 0x16, CTRL),
            chord("Nova aba (Ctrl+T)", 0x17, CTRL),
            chord("Fechar aba (Ctrl+W)", 0x1A, CTRL),
            chord("Reabrir aba (Ctrl+Shift+T)", 0x17, CTRL | SHIFT),
            chord("Próxima aba (Ctrl+PgDn)", 0x4E, CTRL),
            chord("Aba anterior (Ctrl+PgUp)", 0x4B, CTRL),
            chord("Ampliar (Ctrl+=)", 0x2E, CTRL),
            chord("Reduzir (Ctrl+−)", 0x2D, CTRL),
            chord("Fechar janela (Alt+F4)", 0x3D, ALT),
            chord("Alternar janelas (Alt+Tab)", 0x2B, ALT),
            chord("Visão geral (Super)", 0x00, SUPER),
            chord("Mostrar área de trabalho (Super+D)", 0x07, SUPER),
            chord("Bloquear tela (Super+L)", 0x0F, SUPER),
        ],
    },
    Category {
        title: "Teclas",
        actions: &[
            key("Enter", 0x28),
            key("Esc", 0x29),
            key("Tab", 0x2B),
            key("Espaço", 0x2C),
            key("Backspace", 0x2A),
            key("Delete", 0x4C),
            key("Home", 0x4A),
            key("End", 0x4D),
            key("Page Up", 0x4B),
            key("Page Down", 0x4E),
            key("Seta cima", 0x52),
            key("Seta baixo", 0x51),
            key("Seta esquerda", 0x50),
            key("Seta direita", 0x4F),
            key("Print Screen", 0x46),
            key("F5", 0x3E),
            key("F11", 0x44),
        ],
    },
    Category {
        title: "Outros",
        actions: &[Action { label: "Desativado", code: [0x07, 0, 0, 0] }],
    },
];

pub fn describe(code: [u8; 4]) -> String {
    for cat in CATEGORIES {
        for a in cat.actions {
            if a.code == code {
                return a.label.to_string();
            }
        }
    }
    match code[0] {
        0x05 => format!("Macro {}", code[2]),
        0x01 => format!("Tecla 0x{:02X}", code[2]),
        0x04 => format!("Mídia 0x{:04X}", u16::from_le_bytes([code[2], code[3]])),
        _ => format!("{:02X} {:02X} {:02X} {:02X}", code[0], code[1], code[2], code[3]),
    }
}
