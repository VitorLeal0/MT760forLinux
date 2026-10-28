use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    pub cross: Cross,
    /// Cor do produto mostrada na imagem (1 cinza, 2 preto, 3 rosa).
    pub color: u8,
    /// Nomes dados aos canais do mouse (índice 0 = canal 1); vazio = sem nome.
    pub channel_names: Vec<String>,
    pub lan: Lan,
    /// Macros editadas na interface, por espaço ("0".."11").
    pub macros: std::collections::BTreeMap<String, crate::macros::Macro>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Lan {
    /// Anunciar este computador e procurar outros A HUB na rede.
    pub enabled: bool,
    /// Anunciar este computador na rede (avançado: atrapalha o A HUB do Windows).
    pub announce: bool,
    /// Compartilhar o texto copiado com os outros computadores.
    pub clipboard: bool,
    /// Não usado mais (o identificador vem do machine-id); mantido para ler
    /// configurações antigas.
    #[serde(skip_serializing)]
    pub node_mark: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Cross {
    pub enabled: bool,
    /// Canal do mouse ao sair pela borda direita / esquerda.
    pub right: Option<u8>,
    pub left: Option<u8>,
    /// Largura da tela em contagens do sensor (calibrada na interface).
    pub width_counts: f64,
    /// Quanto é preciso continuar empurrando contra a borda para trocar.
    pub push_counts: f64,
}

impl Default for Cross {
    fn default() -> Self {
        Cross { enabled: false, right: None, left: None, width_counts: 3000.0, push_counts: 350.0 }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { cross: Cross::default(), color: 2, channel_names: Vec::new(), lan: Lan::default(), macros: Default::default() }
    }
}

impl Config {
    pub fn channel_name(&self, ch: u8) -> Option<&str> {
        self.channel_names.get(ch as usize).map(|s| s.trim()).filter(|s| !s.is_empty())
    }

    /// "Canal 2 · Windows" ou só "Canal 2".
    pub fn channel_label(&self, ch: u8) -> String {
        match self.channel_name(ch) {
            Some(n) => format!("Canal {} · {n}", ch + 1),
            None => format!("Canal {}", ch + 1),
        }
    }

    pub fn set_channel_name(&mut self, ch: u8, name: &str) {
        let i = ch as usize;
        if self.channel_names.len() <= i {
            self.channel_names.resize(i + 1, String::new());
        }
        self.channel_names[i] = name.to_string();
        while self.channel_names.last().is_some_and(|s| s.trim().is_empty()) {
            self.channel_names.pop();
        }
    }

    /// Canal pelo número (1-based) ou pelo nome (sem diferenciar maiúsculas).
    pub fn find_channel(&self, arg: &str) -> Option<u8> {
        if let Ok(n) = arg.parse::<u8>() {
            return n.checked_sub(1);
        }
        let arg = arg.trim().to_lowercase();
        (0..self.channel_names.len() as u8).find(|&c| self.channel_name(c).is_some_and(|n| n.to_lowercase() == arg))
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("mt760forlinux").join("config.toml")
}

/// Caminho usado pelas versões de desenvolvimento (copiado na primeira execução).
fn old_path() -> PathBuf {
    path().parent().and_then(|p| p.parent()).unwrap_or(std::path::Path::new(".")).join("rapoo-mt760").join("ahub.toml")
}

pub fn load() -> Config {
    if !path().exists() && old_path().exists() {
        if let Some(dir) = path().parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::copy(old_path(), path());
    }
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, toml::to_string_pretty(cfg).unwrap_or_default())?;
    std::fs::rename(tmp, p)
}

pub fn mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_lookup() {
        let mut c = Config::default();
        assert_eq!(c.channel_label(1), "Canal 2");
        c.set_channel_name(1, "PC Windows");
        c.set_channel_name(2, "Notebook Linux");
        assert_eq!(c.channel_label(1), "Canal 2 · PC Windows");
        assert_eq!(c.find_channel("pc windows"), Some(1));
        assert_eq!(c.find_channel("3"), Some(2));
        assert_eq!(c.find_channel("0"), None);
        assert_eq!(c.find_channel("Mac"), None);
        // apagar o último nome não deixa entradas vazias no arquivo
        c.set_channel_name(2, "  ");
        assert_eq!(c.channel_names, vec![String::new(), "PC Windows".to_string()]);
        let round: Config = toml::from_str(&toml::to_string(&c).unwrap()).unwrap();
        assert_eq!(round.channel_label(1), "Canal 2 · PC Windows");
    }
}
