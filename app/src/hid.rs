//! Protocolo HID do MT760L pelo receptor 24AE:1870 (ver ../PROTOCOL.md).
//!
//! Escrita: `write()` de 32 bytes `BA ...`; resposta: `HIDIOCGINPUT(32)`,
//! que volta sem o report ID: `01` = ok, `02` = mouse inacessível (dormindo
//! ou em outro canal), outro = comando rejeitado.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

pub const VENDOR: u16 = 0x24AE;
pub const RECEIVER_PID: u16 = 0x1870;
pub const WIRED_PID: u16 = 0x4500;

const REPORT_ID: u8 = 0xBA;
/// Comandos endereçados ao mouse (através do receptor).
const TO_MOUSE: u8 = 0xA5;
/// Comandos endereçados ao próprio receptor.
const TO_DONGLE: u8 = 0xA3;
const MEM_READ: u8 = 0xA4;
const MEM_WRITE: u8 = 0xA5;

// Endereços na memória do mouse (MouseForTelink).
pub const ADDR_CHANNEL: u32 = 0x23;
pub const ADDR_PAIRED: u32 = 0x34;
pub const ADDR_KEYS: u32 = 0x600;
pub const ADDR_POLLING: u32 = 0x630;
pub const ADDR_DPI_X: u32 = 0x638;
pub const ADDR_DPI_Y: u32 = 0x64C;
pub const KEY_COUNT: usize = 11;
pub const DPI_LEVELS: usize = 7;

#[derive(Debug)]
pub enum HidError {
    NotFound,
    Asleep,
    Rejected(u8),
    Io(std::io::Error),
}

impl std::fmt::Display for HidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HidError::NotFound => write!(f, "receptor Rapoo não encontrado"),
            HidError::Asleep => write!(f, "o mouse não respondeu (mexa o mouse para acordá-lo)"),
            HidError::Rejected(c) => write!(f, "comando rejeitado (código {c:#04x})"),
            HidError::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                write!(f, "sem permissão no hidraw (rode o install.sh)")
            }
            HidError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for HidError {
    fn from(e: std::io::Error) -> Self {
        HidError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, HidError>;

fn sysread(p: impl AsRef<std::path::Path>) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

/// Interface hidraw com o ID e o PID, a partir do sysfs.
fn hid_id(dev: &std::path::Path) -> Option<(u16, u16)> {
    let uevent = sysread(dev.join("uevent"));
    let id = uevent.lines().find_map(|l| l.strip_prefix("HID_ID="))?;
    let mut parts = id.split(':');
    parts.next()?;
    let vid = u32::from_str_radix(parts.next()?, 16).ok()? as u16;
    let pid = u32::from_str_radix(parts.next()?, 16).ok()? as u16;
    Some((vid, pid))
}

/// Procura as interfaces hidraw Rapoo; `pred` recebe o report descriptor.
pub fn find_hidraw(pid: u16, pred: impl Fn(&[u8]) -> bool) -> Option<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir("/sys/class/hidraw").ok()?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let dev = e.path().join("device");
        if hid_id(&dev) != Some((VENDOR, pid)) {
            continue;
        }
        let desc = fs::read(dev.join("report_descriptor")).unwrap_or_default();
        if pred(&desc) {
            return Some(PathBuf::from("/dev").join(e.file_name()));
        }
    }
    None
}

pub fn wired_connected() -> bool {
    find_hidraw(WIRED_PID, |_| true).is_some()
}

fn has_report_ba(desc: &[u8]) -> bool {
    desc.windows(2).any(|w| w == [0x85, 0xBA])
}

/// Trava entre processos (interface gráfica e serviço de cross-screen),
/// para que um par escrita/leitura não se misture com o de outro processo.
struct Lock(File);

impl Lock {
    fn acquire() -> Option<Lock> {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let f = OpenOptions::new().create(true).write(true).open(format!("{dir}/mt760-hid.lock")).ok()?;
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        Some(Lock(f))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub struct Receiver {
    file: File,
    pub path: PathBuf,
}

impl Receiver {
    pub fn open() -> Result<Receiver> {
        let path = find_hidraw(RECEIVER_PID, has_report_ba).ok_or(HidError::NotFound)?;
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        Ok(Receiver { file, path })
    }

    /// HIDIOCGINPUT(32): a próxima resposta pendente do receptor.
    fn get_input(&mut self) -> std::io::Result<Vec<u8>> {
        let mut buf = [0u8; 32];
        buf[0] = REPORT_ID;
        // _IOC(_IOC_READ|_IOC_WRITE, 'H', 0x0A, len)
        let req = (3u64 << 30) | ((buf.len() as u64) << 16) | ((b'H' as u64) << 8) | 0x0A;
        let n = unsafe { libc::ioctl(self.file.as_raw_fd(), req as _, buf.as_mut_ptr()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(buf[..(n as usize).max(1)].to_vec())
    }

    /// Um comando e uma resposta, sem repetição.
    pub fn transact(&mut self, cmd: &[u8]) -> Result<Vec<u8>> {
        let _lock = Lock::acquire();
        // Se o mouse demorou a responder a um comando anterior, a resposta
        // atrasada fica na fila e seria entregue a este comando: descarta.
        for _ in 0..4 {
            if self.get_input().is_err() {
                break;
            }
        }
        let mut pkt = [0u8; 32];
        pkt[..cmd.len()].copy_from_slice(cmd);
        self.file.write_all(&pkt)?;
        thread::sleep(Duration::from_millis(4));
        let resp = match self.get_input() {
            Ok(r) => r,
            // EPIPE: o receptor ainda não tinha resposta
            Err(e) if e.raw_os_error() == Some(libc::EPIPE) => return Err(HidError::Asleep),
            Err(e) => return Err(e.into()),
        };
        match resp[0] {
            1 => Ok(resp),
            2 => Err(HidError::Asleep),
            c => Err(HidError::Rejected(c)),
        }
    }

    /// Repete enquanto o mouse estiver dormindo, até `timeout`.
    pub fn request(&mut self, cmd: &[u8], timeout: Duration) -> Result<Vec<u8>> {
        let start = Instant::now();
        loop {
            match self.transact(cmd) {
                Err(HidError::Asleep) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(120)),
                r => return r,
            }
        }
    }

    fn mouse(&mut self, cmd: &[u8]) -> Result<Vec<u8>> {
        let mut c = vec![REPORT_ID, TO_MOUSE];
        c.extend_from_slice(cmd);
        self.request(&c, Duration::from_secs(8))
    }

    /// Canal atual, sem esperar o mouse acordar: Err(Asleep) quando o mouse
    /// está dormindo ou conectado a outro receptor.
    pub fn channel_now(&mut self) -> Result<u8> {
        let mut c = vec![REPORT_ID, TO_MOUSE, MEM_READ, 8];
        c.extend_from_slice(&ADDR_CHANNEL.to_le_bytes());
        let r = self.request(&c, Duration::from_millis(400))?;
        r.get(4).copied().ok_or(HidError::Rejected(0xFF))
    }

    /// Lê `len` bytes da memória do mouse (leituras curtas podem vir vazias,
    /// então lemos sempre pelo menos 8).
    pub fn read_mem(&mut self, addr: u32, len: usize) -> Result<Vec<u8>> {
        let ask = len.clamp(8, 0x18);
        let mut c = vec![MEM_READ, ask as u8];
        c.extend_from_slice(&addr.to_le_bytes());
        let r = self.mouse(&c)?;
        if r.len() < 4 + len {
            return Err(HidError::Rejected(0xFF));
        }
        Ok(r[4..4 + len].to_vec())
    }

    pub fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<()> {
        let mut c = vec![MEM_WRITE, data.len() as u8];
        c.extend_from_slice(&addr.to_le_bytes());
        c.extend_from_slice(data);
        self.mouse(&c).map(|_| ())
    }

    /// Grava em blocos de 24 bytes (o máximo de um pacote).
    pub fn write_mem_long(&mut self, addr: u32, data: &[u8]) -> Result<()> {
        for (i, chunk) in data.chunks(0x18).enumerate() {
            self.write_mem(addr + (i * 0x18) as u32, chunk)?;
            // A área de macros é flash: a escrita leva alguns ms para valer.
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    pub fn read_mem_long(&mut self, addr: u32, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let n = (len - out.len()).min(0x18);
            out.extend(self.read_mem(addr + out.len() as u32, n.max(8))?.into_iter().take(n));
        }
        Ok(out)
    }

    /// Macro gravada no espaço `id`, se houver.
    pub fn read_macro(&mut self, id: usize) -> Result<Option<Vec<u8>>> {
        let addr = crate::macros::slot_addr(id);
        let head = self.read_mem(addr, 8)?;
        let len = u16::from_le_bytes([head[0], head[1]]) as usize;
        if !(4..=crate::macros::SLOT_SIZE as usize).contains(&len) {
            return Ok(None);
        }
        Ok(Some(self.read_mem_long(addr, len)?))
    }

    pub fn write_macro(&mut self, id: usize, data: &[u8]) -> Result<()> {
        self.write_mem_long(crate::macros::slot_addr(id), data)
    }

    /// Como o A HUB: zera o cabeçalho (comprimento 0 = vazio).
    pub fn delete_macro(&mut self, id: usize) -> Result<()> {
        self.write_mem(crate::macros::slot_addr(id), &[0, 0, 0, 0])
    }

    pub fn read_dongle_mem(&mut self, addr: u16, len: usize) -> Result<Vec<u8>> {
        let a = addr.to_le_bytes();
        let r = self.request(
            &[REPORT_ID, TO_DONGLE, MEM_READ, len as u8, a[0], a[1], 0, 0],
            Duration::from_secs(1),
        )?;
        Ok(r.get(4..4 + len).ok_or(HidError::Rejected(0xFF))?.to_vec())
    }

    // ------------------------------------------------------------ funções

    pub fn battery(&mut self) -> Result<u8> {
        let r = self.mouse(&[0xAA])?;
        r.get(2).copied().ok_or(HidError::Rejected(0xFF))
    }

    pub fn channel(&mut self) -> Result<u8> {
        Ok(self.read_mem(ADDR_CHANNEL, 8)?[0])
    }

    /// IDs RF dos receptores pareados, um por canal (None = vazio).
    pub fn paired(&mut self) -> Result<Vec<Option<[u8; 4]>>> {
        let d = self.read_mem(ADDR_PAIRED, 0x10)?;
        Ok(d.chunks(4)
            .map(|c| <[u8; 4]>::try_from(c).ok().filter(|id| id != &[0xFF; 4]))
            .collect())
    }

    /// ID RF deste receptor.
    pub fn own_id(&mut self) -> Result<[u8; 4]> {
        let d = self.read_dongle_mem(0x20, 8)?;
        Ok([d[0], d[1], d[2], d[3]])
    }

    /// Troca o mouse para outro canal (outro computador). Depois disto o
    /// receptor local deixa de alcançar o mouse.
    pub fn switch_channel(&mut self, idx: u8) -> Result<()> {
        self.mouse(&[0xAE, 0, 0, 0, 0, idx]).map(|_| ())
    }

    pub fn dpi(&mut self) -> Result<Dpi> {
        let d = self.read_mem(ADDR_DPI_X, 0x18)?;
        let levels = (0..DPI_LEVELS)
            .map(|i| u16::from_le_bytes([d[2 * i], d[2 * i + 1]]) as u32 * 50)
            .collect();
        Ok(Dpi { levels, count: d[14] as usize + 1, current: d[16] as usize })
    }

    /// Nível atual (0-based) nos blocos X e Y.
    pub fn set_dpi_level(&mut self, level: usize) -> Result<()> {
        self.write_mem(ADDR_DPI_X + 0x10, &[level as u8])?;
        self.write_mem(ADDR_DPI_Y + 0x10, &[level as u8])
    }

    /// Valores dos 7 níveis, iguais para X e Y (múltiplos de 50).
    pub fn set_dpi_values(&mut self, levels: &[u32]) -> Result<()> {
        let mut data = Vec::with_capacity(DPI_LEVELS * 2);
        for v in levels.iter().take(DPI_LEVELS) {
            data.extend_from_slice(&((v.clamp(&50, &4000) / 50) as u16).to_le_bytes());
        }
        self.write_mem(ADDR_DPI_X, &data)?;
        self.write_mem(ADDR_DPI_Y, &data)
    }

    /// (com fio, sem fio) em Hz.
    pub fn polling(&mut self) -> Result<(u32, u32)> {
        let d = self.read_mem(ADDR_POLLING, 8)?;
        let hz = |c: u8| if c == 0 { 0 } else { 1000 / c as u32 };
        Ok((hz(d[0]), hz(d[1])))
    }

    pub fn set_polling(&mut self, wireless: bool, hz: u32) -> Result<()> {
        let code = (1000 / hz.clamp(125, 1000)) as u8;
        self.write_mem(ADDR_POLLING + wireless as u32, &[code])
    }

    pub fn keys(&mut self) -> Result<Vec<[u8; 4]>> {
        let mut d = self.read_mem(ADDR_KEYS, 0x18)?;
        d.extend(self.read_mem(ADDR_KEYS + 0x18, 0x18)?);
        Ok(d.chunks(4).take(KEY_COUNT).map(|c| [c[0], c[1], c[2], c[3]]).collect())
    }

    pub fn set_key(&mut self, key: usize, code: [u8; 4]) -> Result<()> {
        self.write_mem(ADDR_KEYS + 4 * key as u32, &code)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Dpi {
    pub levels: Vec<u32>,
    pub count: usize,
    pub current: usize,
}

/// Interface de movimento do receptor (sem report ID):
/// [botões, x i16, y i16, roda, roda lateral].
pub fn open_motion() -> std::io::Result<File> {
    let path = find_hidraw(RECEIVER_PID, |d| d.starts_with(&[0x05, 0x01, 0x09, 0x02]))
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "interface de movimento"))?;
    File::open(path)
}
