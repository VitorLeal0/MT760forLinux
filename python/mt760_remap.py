#!/usr/bin/env python3
"""
MT760forLinux - daemon de remapeamento por software para o Rapoo MT760L.
Projeto independente, sem ligacao com a Rapoo.

Copyright (C) 2026 MT760forLinux contributors
Licenca GPL-3.0-or-later: este programa e software livre; voce pode
redistribui-lo e/ou modifica-lo sob os termos da GNU General Public License,
versao 3 ou posterior. Sem nenhuma garantia.

Reimplementa as funcoes do driver oficial (MT760LDriver.exe 1.0.5 para Windows):
  * remapeamento dos 9 botoes (esquerdo, direito, meio, roda cima/baixo,
    avancar, voltar, roda lateral cima/baixo) com o mesmo catalogo de acoes;
  * ajuste de sensibilidade do ponteiro;
  * leitura de informacoes do dispositivo (bateria, canal, IDs pareados)
    pelo protocolo HID vendor (report 0xBA, comando AE 01).

No Windows o remapeamento e feito em software: um filtro de kernel
(rpusbdriver.sys) intercepta o mouse e o app simula a acao. Aqui fazemos o
equivalente com evdev (EVIOCGRAB) + uinput. Sem dependencias externas.
"""
import argparse
import errno
import fcntl
import glob
import os
import select
import shlex
import struct
import subprocess
import sys
import time

try:
    import tomllib
except ImportError:  # Python < 3.11
    tomllib = None

VENDOR = 0x24AE
KNOWN_PRODUCTS = {0x2030: "MT760L (receptor oficial)", 0x1870: "receptor 2.4G",
                  0x4500: "cabo USB (carregando)"}
WIRED_PIDS = {0x4500}
CONFIG_PATH = os.path.expanduser("~/.config/rapoo-mt760/config.toml")
VIRTUAL_NAME = b"Rapoo MT760 virtual"

# ---------------------------------------------------------------- input codes
EV_SYN, EV_KEY, EV_REL, EV_MSC = 0x00, 0x01, 0x02, 0x04
SYN_REPORT = 0
REL_X, REL_Y, REL_HWHEEL, REL_WHEEL = 0x00, 0x01, 0x06, 0x08
REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES = 0x0B, 0x0C
BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA = 0x110, 0x111, 0x112, 0x113, 0x114
BTN_FORWARD, BTN_BACK, BTN_TASK = 0x115, 0x116, 0x117

KEYS = {
    "esc": 1, "1": 2, "2": 3, "3": 4, "4": 5, "5": 6, "6": 7, "7": 8, "8": 9,
    "9": 10, "0": 11, "minus": 12, "equal": 13, "backspace": 14, "tab": 15,
    "q": 16, "w": 17, "e": 18, "r": 19, "t": 20, "y": 21, "u": 22, "i": 23,
    "o": 24, "p": 25, "leftbrace": 26, "rightbrace": 27, "enter": 28,
    "ctrl": 29, "a": 30, "s": 31, "d": 32, "f": 33, "g": 34, "h": 35, "j": 36,
    "k": 37, "l": 38, "semicolon": 39, "apostrophe": 40, "grave": 41,
    "shift": 42, "backslash": 43, "z": 44, "x": 45, "c": 46, "v": 47, "b": 48,
    "n": 49, "m": 50, "comma": 51, "dot": 52, "slash": 53, "rightshift": 54,
    "alt": 56, "space": 57, "capslock": 58, "f1": 59, "f2": 60, "f3": 61,
    "f4": 62, "f5": 63, "f6": 64, "f7": 65, "f8": 66, "f9": 67, "f10": 68,
    "kpminus": 74, "kpplus": 78, "f11": 87, "f12": 88, "rightctrl": 97,
    "print": 99, "altgr": 100, "home": 102, "up": 103, "pageup": 104,
    "left": 105, "right": 106, "end": 107, "down": 108, "pagedown": 109,
    "insert": 110, "delete": 111, "mute": 113, "volumedown": 114,
    "volumeup": 115, "super": 125, "rightsuper": 126, "menu": 127,
    "calc": 140, "mail": 155, "back": 158, "forward": 159, "nextsong": 163,
    "playpause": 164, "previoussong": 165, "stopcd": 166, "homepage": 172,
    "brightnessdown": 224, "brightnessup": 225, "zoomin": 0x1A2, "zoomout": 0x1A3,
}
KEY_ALIASES = {"control": "ctrl", "win": "super", "meta": "super", "cmd": "super",
               "del": "delete", "return": "enter", "pgup": "pageup",
               "pgdn": "pagedown", "plus": "equal", "printscreen": "print"}

# ------------------------------------------------------------------- ioctls
def _IOC(d, t, nr, size):
    return (d << 30) | (size << 16) | (ord(t) << 8) | nr

UI_DEV_CREATE = _IOC(0, "U", 1, 0)
UI_DEV_DESTROY = _IOC(0, "U", 2, 0)
UI_DEV_SETUP = _IOC(1, "U", 3, 92)
UI_SET_EVBIT = _IOC(1, "U", 100, 4)
UI_SET_KEYBIT = _IOC(1, "U", 101, 4)
UI_SET_RELBIT = _IOC(1, "U", 102, 4)
EVIOCGRAB = _IOC(1, "E", 0x90, 4)
HIDIOCGINPUT = lambda n: _IOC(3, "H", 0x0A, n)

EVENT = struct.Struct("llHHi")

# Nomes dos botoes como no DeviceFunction.ini do driver Windows.
# Cada entrada: (tipo, codigo, sinal) -- sinal so para rodas.
BUTTONS = {
    "left": (EV_KEY, BTN_LEFT, 0),
    "right": (EV_KEY, BTN_RIGHT, 0),
    "middle": (EV_KEY, BTN_MIDDLE, 0),
    "wheel_up": (EV_REL, REL_WHEEL, 1),
    "wheel_down": (EV_REL, REL_WHEEL, -1),
    "forward": (EV_KEY, BTN_EXTRA, 0),
    "back": (EV_KEY, BTN_SIDE, 0),
    "side_wheel_up": (EV_REL, REL_HWHEEL, 1),
    "side_wheel_down": (EV_REL, REL_HWHEEL, -1),
}

# Catalogo de acoes (mesmas categorias do driver oficial).
# "keys:" = combinacao de teclas, "btn:" = clique, "cmd:" = comando shell.
ACTIONS = {
    # Basic function
    "copy": "keys:ctrl+c", "cut": "keys:ctrl+x", "paste": "keys:ctrl+v",
    "select_all": "keys:ctrl+a", "undo": "keys:ctrl+z", "redo": "keys:ctrl+shift+z",
    "print": "keys:ctrl+p", "screenshot": "keys:print", "find": "keys:ctrl+f",
    "new": "keys:ctrl+n", "save": "keys:ctrl+s", "menu": "keys:menu",
    # Media function
    "play_pause": "keys:playpause", "stop": "keys:stopcd",
    "previous_track": "keys:previoussong", "next_track": "keys:nextsong",
    "volume_up": "keys:volumeup", "volume_down": "keys:volumedown",
    "mute": "keys:mute", "brightness_up": "keys:brightnessup",
    "brightness_down": "keys:brightnessdown", "home": "keys:homepage",
    "mail": "keys:mail", "calculator": "keys:calc",
    # Mouse function
    "click": "btn:left", "double_click": "btn:left*2", "triple_click": "btn:left*3",
    "right_click": "btn:right", "right_double_click": "btn:right*2",
    "middle_click": "btn:middle", "browser_forward": "btn:extra",
    "browser_back": "btn:side", "scroll_up": "scroll:1", "scroll_down": "scroll:-1",
    "scroll_left": "hscroll:-1", "scroll_right": "hscroll:1",
    # Windows function (equivalentes nos desktops Linux; GNOME/KDE por padrao)
    "close_window": "keys:alt+f4", "lock": "cmd:loginctl lock-session",
    "show_desktop": "keys:super+d", "start_screen": "keys:super",
    "zoom_in": "keys:ctrl+equal", "zoom_out": "keys:ctrl+minus",
    "maximize": "keys:super+up", "minimize": "keys:super+h",
    "previous_tab": "keys:ctrl+pageup", "next_tab": "keys:ctrl+pagedown",
    "notifications": "keys:super+v", "run": "keys:alt+f2",
    "magnifier": "keys:super+alt+8",
    # Advanced
    "speed_cycle": "speed", "speed_toggle": "toggle", "disabled": "none", "default": "default",
}

DEFAULT_CONFIG = """\
# Configuracao do Rapoo MT760L para Linux.
# Botoes: left right middle wheel_up wheel_down forward back
#         side_wheel_up side_wheel_down
# Valores: um nome de acao (veja `mt760-remap actions`), ou
#   "keys:ctrl+shift+t"   combinacao de teclas
#   "cmd:firefox"         executa um comando / programa
#   "open:~/Documentos"   abre arquivo ou pasta (xdg-open)
#   "sound:~/som.wav"     toca um som
#   "default"             comportamento original do botao

[pointer]
sensitivity = 1.0          # multiplicador do movimento (0.1 a 5.0)
custom_sensitivity = 0.5   # alternada pela acao speed_toggle
speed_steps = [0.5, 0.75, 1.0, 1.5, 2.0]   # usados pela acao speed_cycle

[buttons]
left = "default"
right = "default"
middle = "default"
wheel_up = "default"
wheel_down = "default"
forward = "default"
back = "default"
side_wheel_up = "default"
side_wheel_down = "default"
"""

# --------------------------------------------------------------- helpers
def sysread(path, default=""):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def notify(msg):
    try:
        subprocess.Popen(["notify-send", "-a", "Rapoo MT760", "Rapoo MT760", msg],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except FileNotFoundError:
        pass
    print(msg, flush=True)


def spawn(argv):
    try:
        subprocess.Popen(argv, start_new_session=True, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except OSError as e:
        print(f"[erro] {argv[0]}: {e}", file=sys.stderr)


def load_config(path=CONFIG_PATH):
    if not os.path.exists(path):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(DEFAULT_CONFIG)
    if tomllib is None:
        sys.exit("Python 3.11+ e necessario (tomllib).")
    with open(path, "rb") as f:
        cfg = tomllib.load(f)
    buttons = cfg.get("buttons", {})
    for name in buttons:
        if name not in BUTTONS:
            print(f"[aviso] botao desconhecido no config: {name}", file=sys.stderr)
    return cfg


def parse_keys(spec):
    codes = []
    for part in spec.lower().replace(" ", "").split("+"):
        part = KEY_ALIASES.get(part, part)
        if part not in KEYS:
            raise ValueError(f"tecla desconhecida: {part!r}")
        codes.append(KEYS[part])
    return codes

# ---------------------------------------------------------------- devices
def find_event_devices():
    """Lista [(/dev/input/eventN, nome)] das interfaces de mouse Rapoo (com REL_X).
    Com o cabo de carga ligado o mouse aparece duas vezes (receptor + USB)."""
    out = []
    for ev in sorted(glob.glob("/sys/class/input/event*")):
        dev = os.path.join(ev, "device")
        if sysread(f"{dev}/id/vendor") != f"{VENDOR:04x}":
            continue
        if sysread(f"{dev}/name").encode() == VIRTUAL_NAME:
            continue
        rel = int(sysread(f"{dev}/capabilities/rel", "0").split()[-1], 16)
        if rel & (1 << REL_X):
            out.append(("/dev/input/" + os.path.basename(ev), sysread(f"{dev}/name")))
    return out


def find_event_device():
    devs = find_event_devices()
    return devs[0] if devs else (None, None)


def find_hidraws():
    """Lista [(/dev/hidrawN, pid)] das interfaces vendor (reports 0xBA/0xBB)."""
    out = []
    for h in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        uevent = sysread(f"{h}/device/uevent")
        hid_id = next((l.split("=", 1)[1] for l in uevent.splitlines()
                       if l.startswith("HID_ID=")), "")
        parts = hid_id.split(":")
        if len(parts) != 3 or int(parts[1], 16) != VENDOR:
            continue
        try:
            with open(f"{h}/device/report_descriptor", "rb") as f:
                desc = f.read()
        except OSError:
            continue
        if b"\x85\xba" in desc:
            out.append(("/dev/" + os.path.basename(h), int(parts[2], 16)))
    return out


def find_hidraw():
    devs = find_hidraws()
    return devs[0] if devs else (None, None)


class UInput:
    def __init__(self):
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        for t in (EV_KEY, EV_REL, EV_SYN):
            fcntl.ioctl(self.fd, UI_SET_EVBIT, t)
        for code in list(KEYS.values()) + list(range(BTN_LEFT, BTN_TASK + 1)):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, code)
        for code in (REL_X, REL_Y, REL_HWHEEL, REL_WHEEL, REL_WHEEL_HI_RES,
                     REL_HWHEEL_HI_RES):
            fcntl.ioctl(self.fd, UI_SET_RELBIT, code)
        setup = struct.pack("HHHH80sI", 0x06, VENDOR, 0x7601, 1, VIRTUAL_NAME, 0)
        fcntl.ioctl(self.fd, UI_DEV_SETUP, setup)
        fcntl.ioctl(self.fd, UI_DEV_CREATE)
        time.sleep(0.2)

    def emit(self, etype, code, value, syn=True):
        data = EVENT.pack(0, 0, etype, code, value)
        if syn:
            data += EVENT.pack(0, 0, EV_SYN, SYN_REPORT, 0)
        os.write(self.fd, data)

    def syn(self):
        os.write(self.fd, EVENT.pack(0, 0, EV_SYN, SYN_REPORT, 0))

    def tap_keys(self, codes):
        for c in codes:
            self.emit(EV_KEY, c, 1)
        for c in reversed(codes):
            self.emit(EV_KEY, c, 0)

    def click(self, btn, times=1):
        for i in range(times):
            self.emit(EV_KEY, btn, 1)
            self.emit(EV_KEY, btn, 0)
            if i + 1 < times:
                time.sleep(0.03)

    def scroll(self, code, hires, amount):
        os.write(self.fd, EVENT.pack(0, 0, EV_REL, code, amount)
                 + EVENT.pack(0, 0, EV_REL, hires, amount * 120)
                 + EVENT.pack(0, 0, EV_SYN, SYN_REPORT, 0))

    def close(self):
        try:
            fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        finally:
            os.close(self.fd)

# ------------------------------------------------------------------ daemon
CLICK_BTNS = {"left": BTN_LEFT, "right": BTN_RIGHT, "middle": BTN_MIDDLE,
              "side": BTN_SIDE, "extra": BTN_EXTRA}


class Remapper:
    def __init__(self, cfg, ui):
        self.ui = ui
        self.reload(cfg)

    def reload(self, cfg):
        ptr = cfg.get("pointer", {})
        self.sens = min(5.0, max(0.1, float(ptr.get("sensitivity", 1.0))))
        self.base_sens = self.sens
        self.custom_sens = min(5.0, max(0.1, float(ptr.get("custom_sensitivity", 0.5))))
        self.speed_steps = [float(x) for x in ptr.get("speed_steps",
                                                       [0.5, 0.75, 1.0, 1.5, 2.0])]
        self.map = {}
        for name, value in cfg.get("buttons", {}).items():
            if name in BUTTONS and value not in ("default", "", None):
                self.map[BUTTONS[name]] = self.compile(str(value))
        self.acc = [0.0, 0.0]

    def compile(self, spec):
        spec = ACTIONS.get(spec, spec)
        kind, _, arg = spec.partition(":")
        if kind == "keys":
            codes = parse_keys(arg)
            return lambda: self.ui.tap_keys(codes)
        if kind == "btn":
            name, _, n = arg.partition("*")
            btn = CLICK_BTNS[name]
            return lambda: self.ui.click(btn, int(n or 1))
        if kind == "scroll":
            return lambda: self.ui.scroll(REL_WHEEL, REL_WHEEL_HI_RES, int(arg))
        if kind == "hscroll":
            return lambda: self.ui.scroll(REL_HWHEEL, REL_HWHEEL_HI_RES, int(arg))
        if kind == "cmd":
            argv = shlex.split(arg)
            return lambda: spawn(argv)
        if kind == "open":
            path = os.path.expanduser(arg)
            return lambda: spawn(["xdg-open", path])
        if kind == "sound":
            path = os.path.expanduser(arg)
            return lambda: spawn(["sh", "-c", 'pw-play "$0" 2>/dev/null || paplay "$0" '
                                  '2>/dev/null || aplay -q "$0"', path])
        if kind == "speed":
            return self.cycle_speed
        if kind == "toggle":
            return self.toggle_speed
        if kind == "none":
            return lambda: None
        raise ValueError(f"acao desconhecida: {spec!r}")

    def cycle_speed(self):
        steps = self.speed_steps
        nxt = next((s for s in steps if s > self.sens + 1e-6), steps[0])
        self.sens = nxt
        notify(f"Sensibilidade: {nxt:g}x")

    def toggle_speed(self):
        # Como no original: alterna entre a velocidade principal e a personalizada
        self.sens = self.custom_sens if self.sens == self.base_sens else self.base_sens
        notify(f"Sensibilidade: {self.sens:g}x")

    def handle(self, etype, code, value):
        """Processa um evento do mouse fisico. Retorna True se deve repassar."""
        if etype == EV_KEY:
            action = self.map.get((EV_KEY, code, 0))
            if action is None:
                return True
            if value == 1:
                action()
            return False
        if etype == EV_REL:
            if code in (REL_X, REL_Y) and self.sens != 1.0:
                i = code
                self.acc[i] += value * self.sens
                out = int(self.acc[i])
                self.acc[i] -= out
                if out:
                    self.ui.emit(EV_REL, code, out, syn=False)
                return False
            if code in (REL_WHEEL, REL_HWHEEL):
                sign = 1 if value > 0 else -1
                action = self.map.get((EV_REL, code, sign))
                if action is None:
                    return True
                for _ in range(abs(value)):
                    action()
                return False
            if code in (REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES):
                lo = REL_WHEEL if code == REL_WHEEL_HI_RES else REL_HWHEEL
                sign = 1 if value > 0 else -1
                return (EV_REL, lo, sign) not in self.map
        if etype == EV_MSC:
            return False
        return True


def run_daemon(args):
    cfg = load_config(args.config)
    ui = UInput()
    remap = Remapper(cfg, ui)
    cfg_mtime = os.path.getmtime(args.config)
    print(f"[info] config: {args.config}", flush=True)
    devs = {}           # fd -> caminho
    pending = False
    last_scan = 0
    try:
        while True:
            now = time.time()
            if now - last_scan > 2:
                last_scan = now
                for path, name in find_event_devices():
                    if path in devs.values():
                        continue
                    try:
                        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
                    except PermissionError:
                        sys.exit(f"Sem permissao para {path}. Rode ./install.sh (regra udev).")
                    except OSError:
                        continue
                    fcntl.ioctl(fd, EVIOCGRAB, 1)
                    devs[fd] = path
                    print(f"[info] mouse conectado: {name} ({path})", flush=True)
            r, _, _ = select.select(list(devs), [], [], 1.0)
            try:
                m = os.path.getmtime(args.config)
            except OSError:
                m = cfg_mtime
            if m != cfg_mtime:
                cfg_mtime = m
                try:
                    remap.reload(load_config(args.config))
                    notify("Configuracao recarregada")
                except Exception as e:
                    notify(f"Erro no config: {e}")
            for fd in r:
                try:
                    data = os.read(fd, EVENT.size * 64)
                except BlockingIOError:
                    continue
                except OSError as e:
                    if e.errno not in (errno.ENODEV, errno.EIO):
                        raise
                    print(f"[info] desconectado: {devs[fd]}", flush=True)
                    os.close(fd)
                    del devs[fd]
                    continue
                for off in range(0, len(data), EVENT.size):
                    _, _, t, c, v = EVENT.unpack_from(data, off)
                    if t == EV_SYN:
                        if pending:
                            ui.syn()
                            pending = False
                        continue
                    if remap.handle(t, c, v):
                        ui.emit(t, c, v, syn=False)
                    pending = True
    except KeyboardInterrupt:
        pass
    finally:
        for fd in devs:
            os.close(fd)
        ui.close()

# ------------------------------------------------------------------- HID
def hid_command(payload, read_len=32, path=None):
    """Envia [0xBA]+payload como output report e le o input report 0xBA,
    igual ao driver Windows (HidD_SetOutputReport + HidD_GetInputReport)."""
    if path is None:
        path, _ = find_hidraw()
    if not path:
        raise RuntimeError("interface HID vendor (report 0xBA) nao encontrada")
    fd = os.open(path, os.O_RDWR)
    try:
        out = bytes([0xBA]) + bytes(payload)
        out = out.ljust(32, b"\0")
        os.write(fd, out)
        time.sleep(0.04)
        buf = bytearray(read_len)
        buf[0] = 0xBA
        try:
            n = fcntl.ioctl(fd, HIDIOCGINPUT(read_len), buf, True)
            return bytes(buf[:n if n > 0 else read_len])
        except OSError:
            # fallback: espera o report chegar pela interrupcao
            end = time.time() + 0.5
            while time.time() < end:
                r, _, _ = select.select([fd], [], [], 0.1)
                if r:
                    d = os.read(fd, 64)
                    if d and d[0] == 0xBA:
                        return d
            raise RuntimeError("sem resposta do dispositivo")
    finally:
        os.close(fd)


def wait_status(paths, timeout=4.0):
    """Receptor 24AE:1870 e cabo 24AE:4500: enviam sozinhos, a cada ~3 s, o
    input report  BB B0 01 D6 06 01 <bateria %>.
    Confirmado empiricamente: so o ultimo byte varia (14 -> 39 ao ligar o
    carregador). Os bytes 2..5 sao constantes e ainda sem significado conhecido."""
    fds = {}
    for p, pid in paths:
        try:
            fds[os.open(p, os.O_RDONLY | os.O_NONBLOCK)] = pid
        except PermissionError:
            raise
        except OSError:
            pass
    try:
        end = time.time() + timeout
        while fds and time.time() < end:
            r, _, _ = select.select(list(fds), [], [], 0.2)
            for fd in r:
                try:
                    d = os.read(fd, 64)
                except OSError:
                    continue
                if len(d) >= 7 and d[0] == 0xBB and d[1] == 0xB0:
                    return d, fds[fd]
    finally:
        for fd in fds:
            os.close(fd)
    return None, None


def read_battery(path=None):
    """Retorna {"percent", "charging", "raw", "proto", "dev_id"} ou levanta excecao."""
    devs = find_hidraws()
    if not devs:
        raise RuntimeError("interface HID vendor nao encontrada")
    # receptor oficial 2030: protocolo do driver Windows (BA AE 01)
    for p, pid in devs:
        if pid == 0x2030:
            r = hid_command([0xAE, 0x01], path=p)
            if len(r) >= 7 and r[0] == 0xBA:
                return {"percent": r[5], "charging": False, "raw": r,
                        "proto": "BA AE 01", "dev_id": r[1:5].hex().upper()}
    d, _pid = wait_status(devs)
    if not d:
        raise RuntimeError("o mouse nao enviou status de bateria (desligado/dormindo?)")
    charging = any(pid in WIRED_PIDS for _, pid in devs)
    return {"percent": d[6], "charging": charging, "raw": d, "proto": "BB B0",
            "dev_id": None}


def cmd_info(args):
    path, pid = find_hidraw()
    ev, name = find_event_device()
    print(f"Mouse (evdev): {ev or 'nao encontrado'}  {name or ''}")
    if not path:
        print("Interface HID vendor: nao encontrada")
        return 1
    print(f"Interface HID: {path}  (24ae:{pid:04x} {KNOWN_PRODUCTS.get(pid, '')})")
    try:
        b = read_battery(path)
    except PermissionError:
        print(f"Sem permissao para {path}. Rode ./install.sh (regra udev).")
        return 1
    except Exception as e:
        print(f"Falha na consulta: {e}")
        return 1
    r = b["raw"]
    print(f"Resposta ({b['proto']}):", r.hex(" "))
    if b["proto"] == "BB B0":
        print(f"Bateria: {b['percent']}%" + ("  (carregando)" if b["charging"] else ""))
        return 0
    # Layout (de MT760LDriver.exe, funcao 0x41f020):
    # [0]=0xBA [1..4]=ID do dispositivo [5]=bateria [6]=canal/modo
    # [7..30]=6 IDs pareados (uint32 big-endian)
    dev_id = r[1:5].hex().upper()
    print(f"ID do dispositivo: {dev_id}")
    print(f"Bateria: {r[5]}")
    print(f"Canal/modo: {r[6]}")
    ids = [r[7 + 4 * i:11 + 4 * i].hex().upper() for i in range(6)]
    for i, x in enumerate(ids):
        if x.strip("0"):
            print(f"  Pareado #{i + 1}: {x}")
    return 0


def cmd_raw(args):
    payload = bytes.fromhex(args.hex)
    print("Resposta:", hid_command(payload).hex(" "))


def cmd_monitor(args):
    path, name = find_event_device()
    if not path:
        sys.exit("mouse nao encontrado")
    names = {(EV_KEY, c): n for n, (t, c, s) in BUTTONS.items() if t == EV_KEY}
    print(f"Lendo {name} ({path}). Pressione os botoes; Ctrl+C para sair.")
    fd = os.open(path, os.O_RDONLY)
    try:
        while True:
            data = os.read(fd, EVENT.size)
            _, _, t, c, v = EVENT.unpack(data)
            if t == EV_KEY:
                print(f"botao {names.get((t, c), hex(c))} {'pressionado' if v else 'solto'}")
            elif t == EV_REL and c in (REL_WHEEL, REL_HWHEEL):
                base = "wheel" if c == REL_WHEEL else "side_wheel"
                print(f"{base}_{'up' if v > 0 else 'down'} ({v})")
    except KeyboardInterrupt:
        pass


def cmd_actions(args):
    for k, v in ACTIONS.items():
        print(f"  {k:22} {v}")
    print('\nTambem: "keys:<combo>", "cmd:<comando>", "open:<caminho>", "sound:<arquivo>"')


def main():
    ap = argparse.ArgumentParser(description="Driver Linux para Rapoo MT760L")
    ap.add_argument("--config", default=CONFIG_PATH)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("daemon", help="roda o remapeamento (servico)")
    sub.add_parser("info", help="bateria, canal e IDs pareados")
    sub.add_parser("monitor", help="mostra os botoes pressionados")
    sub.add_parser("actions", help="lista as acoes disponiveis")
    p = sub.add_parser("raw", help="envia comando HID cru (avancado)")
    p.add_argument("hex", help='ex: "AE 01"')
    args = ap.parse_args()
    return {"daemon": run_daemon, "info": cmd_info, "monitor": cmd_monitor,
            "actions": cmd_actions, "raw": cmd_raw}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main() or 0)
