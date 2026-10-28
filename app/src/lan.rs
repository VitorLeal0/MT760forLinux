//! Modo LAN, compatível com o A HUB (JQNetworkLan + KCloudClipboard).
//!
//! Descoberta (verificada com o A HUB do Windows): JSON por UDP para o grupo
//! 228.12.23.34:12345 e para broadcast, a cada 2 s:
//!   dutyMark   = "4500-<ID do receptor, u32 hex>-<nome do computador>"
//!   appendData = "<10 IDs pareados no mouse, u32 hex>-<canal atual><canal deste PC>"
//! Área de transferência (experimental com o Windows): pacotes JQNetwork por
//! TCP na porta 33333, ação "SetClipboardToText" com {"text": ...}.

use crate::{config, hid, jqnet};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const GROUP: Ipv4Addr = Ipv4Addr::new(228, 12, 23, 34);
pub const LAN_PORT: u16 = 12345;
pub const MSG_PORT: u16 = 33333;
/// Porta que o A HUB do Windows mantém aberta: escutamos nela também, só
/// para registrar o que ele envia (formato ainda não confirmado).
pub const CAPTURE_PORTS: [u16; 1] = [43020];
const ANNOUNCE_EVERY: Duration = Duration::from_secs(2);
const NODE_TIMEOUT: Duration = Duration::from_secs(7);
const SLOTS: usize = 10;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Peer {
    pub mark: String,
    pub host: String,
    pub ip: String,
    /// ID RF do receptor desse computador.
    pub receiver: Option<[u8; 4]>,
    /// Canal do mouse em que esse receptor está pareado (0-based).
    pub own_channel: Option<u8>,
    /// Canal em que o mouse estava, segundo esse computador.
    pub current_channel: Option<u8>,
    pub last_seen_ms: u64,
}

/// Estado publicado pelo serviço para a interface gráfica.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Status {
    pub running: bool,
    pub self_ips: Vec<String>,
    pub peers: Vec<Peer>,
    pub clipboard_server: bool,
    pub last_event: String,
    pub updated_ms: u64,
}

pub fn status_path() -> std::path::PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(dir).join("mt760-lan.json")
}

pub fn capture_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join(".cache/mt760forlinux/lan-capture.log")
}

pub fn read_status() -> Option<Status> {
    let s: Status = serde_json::from_slice(&std::fs::read(status_path()).ok()?).ok()?;
    // Serviço parado há mais de 10 s: estado velho.
    (now_ms().saturating_sub(s.updated_ms) < 10_000).then_some(s)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn id_hex(id: &[u8; 4]) -> String {
    format!("{:08x}", u32::from_le_bytes(*id))
}

fn id_from_hex(s: &str) -> Option<[u8; 4]> {
    let v = u32::from_str_radix(s, 16).ok()?;
    (v != u32::MAX).then(|| v.to_le_bytes())
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "linux".into())
}

/// IPv4 das interfaces físicas (Wi-Fi/Ethernet), sem Docker, VPN etc.
pub fn local_ips() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    unsafe {
        let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut ifap) != 0 {
            return out;
        }
        let mut cur = ifap;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || (*ifa.ifa_addr).sa_family as i32 != libc::AF_INET {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy();
            if !(name.starts_with("wl") || name.starts_with("en") || name.starts_with("eth")) {
                continue;
            }
            let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
            let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            if !ip.is_loopback() && !ip.is_link_local() {
                out.push(ip);
            }
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// O que este computador sabe sobre o mouse, para o anúncio.
#[derive(Clone, Default)]
pub struct MouseInfo {
    pub receiver: Option<[u8; 4]>,
    pub paired: Vec<Option<[u8; 4]>>,
    pub channel: Option<u8>,
    /// O mouse está conectado ao receptor deste computador agora.
    pub present: bool,
}

impl MouseInfo {
    fn own_channel(&self) -> Option<u8> {
        let r = self.receiver?;
        self.paired.iter().position(|p| *p == Some(r)).map(|i| i as u8)
    }
}

fn announcement(mark: &str, index: u64, info: &MouseInfo, ips: &[Ipv4Addr], offline: bool) -> Vec<u8> {
    let receiver = info.receiver.map(|r| id_hex(&r)).unwrap_or_else(|| "ffffffff".into());
    let mut slots: String = info.paired.iter().take(SLOTS).map(|p| p.map(|id| id_hex(&id)).unwrap_or_else(|| "ffffffff".into())).collect();
    for _ in info.paired.len().min(SLOTS)..SLOTS {
        slots.push_str("ffffffff");
    }
    let suffix = format!("{:02x}{:02x}", info.channel.unwrap_or(0xff), info.own_channel().unwrap_or(0xff));
    // Como o A HUB do Windows: só quem está com o mouse anuncia a lista de canais.
    let append = if info.present { format!("{slots}-{suffix}") } else { String::new() };
    let data = json!({
        "appendData": append,
        "dataPackageIndex": index,
        "dutyMark": format!("{:04x}-{receiver}-{}", hid::WIRED_PID, hostname()),
        "ipList": ips.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        "nodeMarkSummary": mark,
        "requestFeedback": !offline,
        "requestOffline": offline,
    });
    serde_json::to_vec(&data).unwrap_or_default()
}

/// Interpreta o anúncio de outro A HUB.
fn parse_peer(v: &Value, from: Ipv4Addr) -> Option<Peer> {
    let mark = v.get("nodeMarkSummary")?.as_str()?.to_string();
    let duty = v.get("dutyMark").and_then(|d| d.as_str()).unwrap_or_default();
    let mut parts = duty.splitn(3, '-');
    let _pid = parts.next();
    let receiver = parts.next().and_then(id_from_hex);
    let host = parts.next().unwrap_or("?").to_string();
    let append = v.get("appendData").and_then(|a| a.as_str()).unwrap_or_default();
    let (_, suffix) = append.split_once('-').unwrap_or(("", ""));
    let byte = |s: &str| u8::from_str_radix(s, 16).ok().filter(|&b| b != 0xff);
    let current_channel = suffix.get(0..2).and_then(byte);
    let own_channel = suffix.get(2..4).and_then(byte);
    let ips: Vec<String> = v.get("ipList").and_then(|l| l.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // Prefere o IP de onde o pacote veio (o da mesma rede).
    let ip = if ips.iter().any(|i| i == &from.to_string()) || ips.is_empty() { from.to_string() } else { ips[0].clone() };
    Some(Peer { mark, host, ip, receiver, own_channel, current_channel, last_seen_ms: now_ms() })
}

pub struct Lan {
    status: Arc<Mutex<Status>>,
    info: Arc<Mutex<MouseInfo>>,
    last_remote_text: Arc<Mutex<Option<String>>>,
}

impl Lan {
    pub fn start() -> Lan {
        let lan = Lan {
            status: Arc::new(Mutex::new(Status::default())),
            info: Arc::new(Mutex::new(MouseInfo::default())),
            last_remote_text: Arc::new(Mutex::new(None)),
        };
        lan.spawn_mouse_info();
        lan.spawn_discovery();
        lan.spawn_server();
        lan.spawn_clipboard_watch();
        lan
    }

    fn event(status: &Mutex<Status>, msg: String) {
        eprintln!("[mt760-lan] {msg}");
        status.lock().unwrap().last_event = msg;
    }

    /// Lê periodicamente o ID do receptor e a lista do mouse.
    fn spawn_mouse_info(&self) {
        let info = self.info.clone();
        thread::spawn(move || {
            let mut last_full = Instant::now() - Duration::from_secs(60);
            loop {
                if let Ok(mut r) = hid::Receiver::open() {
                    let mut i = info.lock().unwrap().clone();
                    if i.receiver.is_none() {
                        i.receiver = r.own_id().ok();
                    }
                    match r.channel_now() {
                        Ok(c) => {
                            i.channel = Some(c);
                            i.present = true;
                            if last_full.elapsed() > Duration::from_secs(30) || i.paired.is_empty() {
                                if let Ok(p) = r.paired() {
                                    i.paired = p;
                                    last_full = Instant::now();
                                }
                            }
                        }
                        Err(_) => i.present = false,
                    }
                    *info.lock().unwrap() = i;
                }
                thread::sleep(Duration::from_secs(3));
            }
        });
    }

    fn spawn_discovery(&self) {
        let status = self.status.clone();
        let info = self.info.clone();
        thread::spawn(move || {
            let sock = match udp_socket() {
                Ok(s) => s,
                Err(e) => {
                    Self::event(&status, format!("não foi possível abrir a porta UDP {LAN_PORT}: {e}"));
                    return;
                }
            };
            let rsock: UdpSocket = match sock.try_clone() {
                Ok(s) => s.into(),
                Err(e) => {
                    Self::event(&status, format!("socket UDP: {e}"));
                    return;
                }
            };
            let mark = node_mark();
            let mut index: u64 = now_ms() / 1000 % 1_000_000;
            let mut last_send = Instant::now() - ANNOUNCE_EVERY;
            let mut peers: HashMap<String, Peer> = HashMap::new();
            let mut buf = [0u8; 65536];
            let mut was_announcing = false;
            loop {
                let cfg = config::load();
                let enabled = cfg.lan.enabled;
                // Anunciar faz o A HUB do Windows tratar este PC como parceiro
                // LAN, e aí ele deixa de devolver o mouse pela borda: por padrão
                // só escutamos.
                let announce = enabled && cfg.lan.announce;
                let ips = local_ips();
                for ip in &ips {
                    let _ = sock.join_multicast_v4(&GROUP, ip);
                }
                let send = |offline: bool, index: &mut u64, to_peer: Option<Ipv4Addr>| {
                    *index += 1;
                    let data = announcement(&mark, *index, &info.lock().unwrap(), &ips, offline);
                    for ip in &ips {
                        let _ = sock.set_multicast_if_v4(ip);
                        let _ = sock.send_to(&data, &SocketAddrV4::new(GROUP, LAN_PORT).into());
                    }
                    let _ = sock.send_to(&data, &SocketAddrV4::new(Ipv4Addr::BROADCAST, LAN_PORT).into());
                    if let Some(p) = to_peer {
                        let _ = sock.send_to(&data, &SocketAddrV4::new(p, LAN_PORT).into());
                    }
                };
                if was_announcing && !announce {
                    send(true, &mut index, None);
                }
                if !enabled {
                    peers.clear();
                }
                was_announcing = announce;
                if announce && last_send.elapsed() >= ANNOUNCE_EVERY {
                    last_send = Instant::now();
                    send(false, &mut index, None);
                }
                // Recebe por até 500 ms
                let deadline = Instant::now() + Duration::from_millis(500);
                while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                    let _ = rsock.set_read_timeout(Some(left.max(Duration::from_millis(1))));
                    let Ok((n, from)) = rsock.recv_from(&mut buf) else { break };
                    if !enabled {
                        continue;
                    }
                    let std::net::SocketAddr::V4(from) = from else { continue };
                    let from = *from.ip();
                    let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
                    let Some(peer) = parse_peer(&v, from) else { continue };
                    if peer.mark == mark {
                        continue;
                    }
                    if v.get("requestOffline").and_then(|b| b.as_bool()) == Some(true) {
                        if peers.remove(&peer.mark).is_some() {
                            Self::event(&status, format!("{} saiu da rede", peer.host));
                        }
                        continue;
                    }
                    let first = !peers.contains_key(&peer.mark);
                    if first {
                        Self::event(&status, format!("{} ({}) entrou na rede", peer.host, peer.ip));
                        if announce && v.get("requestFeedback").and_then(|b| b.as_bool()) == Some(true) {
                            send(false, &mut index, Some(from));
                        }
                    }
                    peers.insert(peer.mark.clone(), peer);
                }
                let now = now_ms();
                peers.retain(|_, p| now.saturating_sub(p.last_seen_ms) < NODE_TIMEOUT.as_millis() as u64);
                let mut st = status.lock().unwrap();
                st.running = enabled;
                st.self_ips = ips.iter().map(|i| i.to_string()).collect();
                st.peers = peers.values().cloned().collect();
                st.peers.sort_by(|a, b| a.host.cmp(&b.host));
                st.updated_ms = now;
                let _ = std::fs::write(status_path(), serde_json::to_vec(&*st).unwrap_or_default());
            }
        });
    }

    /// Servidor de mensagens: recebe a área de transferência dos outros PCs.
    fn spawn_server(&self) {
        for port in std::iter::once(MSG_PORT).chain(CAPTURE_PORTS) {
            let status = self.status.clone();
            let last_remote = self.last_remote_text.clone();
            thread::spawn(move || {
                let listener = match TcpListener::bind(("0.0.0.0", port)) {
                    Ok(l) => l,
                    Err(e) => {
                        Self::event(&status, format!("não foi possível abrir a porta TCP {port}: {e}"));
                        return;
                    }
                };
                if port == MSG_PORT {
                    status.lock().unwrap().clipboard_server = true;
                }
                for conn in listener.incoming().flatten() {
                    let status = status.clone();
                    let last_remote = last_remote.clone();
                    thread::spawn(move || handle_conn(conn, port, &status, &last_remote));
                }
            });
        }
    }

    /// Envia o texto copiado neste PC para os outros.
    fn spawn_clipboard_watch(&self) {
        let status = self.status.clone();
        let last_remote = self.last_remote_text.clone();
        thread::spawn(move || {
            let mut clip = loop {
                match arboard::Clipboard::new() {
                    Ok(c) => break c,
                    Err(_) => thread::sleep(Duration::from_secs(5)),
                }
            };
            let mut last = clip.get_text().ok();
            loop {
                thread::sleep(Duration::from_millis(700));
                let cfg = config::load();
                let Ok(text) = clip.get_text() else { continue };
                if Some(&text) == last.as_ref() {
                    continue;
                }
                last = Some(text.clone());
                if !(cfg.lan.enabled && cfg.lan.announce && cfg.lan.clipboard) || text.is_empty() {
                    continue;
                }
                if last_remote.lock().unwrap().as_ref() == Some(&text) {
                    continue; // veio de outro PC: não devolve
                }
                let peers = status.lock().unwrap().peers.clone();
                for p in peers {
                    match send_text(&p.ip, &text) {
                        Ok(()) => Self::event(&status, format!("área de transferência enviada para {}", p.host)),
                        Err(e) => Self::event(&status, format!("falha ao enviar para {}: {e}", p.host)),
                    }
                }
            }
        });
    }
}

fn udp_socket() -> std::io::Result<Socket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.set_multicast_loop_v4(false)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, LAN_PORT).into())?;
    Ok(s)
}

/// Identificador do nó: sempre o mesmo nesta máquina (derivado do
/// machine-id), para o A HUB do Windows não registrar um "Linux" novo a cada
/// reinício. Não expõe o machine-id em si.
pub fn node_mark() -> String {
    let id = std::fs::read_to_string("/etc/machine-id").unwrap_or_else(|_| hostname());
    let mut out = String::new();
    for salt in [0x9E37_79B9_7F4A_7C15u64, 0xC2B2_AE3D_27D4_EB4F] {
        // FNV-1a de 64 bits, duas vezes com sementes diferentes = 128 bits
        let mut h = 0xcbf2_9ce4_8422_2325u64 ^ salt;
        for b in format!("ahub-linux:{}", id.trim()).bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        out.push_str(&format!("{h:016x}"));
    }
    out
}

fn capture(note: &str, pkg: &jqnet::Package) {
    let p = capture_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let preview = |v: &[u8]| String::from_utf8_lossy(&v[..v.len().min(2000)]).into_owned();
        let _ = writeln!(
            f,
            "[{}] {note}: flag={} random={} meta={} payload({} bytes)={}",
            now_ms(),
            pkg.flag,
            pkg.random,
            preview(&pkg.meta),
            pkg.payload.len(),
            preview(&pkg.payload)
        );
    }
}

/// Grava os primeiros bytes de cada conexão, mesmo que não sejam JQNetwork.
fn capture_raw(peer: &str, port: u16, conn: &TcpStream) {
    let mut buf = [0u8; 512];
    let _ = conn.set_read_timeout(Some(Duration::from_secs(3)));
    let n = conn.peek(&mut buf).unwrap_or(0);
    let p = capture_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let hex: Vec<String> = buf[..n].iter().map(|b| format!("{b:02x}")).collect();
        let _ = writeln!(f, "[{}] conexão de {peer} na porta {port}: {n} bytes: {}", now_ms(), hex.join(" "));
        let _ = writeln!(f, "    texto: {}", String::from_utf8_lossy(&buf[..n]).replace('\n', "\\n"));
    }
}

fn handle_conn(mut conn: TcpStream, port: u16, status: &Mutex<Status>, last_remote: &Mutex<Option<String>>) {
    let peer = conn.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
    Lan::event(status, format!("{peer} conectou na porta {port}"));
    capture_raw(&peer, port, &conn);
    let _ = conn.set_read_timeout(Some(Duration::from_secs(60)));
    while let Ok(pkg) = jqnet::Package::read_from(&mut conn) {
        let action = pkg.action();
        let text = pkg.payload_json().and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string));
        match (action.as_str(), text) {
            ("SetClipboardToText", Some(text)) if !pkg.partial => {
                let cfg = config::load();
                let ok = cfg.lan.enabled && cfg.lan.announce && cfg.lan.clipboard;
                if ok {
                    *last_remote.lock().unwrap() = Some(text.clone());
                    let set = arboard::Clipboard::new().and_then(|mut c| c.set_text(text));
                    Lan::event(status, match set {
                        Ok(()) => format!("texto recebido de {peer}"),
                        Err(e) => format!("texto recebido de {peer}, mas não foi possível colar: {e}"),
                    });
                }
                let reply = jqnet::Package::new("", &json!({ "succeed": ok, "message": if ok { "" } else { "disabled" } }), pkg.random);
                let _ = reply.write_to(&mut conn);
            }
            _ => {
                // Formato ainda desconhecido (imagens, arquivos, SetDeviceArray…):
                // guarda para análise e responde sem erro.
                capture(&format!("de {peer}, ação '{action}'"), &pkg);
                Lan::event(status, format!("mensagem '{action}' de {peer} gravada em {}", capture_path().display()));
                let reply = jqnet::Package::new("", &json!({ "succeed": false, "message": "unsupported" }), pkg.random);
                let _ = reply.write_to(&mut conn);
            }
        }
    }
}

/// Envia texto para outro A HUB (porta 33333, ação SetClipboardToText).
pub fn send_text(ip: &str, text: &str) -> std::io::Result<()> {
    let addr: std::net::SocketAddr = format!("{ip}:{MSG_PORT}").parse().map_err(|_| std::io::Error::other("IP inválido"))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    let pkg = jqnet::Package::new("SetClipboardToText", &json!({ "text": text }), jqnet::random_flag());
    pkg.write_to(&mut s)?;
    match jqnet::Package::read_from(&mut s) {
        Ok(reply) => {
            if reply.payload_json().and_then(|v| v.get("succeed").and_then(|b| b.as_bool())) == Some(false) {
                capture(&format!("resposta de {ip}"), &reply);
                return Err(std::io::Error::other("o outro computador recusou"));
            }
            Ok(())
        }
        // Sem resposta: o texto pode ter chegado mesmo assim.
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_real_windows_announcement() {
        let raw = r#"{"appendData":"112233445566778899aabbccffffffffffffffffffffffffffffffffffffffff-0201","dataPackageIndex":88,"dutyMark":"4500-55667788-PC-Windows","ipList":["192.168.1.20","172.16.0.1"],"nodeMarkSummary":"0123456789abcdef0123456789abcdef","requestFeedback":true,"requestOffline":false}"#;
        let v: Value = serde_json::from_str(raw).unwrap();
        let p = parse_peer(&v, Ipv4Addr::new(192, 168, 0, 50)).unwrap();
        assert_eq!(p.host, "PC-Windows");
        assert_eq!(p.ip, "192.168.1.20");
        assert_eq!(p.receiver, Some([0x88, 0x77, 0x66, 0x55])); // canal 2 do mouse
        assert_eq!(p.own_channel, Some(1));
        assert_eq!(p.current_channel, Some(2));
    }

    #[test]
    fn our_announcement_matches_the_windows_format() {
        let info = MouseInfo {
            receiver: Some([0xCC, 0xBB, 0xAA, 0x99]),
            paired: vec![Some([0x44, 0x33, 0x22, 0x11]), Some([0x88, 0x77, 0x66, 0x55]), Some([0xCC, 0xBB, 0xAA, 0x99]), None],
            channel: Some(2),
            present: true,
        };
        let data = announcement("fedcba9876543210fedcba9876543210", 5, &info, &[Ipv4Addr::new(192, 168, 0, 35)], false);
        let v: Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(v["appendData"], format!("112233445566778899aabbcc{}-0202", "ffffffff".repeat(7)));
        assert!(v["dutyMark"].as_str().unwrap().starts_with("4500-99aabbcc-"));
        // Sem o mouse aqui, a lista não é anunciada.
        let away = MouseInfo { present: false, ..info.clone() };
        let v2: Value = serde_json::from_slice(&announcement("x", 6, &away, &[], false)).unwrap();
        assert_eq!(v2["appendData"], "");
        assert_eq!(node_mark(), node_mark());
        assert_eq!(node_mark().len(), 32);
        // E nós mesmos conseguimos ler o que anunciamos.
        let p = parse_peer(&v, Ipv4Addr::new(192, 168, 0, 35)).unwrap();
        assert_eq!(p.own_channel, Some(2));
    }
}
