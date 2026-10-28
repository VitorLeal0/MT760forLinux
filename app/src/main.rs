//! MT760forLinux — configurador para Linux do mouse Rapoo MT760L.
//! Projeto independente, sem ligação com a Rapoo. Licença GPL-3.0-or-later.

mod config;
mod cross;
mod hid;
mod jqnet;
mod keys;
mod lan;
mod macros;
mod worker;

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Vec2};
use std::time::{Duration, Instant};
use worker::{Event, Request, State};

// Paleta clara
const BG: Color32 = Color32::from_rgb(0xF4, 0xF6, 0xF7);
const ACCENT: Color32 = Color32::from_rgb(0x20, 0x5E, 0xFF);
const SOFT: Color32 = Color32::from_rgb(0xEB, 0xF2, 0xFF);
const THEME: Color32 = Color32::from_rgb(0x97, 0xB9, 0xFF);
const TEXT: Color32 = Color32::from_rgb(0x1E, 0x20, 0x24);
const MUTED: Color32 = Color32::from_rgb(0x94, 0x9E, 0xA6);
const ERROR: Color32 = Color32::from_rgb(0xED, 0x20, 0x20);

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Keys,
    Macros,
    Dpi,
    Performance,
    Cross,
    Lan,
    About,
}

struct Calibration {
    dev: std::fs::File,
    acc: f64,
    start: Option<f64>,
}

struct App {
    worker: worker::Worker,
    state: State,
    busy: bool,
    toast: Option<(String, bool, Instant)>,
    page: Page,
    cfg: config::Config,
    picking: Option<usize>,
    dpi_edit: Vec<u32>,
    last_battery: Instant,
    calib: Option<Calibration>,
    lan: Option<lan::Status>,
    macro_sel: usize,
    macros_requested: bool,
    macro_edit: Option<macros::Macro>,
    /// Gravação pelo teclado: instante do último evento e modificadores.
    recording: Option<(Instant, egui::Modifiers)>,
    add_key: u8,
    add_mouse: u8,
    add_delay: u32,
    last_lan_read: Instant,
    /// MT760_SCREENSHOT=arquivo.png[:pagina] (desenvolvimento): salva a janela e sai.
    shot: Option<(String, Instant, bool)>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        style(&cc.egui_ctx);
        let worker = worker::spawn(cc.egui_ctx.clone());
        let _ = worker.tx.send(Request::Refresh);
        App {
            worker,
            state: State::default(),
            busy: true,
            toast: None,
            page: Page::Keys,
            cfg: config::load(),
            picking: None,
            dpi_edit: Vec::new(),
            last_battery: Instant::now(),
            calib: None,
            lan: None,
            macro_sel: 0,
            macros_requested: false,
            macro_edit: None,
            recording: None,
            add_key: 0x04,
            add_mouse: 0xF0,
            add_delay: 50,
            last_lan_read: Instant::now() - Duration::from_secs(10),
            shot: std::env::var("MT760_SCREENSHOT").ok().map(|v| (v, Instant::now(), false)),
        }
    }

    fn send(&self, r: Request) {
        let _ = self.worker.tx.send(r);
    }

    fn save_cfg(&mut self) {
        if let Err(e) = config::save(&self.cfg) {
            self.toast = Some((format!("Erro ao salvar configuração: {e}"), true, Instant::now()));
        }
    }

    fn poll(&mut self) {
        while let Ok(ev) = self.worker.rx.try_recv() {
            match ev {
                Event::State(s) => {
                    if let Some(d) = &s.dpi {
                        if self.dpi_edit.len() != d.levels.len() {
                            self.dpi_edit = d.levels.clone();
                        }
                    }
                    self.state = s;
                }
                Event::Busy(b) => self.busy = b,
                Event::Message(m) => self.toast = Some((m, false, Instant::now())),
                Event::Switched(ch) => {
                    let msg = format!("Mouse enviado para {}", self.cfg.channel_label(ch));
                    self.toast = Some((msg, false, Instant::now()));
                }
                Event::Error(e) => self.toast = Some((e, true, Instant::now())),
            }
        }
        if self.last_lan_read.elapsed() > Duration::from_secs(2) {
            self.last_lan_read = Instant::now();
            self.lan = lan::read_status();
            // Canais sem nome recebem o nome do computador descoberto na rede.
            let mut named = false;
            if let Some(st) = &self.lan {
                for p in &st.peers {
                    if let Some(c) = p.own_channel {
                        if self.cfg.channel_name(c).is_none() && !p.host.is_empty() {
                            self.cfg.set_channel_name(c, &p.host);
                            named = true;
                        }
                    }
                }
            }
            if named {
                self.save_cfg();
            }
        }
        if self.last_battery.elapsed() > Duration::from_secs(60) && !self.busy {
            self.last_battery = Instant::now();
            self.send(Request::Battery);
        }
    }
}

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.visuals.panel_fill = BG;
        s.visuals.window_fill = Color32::WHITE;
        s.visuals.override_text_color = Some(TEXT);
        s.visuals.selection.bg_fill = ACCENT;
        s.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
        s.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
        s.visuals.widgets.active.corner_radius = CornerRadius::same(8);
        s.visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
        s.visuals.widgets.hovered.weak_bg_fill = SOFT;
        s.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, THEME);
        // Campos de texto com fundo e borda visíveis sobre os cartões brancos
        s.visuals.extreme_bg_color = BG;
        s.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xDD, 0xE2, 0xE8));
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.item_spacing = Vec2::new(10.0, 8.0);
    });
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
}

fn mouse_image(color: u8) -> egui::ImageSource<'static> {
    match color {
        1 => egui::include_image!("../assets/mouse_gray.svg"),
        3 => egui::include_image!("../assets/mouse_pink.svg"),
        _ => egui::include_image!("../assets/mouse_black.svg"),
    }
}

fn nav_icon(page: Page, on: bool) -> egui::ImageSource<'static> {
    match (page, on) {
        (Page::Keys, false) => egui::include_image!("../assets/nav/keys.svg"),
        (Page::Keys, true) => egui::include_image!("../assets/nav/keys_on.svg"),
        (Page::Macros, false) => egui::include_image!("../assets/nav/macro.svg"),
        (Page::Macros, true) => egui::include_image!("../assets/nav/macro_on.svg"),
        (Page::Dpi, false) => egui::include_image!("../assets/nav/dpi.svg"),
        (Page::Dpi, true) => egui::include_image!("../assets/nav/dpi_on.svg"),
        (Page::Performance, false) => egui::include_image!("../assets/nav/perf.svg"),
        (Page::Performance, true) => egui::include_image!("../assets/nav/perf_on.svg"),
        (Page::Cross, false) => egui::include_image!("../assets/nav/cross.svg"),
        (Page::Cross, true) => egui::include_image!("../assets/nav/cross_on.svg"),
        (Page::Lan, false) => egui::include_image!("../assets/nav/lan.svg"),
        (Page::Lan, true) => egui::include_image!("../assets/nav/lan_on.svg"),
        (Page::About, false) => egui::include_image!("../assets/nav/about.svg"),
        (Page::About, true) => egui::include_image!("../assets/nav/about_on.svg"),
    }
}

fn hex_id(id: &[u8; 4]) -> String {
    id.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        ui.ctx().request_repaint_after(Duration::from_millis(500));
        self.screenshot(ui.ctx());

        egui::Panel::top("top")
            .exact_size(64.0)
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(20, 10)))
            .show(ui, |ui| self.top_bar(ui));

        egui::Panel::left("nav")
            .exact_size(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(Margin::symmetric(12, 18)))
            .show(ui, |ui| self.nav(ui));

        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(22)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                    Page::Keys => self.page_keys(ui),
                    Page::Macros => self.page_macros(ui),
                    Page::Dpi => self.page_dpi(ui),
                    Page::Performance => self.page_performance(ui),
                    Page::Cross => self.page_cross(ui),
                    Page::Lan => self.page_lan(ui),
                    Page::About => self.page_about(ui),
                });
            });

        self.key_picker(ui.ctx());
        self.toast(ui.ctx());
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.add(egui::Image::new(egui::include_image!("../assets/logo.svg")).fit_to_exact_size(Vec2::new(36.0, 36.0)));
            ui.label(RichText::new("MT760forLinux").size(20.0).strong().color(ACCENT));
            ui.add_space(18.0);
            ui.label(RichText::new("Rapoo MT760L").size(17.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.busy, egui::Button::new("Atualizar")).clicked() {
                    self.send(Request::Refresh);
                }
                if self.busy {
                    ui.spinner();
                }
                let st = &self.state;
                if !st.connected {
                    ui.label(RichText::new("Receptor não encontrado").color(ERROR));
                    return;
                }
                if let Some(b) = st.battery {
                    let txt = if st.wired { format!("Bateria {b}% · carregando") } else { format!("Bateria {b}%") };
                    let color = if b <= 15 && !st.wired { ERROR } else { TEXT };
                    ui.label(RichText::new(txt).size(15.0).color(color));
                }
                if let Some(ch) = st.channel {
                    ui.label(RichText::new(self.cfg.channel_label(ch)).color(MUTED));
                }
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        let items = [
            (Page::Keys, "Botões"),
            (Page::Macros, "Macros"),
            (Page::Dpi, "DPI"),
            (Page::Performance, "Desempenho"),
            (Page::Cross, "Cross-screen"),
            (Page::Lan, "Modo LAN"),
            (Page::About, "Sobre"),
        ];
        for (page, label) in items {
            let on = self.page == page;
            let img = egui::Image::new(nav_icon(page, on)).fit_to_exact_size(Vec2::splat(22.0));
            let text = RichText::new(label).size(15.0).color(if on { ACCENT } else { TEXT });
            let btn = egui::Button::image_and_text(img, text)
                .fill(if on { SOFT } else { Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .min_size(Vec2::new(166.0, 44.0));
            if ui.add(btn).clicked() {
                self.page = page;
            }
        }
    }

    fn needs_mouse(&self, ui: &mut egui::Ui) -> bool {
        if self.state.keys.is_some() {
            return true;
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Não foi possível ler o mouse").size(17.0).strong());
            ui.label("Mexa o mouse para acordá-lo e clique em Atualizar. Se ele estiver em outro computador, traga-o de volta pelo botão de canal.");
        });
        false
    }

    // ------------------------------------------------------------ Botões
    fn page_keys(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Personalização de botões").size(20.0).strong());
        ui.label(RichText::new("As funções são gravadas no próprio mouse e valem em qualquer computador.").color(MUTED));
        ui.add_space(8.0);
        if !self.needs_mouse(ui) {
            return;
        }
        let keys = self.state.keys.clone().unwrap_or_default();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let total = ui.available_width();
            let img_h = 470.0;
            let img_w = img_h * 532.0 / 794.0;
            let col_w = ((total - img_w) / 2.0 - 20.0).max(170.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(total, img_h + 20.0), egui::Sense::hover());
            let img_rect = egui::Rect::from_center_size(rect.center(), Vec2::new(img_w, img_h));
            egui::Image::new(mouse_image(self.cfg.color)).paint_at(ui, img_rect);
            // Rótulos em duas colunas, na ordem vertical dos pontos sobre a foto.
            for right in [false, true] {
                let mut col: Vec<usize> = (0..keys::PHYS_KEYS.len()).filter(|&i| keys::PHYS_KEYS[i].right == right).collect();
                col.sort_by(|a, b| keys::PHYS_KEYS[*a].tip.1.total_cmp(&keys::PHYS_KEYS[*b].tip.1));
                let step = (img_h / col.len() as f32).min(70.0);
                let top = img_rect.center().y - step * col.len() as f32 / 2.0;
                for (row, &i) in col.iter().enumerate() {
                    let pk = &keys::PHYS_KEYS[i];
                    let y = top + step * (row as f32 + 0.5);
                    let x = if right { img_rect.right() + 20.0 } else { img_rect.left() - 20.0 - col_w };
                    let r = egui::Rect::from_min_size(egui::pos2(x, y - 24.0), Vec2::new(col_w, 48.0));
                    let label = keys.get(i).map(|c| self.describe_key(*c)).unwrap_or_default();
                    let changed = keys.get(i) != Some(&keys::FACTORY[i]);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
                    let mut job = egui::text::LayoutJob::default();
                    job.append(&format!("{}\n", pk.name), 0.0, egui::TextFormat { color: MUTED, font_id: egui::FontId::proportional(12.0), ..Default::default() });
                    job.append(&label, 0.0, egui::TextFormat { color: if changed { ACCENT } else { TEXT }, font_id: egui::FontId::proportional(15.0), ..Default::default() });
                    let hovered = self.picking == Some(i);
                    let btn = egui::Button::new(job)
                        .fill(if hovered { SOFT } else { BG })
                        .stroke(Stroke::new(1.0, if hovered { ACCENT } else { Color32::TRANSPARENT }))
                        .min_size(Vec2::new(col_w, 48.0));
                    let resp = child.add_enabled(!self.busy, btn);
                    if resp.clicked() {
                        self.picking = Some(i);
                    }
                    let tip = egui::pos2(img_rect.left() + pk.tip.0 * img_w, img_rect.top() + pk.tip.1 * img_h);
                    let anchor = if right { r.left_center() } else { r.right_center() };
                    let active = resp.hovered() || hovered;
                    let color = if active { ACCENT } else { THEME };
                    ui.painter().line_segment([anchor, tip], Stroke::new(if active { 2.0 } else { 1.0 }, color));
                    ui.painter().circle_filled(tip, if active { 6.0 } else { 4.0 }, ACCENT);
                    ui.painter().circle_stroke(tip, if active { 9.0 } else { 6.0 }, Stroke::new(1.5, Color32::from_white_alpha(200)));
                }
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Cor do mouse:");
            for (c, name) in [(2u8, "Preto"), (1, "Cinza"), (3, "Rosa")] {
                if ui.selectable_label(self.cfg.color == c, name).clicked() {
                    self.cfg.color = c;
                    self.save_cfg();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.busy, egui::Button::new("Restaurar padrão")).clicked() {
                    self.send(Request::ResetKeys);
                }
            });
        });
    }

    fn key_picker(&mut self, ctx: &egui::Context) {
        let Some(k) = self.picking else { return };
        let mut open = true;
        let mut chosen = None;
        egui::Window::new(format!("Função: {}", keys::PHYS_KEYS[k].name))
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                    if ui.button(format!("Padrão ({})", keys::describe(keys::FACTORY[k]))).clicked() {
                        chosen = Some(keys::FACTORY[k]);
                    }
                    let saved: Vec<(usize, String)> = self.state.macros.as_ref().map(|list| {
                        list.iter().enumerate().filter(|(_, m)| m.is_some()).map(|(id, _)| (id, self.macro_title(id))).collect()
                    }).unwrap_or_default();
                    egui::CollapsingHeader::new(RichText::new("Macros").strong().color(ACCENT)).show(ui, |ui| {
                        if saved.is_empty() {
                            ui.label(RichText::new("Nenhuma macro gravada. Crie uma na página Macros.").color(MUTED));
                        }
                        for (id, title) in &saved {
                            if ui.selectable_label(false, title).clicked() {
                                chosen = Some([0x05, 0, *id as u8, 0]);
                            }
                        }
                    });
                    for cat in keys::CATEGORIES {
                        egui::CollapsingHeader::new(RichText::new(cat.title).strong().color(ACCENT))
                            .default_open(cat.title == "Mouse")
                            .show(ui, |ui| {
                                for a in cat.actions {
                                    if ui.selectable_label(false, a.label).clicked() {
                                        chosen = Some(a.code);
                                    }
                                }
                            });
                    }
                });
            });
        if let Some(code) = chosen {
            self.send(Request::SetKey(k, code));
            self.picking = None;
        } else if !open {
            self.picking = None;
        }
    }

    fn macro_title(&self, id: usize) -> String {
        match self.cfg.macros.get(&id.to_string()).map(|m| m.name.trim()).filter(|n| !n.is_empty()) {
            Some(n) => format!("Macro {} · {n}", id + 1),
            None => format!("Macro {}", id + 1),
        }
    }

    fn describe_key(&self, code: [u8; 4]) -> String {
        if code[0] == 0x05 {
            return self.macro_title(code[2] as usize);
        }
        keys::describe(code)
    }

    fn select_macro(&mut self, id: usize) {
        self.macro_sel = id;
        self.recording = None;
        let device = self.state.macros.as_ref().and_then(|l| l[id].clone());
        let saved = self.cfg.macros.get(&id.to_string()).cloned();
        // O mouse é a fonte da verdade; a configuração guarda o nome e as opções.
        self.macro_edit = Some(match (device, saved) {
            (Some(bytes), Some(s)) if macros::encode(&s) == bytes => s,
            (Some(bytes), s) => match macros::decode(&bytes) {
                Some((play, steps)) => macros::Macro { name: s.map(|s| s.name).unwrap_or_default(), play, steps, ignore_delay: false },
                None => macros::Macro::default(),
            },
            (None, _) => macros::Macro::default(),
        });
    }

    // ------------------------------------------------------------ Macros
    fn page_macros(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Macros").size(20.0).strong());
        ui.label(RichText::new("Sequências de teclas e cliques gravadas no próprio mouse. Depois de salvar, atribua a macro a um botão na página Botões.").color(MUTED));
        ui.add_space(8.0);
        if self.state.macros.is_none() {
            if !self.macros_requested && !self.busy && self.state.connected {
                self.macros_requested = true;
                self.send(Request::ReadMacros);
            }
            if !self.needs_mouse(ui) {
                return;
            }
            if self.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Lendo as macros do mouse…");
                });
            } else if ui.button("Ler as macros do mouse").clicked() {
                self.send(Request::ReadMacros);
            }
            return;
        }
        if self.macro_edit.is_none() {
            self.select_macro(self.macro_sel);
        }
        let used: Vec<bool> = self.state.macros.as_ref().unwrap().iter().map(|m| m.is_some()).collect();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(220.0);
                card().show(ui, |ui| {
                    ui.set_width(184.0);
                    ui.label(RichText::new("Espaços").strong());
                    for id in 0..macros::MAX_MACROS {
                        let on = self.macro_sel == id;
                        let title = if used[id] { self.macro_title(id) } else { format!("{} · vazio", id + 1) };
                        let text = RichText::new(title).color(if on { ACCENT } else if used[id] { TEXT } else { MUTED });
                        let btn = egui::Button::new(text).fill(if on { SOFT } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(Vec2::new(184.0, 28.0));
                        if ui.add(btn).clicked() {
                            self.select_macro(id);
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.macro_editor(ui, used[self.macro_sel]);
                });
            });
        });
    }

    fn macro_editor(&mut self, ui: &mut egui::Ui, on_device: bool) {
        let id = self.macro_sel;
        let Some(mut m) = self.macro_edit.take() else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Macro {}", id + 1)).size(17.0).strong());
            ui.add(egui::TextEdit::singleline(&mut m.name).hint_text("nome").desired_width(220.0).char_limit(32));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Reprodução:");
            let times = if let macros::Play::Times(n) = m.play { n } else { 1 };
            if ui.radio(matches!(m.play, macros::Play::Times(_)), "repetir").clicked() {
                m.play = macros::Play::Times(times);
            }
            if let macros::Play::Times(n) = &mut m.play {
                ui.add(egui::DragValue::new(n).range(1..=999).suffix(" vez(es)"));
            }
            if ui.radio(m.play == macros::Play::Toggle, "o botão liga/desliga").clicked() {
                m.play = macros::Play::Toggle;
            }
            if ui.radio(m.play == macros::Play::WhileHeld, "enquanto pressionado").clicked() {
                m.play = macros::Play::WhileHeld;
            }
        });
        ui.checkbox(&mut m.ignore_delay, "Ignorar os atrasos entre os eventos");
        ui.add_space(8.0);

        // Gravação pelo teclado
        if let Some((last, mods)) = &mut self.recording {
            let (events, now_mods) = ui.input(|i| (i.events.clone(), i.modifiers));
            let push = |m: &mut macros::Macro, step: macros::Step, last: &mut Instant| {
                let ms = last.elapsed().as_millis() as u32;
                if !m.steps.is_empty() {
                    m.steps.push(macros::Step::Delay { ms });
                }
                m.steps.push(step);
                *last = Instant::now();
            };
            for (flag, usage) in [(now_mods.ctrl != mods.ctrl, 0xE0u8), (now_mods.shift != mods.shift, 0xE1), (now_mods.alt != mods.alt, 0xE2)] {
                if flag {
                    let down = match usage { 0xE0 => now_mods.ctrl, 0xE1 => now_mods.shift, _ => now_mods.alt };
                    push(&mut m, macros::Step::Key { usage, down }, last);
                }
            }
            *mods = now_mods;
            for ev in events {
                if macros::event_count(&m) >= macros::MAX_EVENTS {
                    break;
                }
                if let egui::Event::Key { key, pressed, repeat: false, .. } = ev {
                    if let Some(usage) = macros::egui_key_usage(key) {
                        push(&mut m, macros::Step::Key { usage, down: pressed }, last);
                    }
                }
            }
            ui.ctx().request_repaint();
        }
        if self.recording.is_some() && macros::event_count(&m) >= macros::MAX_EVENTS {
            self.recording = None;
            self.toast = Some(("Gravação parada: limite de 14 eventos do mouse".into(), false, Instant::now()));
        }
        ui.horizontal(|ui| {
            if self.recording.is_some() {
                if ui.add(egui::Button::new(RichText::new("Parar gravação").color(Color32::WHITE)).fill(ERROR)).clicked() {
                    self.recording = None;
                }
                ui.label(RichText::new("Gravando: digite no teclado (com esta janela em foco).").color(ERROR));
            } else if ui.button("Gravar pelo teclado").clicked() {
                self.recording = Some((Instant::now(), ui.input(|i| i.modifiers)));
            }
        });
        ui.add_space(6.0);

        // Adicionar eventos manualmente
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("add_key").selected_text(macros::usage_name(self.add_key)).show_ui(ui, |ui| {
                for u in (0x04u8..=0x52).chain(0xE0..=0xE3) {
                    ui.selectable_value(&mut self.add_key, u, macros::usage_name(u));
                }
            });
            if ui.button("+ tecla (pressionar e soltar)").clicked() {
                m.steps.push(macros::Step::Key { usage: self.add_key, down: true });
                m.steps.push(macros::Step::Delay { ms: 30 });
                m.steps.push(macros::Step::Key { usage: self.add_key, down: false });
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("add_mouse").selected_text(macros::mouse_name(self.add_mouse)).show_ui(ui, |ui| {
                for b in 0xF0u8..=0xF4 {
                    ui.selectable_value(&mut self.add_mouse, b, macros::mouse_name(b));
                }
            });
            if ui.button("+ clique").clicked() {
                m.steps.push(macros::Step::Mouse { button: self.add_mouse, down: true });
                m.steps.push(macros::Step::Delay { ms: 30 });
                m.steps.push(macros::Step::Mouse { button: self.add_mouse, down: false });
            }
            ui.add_space(12.0);
            ui.add(egui::DragValue::new(&mut self.add_delay).range(2..=30000).suffix(" ms"));
            if ui.button("+ atraso").clicked() {
                m.steps.push(macros::Step::Delay { ms: self.add_delay });
            }
        });
        ui.add_space(8.0);

        // Lista de eventos
        let count = macros::event_count(&m);
        let over = count > macros::MAX_EVENTS;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Eventos").strong());
            ui.label(RichText::new(format!("{count} de {} (limite do mouse; atrasos não contam)", macros::MAX_EVENTS)).color(if over { ERROR } else { MUTED }));
        });
        if over {
            ui.label(RichText::new("A macro tem eventos demais: o MT760L guarda no máximo 14. Remova alguns antes de salvar.").color(ERROR));
        }
        let mut remove = None;
        let mut swap = None;
        egui::ScrollArea::vertical().id_salt("macro_steps").max_height(260.0).show(ui, |ui| {
            if m.steps.is_empty() {
                ui.label(RichText::new("Nenhum evento. Grave pelo teclado ou adicione acima.").color(MUTED));
            }
            let n = m.steps.len();
            for (i, s) in m.steps.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:>3}.", i + 1)).monospace().color(MUTED));
                    match s {
                        macros::Step::Key { usage, down } => {
                            ui.label(format!("{} {}", if *down { "pressionar" } else { "soltar" }, macros::usage_name(*usage)));
                        }
                        macros::Step::Mouse { button, down } => {
                            ui.label(format!("{} {}", if *down { "pressionar" } else { "soltar" }, macros::mouse_name(*button)));
                        }
                        macros::Step::Delay { ms } => {
                            ui.label("esperar");
                            ui.add(egui::DragValue::new(ms).range(2..=30000).suffix(" ms"));
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("remover").clicked() {
                            remove = Some(i);
                        }
                        if i + 1 < n && ui.small_button("descer").clicked() {
                            swap = Some(i);
                        }
                        if i > 0 && ui.small_button("subir").clicked() {
                            swap = Some(i - 1);
                        }
                    });
                });
            }
        });
        if let Some(i) = remove {
            m.steps.remove(i);
        }
        if let Some(i) = swap {
            m.steps.swap(i, i + 1);
        }
        ui.add_space(8.0);
        let bytes = macros::encode(&m);
        let has_events = bytes.len() > 4;
        ui.horizontal(|ui| {
            let save = egui::Button::new(RichText::new("Salvar no mouse").color(Color32::WHITE)).fill(ACCENT);
            if ui.add_enabled(has_events && !over && !self.busy && self.recording.is_none(), save).clicked() {
                self.cfg.macros.insert(id.to_string(), m.clone());
                self.save_cfg();
                self.send(Request::SetMacro(id, bytes.clone()));
            }
            if ui.add_enabled(on_device && !self.busy, egui::Button::new("Apagar do mouse")).clicked() {
                self.cfg.macros.remove(&id.to_string());
                self.save_cfg();
                self.send(Request::DeleteMacro(id));
                m = macros::Macro::default();
            }
            if ui.add_enabled(!m.steps.is_empty(), egui::Button::new("Limpar eventos")).clicked() {
                m.steps.clear();
            }
            ui.label(RichText::new(format!("{} bytes", bytes.len())).color(MUTED));
        });
        if let Some(bound) = self.state.keys.as_ref() {
            let users: Vec<&str> = bound.iter().enumerate().filter(|(_, c)| **c == [0x05, 0, id as u8, 0]).map(|(k, _)| keys::PHYS_KEYS[k].name).collect();
            if !users.is_empty() {
                ui.label(RichText::new(format!("Atribuída a: {}", users.join(", "))).color(ACCENT));
            }
        }
        self.macro_edit = Some(m);
    }

    // ------------------------------------------------------------ DPI
    fn page_dpi(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("DPI").size(20.0).strong());
        ui.label(RichText::new("O botão DPI do mouse alterna entre estes níveis.").color(MUTED));
        ui.add_space(8.0);
        if !self.needs_mouse(ui) {
            return;
        }
        let Some(dpi) = self.state.dpi.clone() else { return };
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Nível atual").strong());
            ui.horizontal_wrapped(|ui| {
                for i in 0..dpi.count.min(dpi.levels.len()) {
                    let on = dpi.current == i;
                    let text = RichText::new(format!("{}\n{} DPI", i + 1, dpi.levels[i])).size(15.0).color(if on { Color32::WHITE } else { TEXT });
                    let btn = egui::Button::new(text).fill(if on { ACCENT } else { SOFT }).min_size(Vec2::new(96.0, 60.0));
                    if ui.add_enabled(!self.busy, btn).clicked() {
                        self.send(Request::SetDpiLevel(i));
                    }
                }
            });
        });
        ui.add_space(12.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Valores dos níveis").strong());
            ui.label(RichText::new("De 50 a 4000, em passos de 50.").color(MUTED));
            ui.horizontal_wrapped(|ui| {
                for (i, v) in self.dpi_edit.iter_mut().enumerate().take(dpi.count) {
                    ui.vertical(|ui| {
                        ui.label(format!("Nível {}", i + 1));
                        ui.add(egui::DragValue::new(v).range(50..=4000).speed(10.0));
                        *v = (*v / 50).max(1) * 50;
                    });
                }
            });
            ui.add_space(6.0);
            let changed = self.dpi_edit != dpi.levels;
            ui.horizontal(|ui| {
                if ui.add_enabled(changed && !self.busy, egui::Button::new(RichText::new("Aplicar").color(Color32::WHITE)).fill(ACCENT)).clicked() {
                    self.send(Request::SetDpiValues(self.dpi_edit.clone()));
                }
                if ui.add_enabled(changed, egui::Button::new("Desfazer")).clicked() {
                    self.dpi_edit = dpi.levels.clone();
                }
            });
        });
    }

    // ------------------------------------------------------------ Desempenho
    fn page_performance(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Taxa de polling").size(20.0).strong());
        ui.label(RichText::new("Quantas vezes por segundo o mouse envia a posição. Taxas maiores gastam mais bateria.").color(MUTED));
        ui.add_space(8.0);
        if !self.needs_mouse(ui) {
            return;
        }
        let Some((wired, wireless)) = self.state.polling else { return };
        for (title, is_wireless, cur) in [("Sem fio (receptor 2.4G)", true, wireless), ("Com fio (cabo USB)", false, wired)] {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(title).strong());
                ui.horizontal(|ui| {
                    for hz in [125u32, 250, 500, 1000] {
                        let on = cur == hz;
                        let text = RichText::new(format!("{hz} Hz")).size(15.0).color(if on { Color32::WHITE } else { TEXT });
                        let btn = egui::Button::new(text).fill(if on { ACCENT } else { SOFT }).min_size(Vec2::new(110.0, 46.0));
                        if ui.add_enabled(!self.busy, btn).clicked() {
                            self.send(Request::SetPolling { wireless: is_wireless, hz });
                        }
                    }
                });
            });
            ui.add_space(12.0);
        }
    }

    // ------------------------------------------------------------ Cross-screen
    fn page_cross(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Cross-screen").size(20.0).strong());
        ui.label(RichText::new("Modo offline: com um receptor em cada computador, empurre o cursor contra a borda da tela para levar o mouse ao outro computador.").color(MUTED));
        ui.add_space(8.0);
        let st = self.state.clone();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Computadores pareados").strong());
            if st.paired.is_empty() {
                ui.label(RichText::new("Mexa o mouse e clique em Atualizar para ler os canais.").color(MUTED));
            }
            let mut renamed = false;
            for (i, id) in st.paired.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Canal {}", i + 1)).strong());
                    let mut name = self.cfg.channel_name(i as u8).unwrap_or_default().to_string();
                    let own = id.is_some() && st.own_id.as_ref() == id.as_ref();
                    let hint = if own { "nome, ex.: Notebook Linux" } else { "nome, ex.: PC Windows" };
                    let edit = egui::TextEdit::singleline(&mut name).hint_text(hint).desired_width(190.0).char_limit(32);
                    if ui.add(edit).changed() {
                        self.cfg.set_channel_name(i as u8, &name);
                        renamed = true;
                    }
                    match id {
                        Some(id) => {
                            ui.label(RichText::new(hex_id(id)).monospace().color(MUTED));
                            if st.own_id.as_ref() == Some(id) {
                                ui.label(RichText::new("este computador").color(ACCENT));
                            }
                        }
                        None => {
                            ui.label(RichText::new("vazio").color(MUTED));
                        }
                    }
                    if st.channel == Some(i as u8) {
                        ui.label(RichText::new("em uso").strong().color(ACCENT));
                    } else if id.is_some() && ui.add_enabled(!self.busy, egui::Button::new("Enviar mouse para cá")).clicked() {
                        self.send(Request::Switch(i as u8));
                    }
                });
            }
            if renamed {
                self.save_cfg();
            }
        });
        ui.add_space(12.0);
        let mut cross = self.cfg.cross.clone();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(&mut cross.enabled, RichText::new("Ativar cross-screen").strong());
            let n = st.paired.len().max(3);
            let names = &self.cfg;
            for (label, slot) in [("Ao sair pela borda direita", &mut cross.right), ("Ao sair pela borda esquerda", &mut cross.left)] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    egui::ComboBox::from_id_salt(label)
                        .selected_text(slot.map(|c| format!("ir para {}", names.channel_label(c))).unwrap_or("não fazer nada".into()))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(slot, None, "não fazer nada");
                            for c in 0..n as u8 {
                                ui.selectable_value(slot, Some(c), format!("ir para {}", names.channel_label(c)));
                            }
                        });
                });
            }
            ui.horizontal(|ui| {
                ui.label("Força para trocar");
                ui.add(egui::Slider::new(&mut cross.push_counts, 100.0..=1500.0).show_value(false));
                ui.label(RichText::new("(quanto empurrar além da borda)").color(MUTED));
            });
        });
        if cross != self.cfg.cross {
            self.cfg.cross = cross;
            self.save_cfg();
        }
        ui.add_space(12.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Calibração").strong());
            ui.label(RichText::new(format!(
                "Largura atual: {:.0} contagens. Calibre de novo se mudar o DPI ou a resolução da tela.",
                self.cfg.cross.width_counts
            )).color(MUTED));
            self.calibration(ui);
        });
        ui.add_space(12.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Serviço").strong());
            let active = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "mt760-cross.service"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
                .unwrap_or(false);
            if active {
                ui.label(RichText::new("O serviço de cross-screen está rodando.").color(ACCENT));
            } else {
                ui.label(RichText::new("O serviço não está rodando: rode o app/install.sh (ou `mt760 --daemon`).").color(ERROR));
            }
            ui.label(RichText::new("No outro computador, use o A HUB oficial (Windows) para trazer o mouse de volta pela borda.").color(MUTED));
        });
    }

    fn calibration(&mut self, ui: &mut egui::Ui) {
        if let Some(c) = &mut self.calib {
            // Lê o movimento sem bloquear (O_NONBLOCK)
            while let Ok(dx) = cross::read_dx(&mut c.dev) {
                if dx == 0 {
                    break;
                }
                c.acc += dx as f64;
            }
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
        let space = ui.input(|i| i.key_pressed(egui::Key::Space));
        match &self.calib {
            None => {
                ui.label("1. Leve o cursor até a borda ESQUERDA da tela.");
                if ui.button("Começar calibração").clicked() {
                    match open_motion_nonblock() {
                        Ok(dev) => self.calib = Some(Calibration { dev, acc: 0.0, start: None }),
                        Err(e) => self.toast = Some((format!("Não foi possível ler o movimento: {e}"), true, Instant::now())),
                    }
                }
            }
            Some(c) if c.start.is_none() => {
                ui.label(RichText::new("2. Com o cursor na borda esquerda, aperte ESPAÇO.").color(ACCENT));
                if space {
                    let acc = c.acc;
                    self.calib.as_mut().unwrap().start = Some(acc);
                }
            }
            Some(c) => {
                let w = c.acc - c.start.unwrap();
                ui.label(RichText::new(format!("3. Mova o cursor até a borda DIREITA e aperte ESPAÇO.  ({w:.0} contagens)")).color(ACCENT));
                if space {
                    if w > 200.0 {
                        self.cfg.cross.width_counts = w;
                        self.save_cfg();
                        self.toast = Some((format!("Calibrado: {w:.0} contagens"), false, Instant::now()));
                    }
                    self.calib = None;
                }
            }
        }
        if self.calib.is_some() && ui.button("Cancelar").clicked() {
            self.calib = None;
        }
    }

    // ------------------------------------------------------------ Modo LAN
    fn page_lan(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Modo LAN").size(20.0).strong());
        ui.label(RichText::new("Os computadores com A HUB na mesma rede se encontram sozinhos. Funciona junto com o A HUB oficial do Windows.").color(MUTED));
        ui.add_space(8.0);
        let mut lan = self.cfg.lan.clone();
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(&mut lan.enabled, RichText::new("Procurar computadores com A HUB na rede").strong());
            ui.label(RichText::new("Só escuta a rede: mostra os outros computadores, o canal do receptor de cada um e dá nome aos canais. O cross-screen continua no modo offline.").color(MUTED));
            ui.add_space(6.0);
            ui.add_enabled_ui(lan.enabled, |ui| {
                egui::CollapsingHeader::new("Avançado").show(ui, |ui| {
                    ui.checkbox(&mut lan.announce, format!("Anunciar este computador como \"{}\"", lan::hostname()));
                    ui.label(RichText::new("Não recomendado com o Windows: quando este PC aparece como parceiro LAN, o A HUB do Windows deixa de devolver o mouse pela borda. Se isso acontecer, desmarque esta opção, coloque o A HUB do Windows em modo offline e empurre o cursor para a borda.").color(ERROR));
                    ui.add_enabled_ui(lan.announce, |ui| {
                        ui.checkbox(&mut lan.clipboard, "Compartilhar a área de transferência (texto)");
                    });
                    ui.label(RichText::new("A área de transferência só funciona entre computadores com este app: o A HUB do Windows não abre nenhuma porta TCP para isso.").color(MUTED));
                });
            });
        });
        if lan != self.cfg.lan {
            self.cfg.lan = lan;
            self.save_cfg();
        }
        ui.add_space(12.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Computadores na rede").strong());
            let Some(st) = self.lan.clone() else {
                ui.label(RichText::new("O serviço mt760-cross não está rodando: rode o app/install.sh.").color(ERROR));
                return;
            };
            if !self.cfg.lan.enabled {
                ui.label(RichText::new("Ative a opção acima para procurar outros computadores.").color(MUTED));
                return;
            }
            if st.peers.is_empty() {
                ui.label(RichText::new("Procurando… Confira se o outro computador está com o A HUB aberto em modo LAN e se o firewall permite a conexão (veja abaixo).").color(MUTED));
            }
            for p in &st.peers {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&p.host).size(15.0).strong());
                    ui.label(RichText::new(&p.ip).monospace().color(MUTED));
                    match p.own_channel {
                        Some(c) => {
                            ui.label(RichText::new(format!("receptor no canal {}", c + 1)).color(ACCENT));
                            let here = self.state.channel == Some(c);
                            if here {
                                ui.label(RichText::new("o mouse está lá").strong().color(ACCENT));
                            } else if ui.add_enabled(!self.busy, egui::Button::new("Enviar mouse para lá")).clicked() {
                                self.send(Request::Switch(c));
                            }
                        }
                        None => {
                            ui.label(RichText::new("receptor não pareado com este mouse").color(MUTED));
                        }
                    }
                });
            }
            if !st.last_event.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Último evento: {}", st.last_event)).color(MUTED));
            }
            if !st.clipboard_server {
                ui.label(RichText::new(format!("A porta TCP {} está ocupada: a área de transferência não vai receber dados.", lan::MSG_PORT)).color(ERROR));
            }
        });
        ui.add_space(12.0);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Firewall").strong());
            ui.label(RichText::new("Se o UFW estiver ativo, libere as portas do A HUB para a sua rede local:").color(MUTED));
            let net = self.lan.as_ref().and_then(|s| s.self_ips.first().cloned()).map(|ip| {
                let mut o: Vec<&str> = ip.split('.').collect();
                o.truncate(3);
                format!("{}.0/24", o.join("."))
            }).unwrap_or("192.168.0.0/24".into());
            for cmd in [
                format!("sudo ufw allow from {net} to any port {} proto udp", lan::LAN_PORT),
                format!("sudo ufw allow from {net} to any port {} proto tcp", lan::MSG_PORT),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&cmd).monospace());
                    if ui.small_button("Copiar").clicked() {
                        ui.ctx().copy_text(cmd.clone());
                    }
                });
            }
            ui.label(RichText::new("No Windows, permita o A HUB no firewall quando ele pedir.").color(MUTED));
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Mensagens não reconhecidas ficam em {}", lan::capture_path().display())).color(MUTED));
        });
    }

    // ------------------------------------------------------------ Sobre
    fn page_about(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Sobre").size(20.0).strong());
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("MT760forLinux").size(17.0).strong());
            ui.label(format!("Versão {}", env!("CARGO_PKG_VERSION")));
            ui.label("Configurador independente do mouse Rapoo MT760L, compatível com o protocolo usado pelo A HUB oficial. Sem ligação com a Rapoo; Rapoo e A HUB são marcas dos seus donos.");
            ui.label(RichText::new("Licença GPL-3.0-or-later. Sem garantia: o app grava na memória do mouse; use por sua conta e risco.").color(MUTED));
            ui.add_space(8.0);
            let st = &self.state;
            if let Some(id) = &st.own_id {
                ui.label(format!("ID deste receptor: {}", hex_id(id)));
            }
            if let Some(ch) = st.channel {
                ui.label(format!("Canal em uso: {}", self.cfg.channel_label(ch)));
            }
            ui.label(format!("Configuração: {}", config::path().display()));
            ui.add_space(8.0);
            ui.label(RichText::new("Não implementado: imagens/arquivos no modo LAN e macros com movimento ou roda do mouse.").color(MUTED));
        });
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some((msg, err, t)) = &self.toast else { return };
        if t.elapsed() > Duration::from_secs(5) {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(if *err { ERROR } else { TEXT })
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| ui.label(RichText::new(msg.as_str()).color(Color32::WHITE)));
            });
    }
}

impl App {
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some((spec, t0, requested)) = &mut self.shot else { return };
        let (file, page) = spec.split_once(':').unwrap_or((spec.as_str(), "keys"));
        self.page = match page {
            "dpi" => Page::Dpi,
            "macros" => Page::Macros,
            "perf" => Page::Performance,
            "cross" => Page::Cross,
            "about" => Page::About,
            "lan" => Page::Lan,
            "picker" => { self.picking = Some(6); Page::Keys }
            _ => Page::Keys,
        };
        let file = file.to_string();
        if !*requested && t0.elapsed() > Duration::from_secs(6) && !self.busy {
            *requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                let [w, h] = img.size;
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = image::save_buffer(&file, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
                std::process::exit(0);
            })));
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

fn open_motion_nonblock() -> std::io::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let f = hid::open_motion()?;
    unsafe {
        let fl = libc::fcntl(f.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(f.as_raw_fd(), libc::F_SETFL, fl | libc::O_NONBLOCK);
    }
    Ok(f)
}

fn cli(args: &[String]) -> Option<i32> {
    let r = || hid::Receiver::open();
    let cfg = config::load();
    match args.get(1).map(String::as_str) {
        Some("--daemon") => cross::run_daemon(),
        Some("--switch") => {
            let Some(ch) = args.get(2).and_then(|a| cfg.find_channel(a)) else {
                eprintln!("canal desconhecido: use o número (1-3) ou um nome dado na interface");
                return Some(2);
            };
            Some(match r().and_then(|mut r| r.switch_channel(ch)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            })
        }
        Some("--info") => {
            let res = r().and_then(|mut r| {
                println!("Receptor: {}", r.path.display());
                println!("Bateria: {}%", r.battery()?);
                println!("Canal: {}", cfg.channel_label(r.channel()?));
                let d = r.dpi()?;
                println!("DPI: {:?} (atual {})", &d.levels[..d.count], d.levels[d.current]);
                let (wd, wl) = r.polling()?;
                println!("Polling: sem fio {wl} Hz, com fio {wd} Hz");
                for (i, id) in r.paired()?.iter().enumerate() {
                    let id = id.map(|id| hex_id(&id)).unwrap_or("vazio".into());
                    println!("{}: {id}", cfg.channel_label(i as u8));
                }
                Ok(())
            });
            Some(if let Err(e) = res { eprintln!("{e}"); 1 } else { 0 })
        }
        Some("--help") | Some("-h") => {
            println!("mt760            abre a interface\nmt760 --info     mostra bateria, DPI, polling e canais\nmt760 --switch N envia o mouse para o canal N (número ou nome)\nmt760 --daemon   serviço de cross-screen e modo LAN");
            Some(0)
        }
        _ => None,
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("MT760forLinux")
            .with_app_id("io.github.mt760forlinux")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([980.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native("MT760forLinux", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
