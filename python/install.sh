#!/bin/sh
# MT760forLinux — daemon opcional de remapeamento por software (Python).
# Executa ações que o mouse não sabe fazer sozinho (abrir programa, arquivo,
# tocar som...) e ajusta a sensibilidade. Licença GPL-3.0-or-later.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
LIB="$HOME/.local/share/mt760forlinux"
BIN="$HOME/.local/bin"

mkdir -p "$LIB" "$BIN" "$HOME/.config/systemd/user"
install -m 755 "$DIR/mt760_remap.py" "$LIB/"
ln -sf "$LIB/mt760_remap.py" "$BIN/mt760-remap"

RULE=/etc/udev/rules.d/70-mt760.rules
echo ">> Instalando a regra udev e o módulo uinput (precisa de sudo)"
sudo install -m 644 "$DIR/../udev/70-mt760.rules" "$RULE"
sudo modprobe uinput
echo uinput | sudo tee /etc/modules-load.d/mt760forlinux.conf >/dev/null
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --subsystem-match=hidraw --subsystem-match=misc

# Versão de desenvolvimento usava outro nome: não rodar dois daemons
systemctl --user disable --now rapoo-mt760.service 2>/dev/null || true

sed "s|@BIN@|$LIB/mt760_remap.py|" "$DIR/mt760-remap.service" > "$HOME/.config/systemd/user/mt760-remap.service"
systemctl --user daemon-reload
systemctl --user enable mt760-remap.service
systemctl --user restart mt760-remap.service

echo
echo "Pronto. Configure em ~/.config/rapoo-mt760/config.toml (recarrega sozinho)."
echo "Comandos: mt760-remap monitor | actions | info"
