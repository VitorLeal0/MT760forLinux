# MT760forLinux

Configurador para Linux do mouse sem fio **Rapoo MT760L** (receptor `24AE:1870`).

> **Projeto independente.** Não é afiliado, endossado nem suportado pela Rapoo.
> "Rapoo", "MT760" e "A HUB" são marcas dos seus respectivos donos e aparecem aqui só para
> indicar com qual hardware e software este projeto é compatível. Nenhum código, imagem ou
> arquivo da Rapoo é distribuído neste repositório.

> **Aviso.** O app grava na memória do mouse (botões, DPI, taxa de polling, macros) usando um
> protocolo descoberto por engenharia reversa para fins de interoperabilidade. Ele foi testado em
> um único MT760L. Use por sua conta e risco; não há garantia (veja a [licença](LICENSE)).
> Comandos perigosos conhecidos (restaurar o receptor de fábrica, trocar o PID) não são usados.

## Funções

- **Botões**: as funções são gravadas no próprio mouse e valem em qualquer computador
  (botões do mouse, rolagem, DPI, mídia, atalhos, teclas, macros, desativar).
- **Macros**: 12 espaços no mouse, com editor (teclas, cliques, atrasos), gravação pelo teclado e
  três modos de reprodução (repetir N vezes, o botão liga/desliga, enquanto pressionado).
  Limite do firmware: 14 eventos por macro.
- **DPI**: nível atual e valores dos 7 níveis (50–4000).
- **Desempenho**: taxa de polling sem fio e com fio (125–1000 Hz).
- **Cross-screen (modo offline)**: com um receptor pareado em cada computador, empurre o cursor
  contra a borda da tela para levar o mouse ao outro computador. Funciona junto com o A HUB
  oficial do Windows, que faz o caminho de volta.
- **Modo LAN**: encontra os outros computadores com A HUB na rede, mostra em que canal está o
  receptor de cada um e dá nome automático aos canais.
- Bateria e canal atual sempre visíveis.

## Instalação

Requisitos: Linux com systemd, [Rust](https://rustup.rs) (stable) e `sudo` uma vez para a regra udev.

```sh
git clone <este repositório> MT760forLinux
cd MT760forLinux
./app/install.sh
```

Isso compila e instala `~/.local/bin/mt760`, o atalho **MT760forLinux** no menu de aplicativos, a
regra udev (`/etc/udev/rules.d/70-mt760.rules`, acesso ao receptor sem root) e o serviço do usuário
`mt760-cross` (cross-screen e modo LAN).

### Linha de comando
```sh
mt760 --info        # bateria, DPI, polling e canais pareados
mt760 --switch 2    # envia o mouse para o canal 2 (número ou nome dado na interface)
mt760 --daemon      # serviço de cross-screen e modo LAN
```

### Remapeamento por software (opcional)

`python/` tem um daemon que executa ações que o mouse não sabe fazer sozinho (abrir um programa,
arquivo ou pasta, tocar um som, alternar a sensibilidade). Ele captura o mouse via evdev e reemite
os eventos via uinput, sem dependências além do Python 3.11+.
```sh
./python/install.sh
mt760-remap monitor   # mostra o nome de cada botão ao pressionar
```

## Limitações

- **Wayland**: nenhum programa consegue ler a posição global do cursor. O cross-screen estima a posição
  pelo movimento bruto do sensor; calibre a largura na página Cross-screen. Com aceleração do ponteiro
  a estimativa é aproximada, por isso a troca exige empurrar um pouco além da borda.
- **Modo LAN**: por padrão o app só escuta a rede. Se este computador se anunciar como parceiro LAN, o
  A HUB do Windows deixa de devolver o mouse pela borda. A área de transferência entre Linux e Windows
  não funciona: o A HUB do Windows não abre nenhuma porta TCP para isso.
- **Macros**: no máximo 14 eventos de tecla/botão; macros com movimento ou roda não são editáveis.
- O mouse precisa estar acordado para responder; mexa nele se aparecer "o mouse não respondeu".
- Firewall: com UFW ativo, libere `UDP 12345` da rede local para o modo LAN.

## Estrutura

| caminho | conteúdo |
|---|---|
| `app/` | aplicativo principal (Rust + egui) |
| `app/assets/` | ilustrações e ícones próprios (GPL-3.0) |
| `python/` | daemon opcional de remapeamento por software |
| `udev/` | regra de permissão para o receptor |
| [`PROTOCOL.md`](PROTOCOL.md) | documentação do protocolo HID e LAN |

Para desenvolver: `cd app && cargo test` (o teste de hardware roda com
`cargo test -- --ignored`, com o mouse acordado). `MT760_SCREENSHOT=saida.png:pagina mt760` salva uma
captura da janela e sai.

## Licença

[GPL-3.0-or-later](LICENSE). Copyright (C) 2026 MT760forLinux contributors.
