//! Pacotes TCP do JQNetwork 0.6.6 (biblioteca usada pelo A HUB).
//!
//! Cabeçalho de 24 bytes, little-endian, `#pragma pack(1)`:
//!   i8 bootFlag (0x7d), i8 packageFlag, i32 randomFlag,
//!   i8 metaDataFlag, i32 metaTotal, i32 metaCurrent,
//!   i8 payloadFlag, i32 payloadTotal, i32 payloadCurrent
//! Metadado = JSON {"targetActionFlag": ação, "appendData": ...}.
//! Payload de mapa = JSON compacto; com flag 2 vem em `qCompress`
//! (u32 big-endian com o tamanho original + zlib).

use serde_json::{Value, json};
use std::io::{self, Read, Write};

const BOOT: u8 = 0x7d;
pub const PAYLOAD_TRANSPORT: u8 = 1;
const UNCOMPRESSED: u8 = 1;
const COMPRESSED: u8 = 2;
const HEAD: usize = 24;

#[derive(Debug, Clone)]
pub struct Package {
    pub flag: u8,
    pub random: i32,
    pub meta: Vec<u8>,
    pub payload: Vec<u8>,
    /// Pacote cortado em partes (o resto viria por pedidos): não suportado.
    pub partial: bool,
}

impl Package {
    pub fn new(action: &str, payload: &Value, random: i32) -> Package {
        let meta = json!({ "appendData": {}, "targetActionFlag": action });
        Package {
            flag: PAYLOAD_TRANSPORT,
            random,
            meta: serde_json::to_vec(&meta).unwrap_or_default(),
            payload: serde_json::to_vec(payload).unwrap_or_default(),
            partial: false,
        }
    }

    pub fn action(&self) -> String {
        serde_json::from_slice::<Value>(&self.meta)
            .ok()
            .and_then(|m| m.get("targetActionFlag").and_then(|a| a.as_str()).map(str::to_string))
            .unwrap_or_default()
    }

    pub fn payload_json(&self) -> Option<Value> {
        serde_json::from_slice(&self.payload).ok()
    }

    pub fn encode(&self) -> Vec<u8> {
        let size = |v: &Vec<u8>| if v.is_empty() { -1i32 } else { v.len() as i32 };
        let mut out = Vec::with_capacity(HEAD + self.meta.len() + self.payload.len());
        out.push(BOOT);
        out.push(self.flag);
        out.extend(self.random.to_le_bytes());
        out.push(UNCOMPRESSED);
        out.extend(size(&self.meta).to_le_bytes());
        out.extend(size(&self.meta).to_le_bytes());
        out.push(UNCOMPRESSED);
        out.extend(size(&self.payload).to_le_bytes());
        out.extend(size(&self.payload).to_le_bytes());
        out.extend(&self.meta);
        out.extend(&self.payload);
        out
    }

    pub fn read_from(r: &mut impl Read) -> io::Result<Package> {
        let mut h = [0u8; HEAD];
        r.read_exact(&mut h)?;
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        if h[0] != BOOT {
            return Err(bad("bootFlag inválido"));
        }
        let i32_at = |o: usize| i32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let (meta_flag, meta_total, meta_cur) = (h[6], i32_at(7), i32_at(11));
        let (pay_flag, pay_total, pay_cur) = (h[15], i32_at(16), i32_at(20));
        let read_n = |r: &mut dyn Read, n: i32| -> io::Result<Vec<u8>> {
            if n <= 0 {
                return Ok(Vec::new());
            }
            if n > 64 * 1024 * 1024 {
                return Err(bad("pacote grande demais"));
            }
            let mut v = vec![0u8; n as usize];
            r.read_exact(&mut v)?;
            Ok(v)
        };
        let mut meta = read_n(r, meta_cur)?;
        let mut payload = read_n(r, pay_cur)?;
        if meta_flag == COMPRESSED {
            meta = q_uncompress(&meta)?;
        }
        if pay_flag == COMPRESSED {
            payload = q_uncompress(&payload)?;
        }
        Ok(Package {
            flag: h[1],
            random: i32_at(2),
            meta,
            payload,
            partial: meta_total > meta_cur.max(0) || (pay_total > pay_cur.max(0) && pay_flag == UNCOMPRESSED),
        })
    }

    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.encode())?;
        w.flush()
    }
}

fn q_uncompress(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.len() < 4 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(&data[4..]).read_to_end(&mut out)?;
    Ok(out)
}

pub fn random_flag() -> i32 {
    let mut b = [0u8; 4];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    (i32::from_le_bytes(b) & 0x3FFF_FFFF).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = Package::new("SetClipboardToText", &json!({ "text": "olá" }), 1234);
        let bytes = p.encode();
        assert_eq!(bytes[0], 0x7d);
        assert_eq!(&bytes[2..6], &1234i32.to_le_bytes());
        let q = Package::read_from(&mut &bytes[..]).unwrap();
        assert_eq!(q.action(), "SetClipboardToText");
        assert_eq!(q.payload_json().unwrap()["text"], "olá");
        assert!(!q.partial);
    }

    #[test]
    fn compressed_payload() {
        // qCompress: tamanho original em u32 big-endian + zlib
        let raw = br#"{"text":"comprimido"}"#;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(raw).unwrap();
        let mut payload = (raw.len() as u32).to_be_bytes().to_vec();
        payload.extend(z.finish().unwrap());
        let mut p = Package::new("SetClipboardToText", &json!({}), 9);
        p.payload = payload;
        let mut bytes = p.encode();
        bytes[15] = COMPRESSED;
        let q = Package::read_from(&mut &bytes[..]).unwrap();
        assert_eq!(q.payload_json().unwrap()["text"], "comprimido");
    }
}
