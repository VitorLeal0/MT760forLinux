#!/bin/sh
# MT760forLinux — compila e instala para o usuário atual.
# Copyright (C) 2026 MT760forLinux contributors. Licença GPL-3.0-or-later.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

command -v cargo >/dev/null || { echo "Instale o Rust antes: https://rustup.rs"; exit 1; }
cargo build --release --manifest-path "$DIR/Cargo.toml"

mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons" "$HOME/.config/systemd/user"
install -m 755 "$DIR/target/release/mt760" "$HOME/.local/bin/mt760"
install -m 644 "$DIR/assets/logo.svg" "$HOME/.local/share/icons/mt760forlinux.svg"

# Acesso ao receptor (hidraw) sem root
RULE=/etc/udev/rules.d/70-mt760.rules
if [ ! -f "$RULE" ]; then
    echo ">> Instalando a regra udev (precisa de sudo)"
    sudo install -m 644 "$DIR/../udev/70-mt760.rules" "$RULE"
    sudo udevadm control --reload-rules
    sudo udevadm trigger --subsystem-match=hidraw --subsystem-match=input --subsystem-match=misc
fi

# Versões de desenvolvimento usavam outros nomes
if systemctl --user list-unit-files ahub-cross.service >/dev/null 2>&1; then
    systemctl --user disable --now ahub-cross.service 2>/dev/null || true
    rm -f "$HOME/.config/systemd/user/ahub-cross.service"
fi
rm -f "$HOME/.local/bin/ahub" "$HOME/.local/share/applications/ahub.desktop" "$HOME/.local/share/icons/ahub.png"

cat > "$HOME/.local/share/applications/mt760forlinux.desktop" <<EOT
[Desktop Entry]
Type=Application
Name=MT760forLinux
Comment=Configurar o mouse Rapoo MT760L
Exec=$HOME/.local/bin/mt760
Icon=$HOME/.local/share/icons/mt760forlinux.svg
Categories=Settings;HardwareSettings;
StartupWMClass=io.github.mt760forlinux
EOT

cat > "$HOME/.config/systemd/user/mt760-cross.service" <<EOT
[Unit]
Description=MT760forLinux - cross-screen e modo LAN

[Service]
ExecStart=$HOME/.local/bin/mt760 --daemon
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOT
systemctl --user daemon-reload
systemctl --user enable mt760-cross.service
systemctl --user restart mt760-cross.service

echo
echo "Pronto. Abra \"MT760forLinux\" no menu de aplicativos (ou rode: mt760)."
echo "Linha de comando: mt760 --info | mt760 --switch <canal>"
