//! Thread que fala com o receptor, para a interface nunca travar.
use eframe::egui;

use crate::hid::{self, Dpi, HidError, Receiver};
use std::sync::mpsc::{self, Receiver as Rx, Sender};
use std::thread;
use std::time::Duration;

pub enum Request {
    Refresh,
    Battery,
    SetKey(usize, [u8; 4]),
    ResetKeys,
    SetDpiLevel(usize),
    SetDpiValues(Vec<u32>),
    SetPolling { wireless: bool, hz: u32 },
    Switch(u8),
    ReadMacros,
    SetMacro(usize, Vec<u8>),
    DeleteMacro(usize),
}

#[derive(Default, Clone)]
pub struct State {
    pub connected: bool,
    pub battery: Option<u8>,
    pub wired: bool,
    pub channel: Option<u8>,
    pub paired: Vec<Option<[u8; 4]>>,
    pub own_id: Option<[u8; 4]>,
    pub dpi: Option<Dpi>,
    pub polling: Option<(u32, u32)>,
    pub keys: Option<Vec<[u8; 4]>>,
    /// Conteúdo bruto de cada espaço de macro (None = vazio); None = não lido.
    pub macros: Option<Vec<Option<Vec<u8>>>>,
}

pub enum Event {
    State(State),
    Busy(bool),
    Message(String),
    Switched(u8),
    Error(String),
}

pub struct Worker {
    pub tx: Sender<Request>,
    pub rx: Rx<Event>,
}

pub fn spawn(ctx: egui::Context) -> Worker {
    let (tx, req_rx) = mpsc::channel::<Request>();
    let (ev_tx, rx) = mpsc::channel::<Event>();
    thread::spawn(move || {
        let mut state = State::default();
        let send = |e: Event| {
            let _ = ev_tx.send(e);
            ctx.request_repaint();
        };
        while let Ok(req) = req_rx.recv() {
            send(Event::Busy(true));
            let res = handle(&req, &mut state);
            if let (Request::Switch(ch), Ok(_)) = (&req, &res) {
                send(Event::Switched(*ch));
            }
            state.wired = hid::wired_connected();
            send(Event::State(state.clone()));
            match res {
                Ok(Some(msg)) => send(Event::Message(msg)),
                Ok(None) => {}
                Err(e) => send(Event::Error(e.to_string())),
            }
            send(Event::Busy(false));
        }
    });
    Worker { tx, rx }
}

fn handle(req: &Request, st: &mut State) -> Result<Option<String>, HidError> {
    let mut r = match Receiver::open() {
        Ok(r) => r,
        Err(e) => {
            *st = State::default();
            return Err(e);
        }
    };
    st.connected = true;
    st.own_id = r.own_id().ok();
    match req {
        Request::Refresh => {
            // Se o mouse estiver em outro canal, só o receptor responde.
            st.battery = Some(r.battery()?);
            st.channel = Some(r.channel()?);
            st.paired = r.paired()?;
            st.dpi = Some(r.dpi()?);
            st.polling = Some(r.polling()?);
            st.keys = Some(r.keys()?);
            Ok(None)
        }
        Request::Battery => {
            st.battery = Some(r.battery()?);
            st.channel = Some(r.channel()?);
            Ok(None)
        }
        Request::SetKey(k, code) => {
            r.set_key(*k, *code)?;
            st.keys = Some(r.keys()?);
            Ok(Some("Botão gravado no mouse".into()))
        }
        Request::ResetKeys => {
            for (k, code) in crate::keys::FACTORY.iter().enumerate() {
                r.set_key(k, *code)?;
            }
            st.keys = Some(r.keys()?);
            Ok(Some("Botões restaurados para o padrão de fábrica".into()))
        }
        Request::SetDpiLevel(l) => {
            r.set_dpi_level(*l)?;
            st.dpi = Some(r.dpi()?);
            Ok(None)
        }
        Request::SetDpiValues(v) => {
            r.set_dpi_values(v)?;
            st.dpi = Some(r.dpi()?);
            Ok(Some("Níveis de DPI gravados".into()))
        }
        Request::SetPolling { wireless, hz } => {
            r.set_polling(*wireless, *hz)?;
            st.polling = Some(r.polling()?);
            Ok(None)
        }
        Request::ReadMacros => {
            let mut list = Vec::with_capacity(crate::macros::MAX_MACROS);
            for id in 0..crate::macros::MAX_MACROS {
                list.push(r.read_macro(id)?);
            }
            st.macros = Some(list);
            Ok(None)
        }
        Request::SetMacro(id, data) => {
            r.write_macro(*id, data)?;
            // Logo após gravar, a leitura ainda pode trazer o conteúdo antigo.
            let mut back = None;
            for _ in 0..5 {
                thread::sleep(Duration::from_millis(150));
                back = r.read_macro(*id)?;
                if back.as_deref() == Some(&data[..]) {
                    break;
                }
            }
            if back.as_deref() != Some(&data[..]) {
                return Err(HidError::Rejected(0xFE));
            }
            if let Some(m) = st.macros.as_mut() {
                m[*id] = back;
            }
            Ok(Some(format!("Macro {} gravada no mouse", id + 1)))
        }
        Request::DeleteMacro(id) => {
            r.delete_macro(*id)?;
            thread::sleep(Duration::from_millis(150));
            if let Some(m) = st.macros.as_mut() {
                m[*id] = None;
            }
            Ok(Some(format!("Macro {} apagada", id + 1)))
        }
        Request::Switch(ch) => {
            r.switch_channel(*ch)?;
            thread::sleep(Duration::from_millis(200));
            st.channel = Some(*ch);
            Ok(None)
        }
    }
}
