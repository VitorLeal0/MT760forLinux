# Protocolo HID do Rapoo MT760L (receptor 24AE:1870)

> Documentação de interoperabilidade escrita a partir de observação e engenharia reversa, para que
> o MT760L funcione no Linux. Não contém código da Rapoo. Os IDs, nomes e IPs são exemplos.

Engenharia reversa de `RapooDevice.dll` (A HUB Office 1.0.1, classe `MouseForTelink` / `BaseProtocol`),
validada no hardware em 2026-10-27.

## Transporte
- Interface vendor do receptor: hidraw com report ID `0xBA` (31 bytes de dados), usage page 0xFF00.
- Enviar: `write()` de 32 bytes = `BA <cmd...>` com zeros no fim (no A HUB: `hid_write`).
- Ler resposta: `HIDIOCGINPUT(32)` com `buf[0]=0xBA` (no A HUB: `hid_get_input_report`).
  A resposta vem **sem** o report ID:
  - `01 ...` = sucesso
  - `02` = TIMEOUT: o receptor não alcançou o mouse (mouse dormindo ou em outro canal)
  - `00` / outro = comando inválido
- Uma leitura por comando; uma segunda leitura sem novo comando dá EPIPE.

## Comandos ao receptor (`BA A3 ...`, `BA Ax`)
| bytes | função |
|---|---|
| `BA A3 A4 <len> <addr16 LE> 00 00` | ler memória do receptor → `01 <len> <addr16> <dados>` |
| `BA A3 A5 <len> <addr16 LE> 00 00 <dados>` | gravar memória do receptor (`setDgChannel` = endereço 0x70) |
| `BA A7` | iniciar pareamento |
| `BA AC A7` | resetDongle |
| `BA AD 52` | **restaurar receptor de fábrica (não usar)** |
| `BA B0 ...` / `BA B1` | versão de firmware |
| `BA CC ...` | trocar PID (não usar) |

Memória do receptor: `0x00` = VID/PID (`ae 24 .. 70 18`), `0x20`… = ID RF deste receptor (`cc bb aa 99`).

## Comandos ao mouse através do receptor (`BA A5 <op> ...`)
O segundo byte é `A5` para o MT760 sem fio (DeviceType 5/6; `FF` para outros tipos).

| bytes | função | resposta |
|---|---|---|
| `BA A5 AA` | bateria (`getElectricity`) | `01 01 <pct>` |
| `BA A5 A4 <len> <addr32 LE>` | ler memória do mouse | `01 00 00 00 <dados>` |
| `BA A5 A5 <len> <addr32 LE> <dados>` | gravar memória do mouse | |
| `BA A5 AE 00 00 00 00 <idx>` | **trocar de canal/PC** (`setTelinkMTSwitchPairDongle`) | `01` |
| `BA A5 AE 00 00 00 00 FF <id32 BE>` | trocar para o receptor com esse ID | `01` |

Memória do mouse:
- `0x23` = índice do canal atual (`getDeviceID`, lido com `BA A5 A4 18 23 00 00 00`, byte `resp[4]`)
- `0x34` = IDs RF dos receptores pareados, 4 bytes por canal (`getDeviceRFBuffer`)
  - exemplo: canal 0 `44 33 22 11`, canal 1 `88 77 66 55` (PC Windows), canal 2 `cc bb aa 99` (este PC)

### Configuração (memória do mouse, endereços de `MouseForTelink`)
Ler: `BA A5 A4 <len> <addr32 LE>`. Gravar: `BA A5 A5 <len> <addr32 LE> <dados>`.

| endereço | conteúdo | status |
|---|---|---|
| `0x600 + 4×tecla` | botão (11 teclas, ver abaixo) | leitura ✅ escrita ✅ (voltar→avançar testado, efeito imediato) |
| `0x630` | polling com fio (código = 1000/Hz: 1=1000, 2=500, 4=250, 8=125) | leitura ✅ |
| `0x631` | polling sem fio (mesmo código) | leitura ✅ escrita ✅ (01→08 medido 1000→125 Hz) |
| `0x638` | DPI X: 7 × u16 LE, valor = DPI/50 (`10 14 18 23 30 40 50` = 800…4000) | leitura ✅ |
| `0x646` | número de níveis − 1 (`06`) | leitura ✅ |
| `0x648` | **nível de DPI atual − 1** (`03` = nível 4) | leitura ✅ escrita ✅ (efeito imediato) |
| `0x64C` | DPI Y: 7 × u16 (mesmo formato) | leitura ✅ |
| `0x65A` / `0x65C` | níveis − 1 / nível atual do bloco Y | leitura ✅ |
| `0x1000` | macros (vazio = `ff`) | — |

### Macros (`MouseForTelink::setMacro`) — verificado no hardware
- 12 espaços (`MaxMacroNum`), em `0x1000 + id × 0x1000`; de `0xD000` em diante o mouse não responde.
- Conteúdo: `u16 comprimento total` · `u16 reprodução` (N vezes · `0xFFFE` o botão liga/desliga ·
  `0xFFFF` enquanto pressionado) · eventos de 3 bytes `u16 (atraso_ms×2 | ação) + código`
  (ação 1 = pressionar, 0 = soltar; código = HID usage do teclado, ou `F0..F4` = botões do mouse).
  O A HUB também tem registros de 6 bytes (`F7`, movimento X/Y em 12 bits) e de 10 bytes (`F8`, roda).
- Gravação em blocos de 24 bytes; **o mouse só grava quando recebe todos os bytes anunciados no
  cabeçalho**, e **no máximo 48 bytes** (46 grava; 49 é rejeitado mesmo com pausas). Escritas no meio
  de um espaço vazio são ignoradas. A leitura logo após gravar pode trazer o conteúdo antigo por alguns ms.
- Apagar: gravar `00 00 00 00` no início do espaço. Atribuir a um botão: código `05 00 <id> 00`.

Teclas (`MT760.json`): 0 esquerdo, 1 meio, 2 direito, 3 DIY, 4 DPI cycle+, 5 lateral avançar,
6 lateral voltar, 7 roda lateral esq., 8 roda lateral dir., 9 roda cima, 10 roda baixo.

Codificação de tecla (4 bytes):
| bytes | ação |
|---|---|
| `07 00 00 00` | desativado |
| `01 00 <HID usage> 00` | tecla do teclado |
| `02 00 <tecla> <modificador>` | atalho (copiar, colar…) |
| `03 00 <máscara> 00` | botão do mouse: 01 esq, 02 dir, 04 meio, 08 voltar, 10 avançar |
| `04 00 <consumer u16 LE>` | mídia/consumer (volume, play…) |
| `05 00 <id> 00` | macro |
| `08 00 <código> 00` | DPI (03 = cycle+, 04 = cycle−, 05 = +, 06 = −) |
| `0B 00 01/02 00` | roda vertical cima/baixo |
| `0C 00 01/02 00` | roda horizontal |

Cross-screen (modo offline): quando o cursor encosta na borda, enviar `BA A5 AE 00 00 00 00 <canal do outro PC>`.

## Status espontâneo
O receptor envia sozinho, a cada ~3 s, o input report `BB B0 01 D6 06 01 <bateria %>`.

## Observações
- Leituras com `<len>` menor que 8 podem voltar só o status; use 8 ou mais.
- O mouse precisa estar acordado (status `02` = dormindo); repetir até ~10 s.

## Modo LAN (A HUB Office ↔ A HUB Linux)

Bibliotecas: JQNetwork 0.6.6 (código aberto) + KCloudClipboard (Rapoo).

### Descoberta — verificada com o A HUB do Windows
JSON compacto por UDP para `228.12.23.34:12345` e para broadcast `255.255.255.255:12345`, a cada 2 s;
um nó expira após 7 s sem anúncios. Exemplo real:
```json
{"appendData":"112233445566778899aabbccffffffffffffffffffffffffffffffffffffffff-0201",
 "dataPackageIndex":88,"dutyMark":"4500-55667788-PC-Windows","ipList":["192.168.1.20","172.16.0.1"],
 "nodeMarkSummary":"0123456789abcdef0123456789abcdef","requestFeedback":true,"requestOffline":false}
```
- `dutyMark` = `4500-<ID RF do receptor deste PC, u32 hex>-<nome do computador>`
- `appendData` = 10 IDs pareados no mouse (u32 hex, `ffffffff` = vazio) + `-` + canal atual do mouse
  (1 byte hex) + canal do receptor deste PC (1 byte hex)
- `requestFeedback: true` no primeiro anúncio: quem recebe responde com o próprio anúncio.
- `requestOffline: true` ao sair.

### Mensagens TCP — pacote JQNetwork
Cabeçalho de 24 bytes little-endian: `i8 0x7d, i8 packageFlag (1 = dados), i32 randomFlag,
i8 metaFlag, i32 metaTotal, i32 metaAtual, i8 payloadFlag, i32 payloadTotal, i32 payloadAtual`.
Flag 1 = sem compressão, 2 = `qCompress` (u32 BE + zlib). Metadado = `{"targetActionFlag":…,"appendData":…}`,
payload = JSON. A resposta usa o mesmo `randomFlag`.

Ações vistas no executável: `SetClipboardToText`, `SetClipboardToImage`, `SetClipboardToFiles`, `SetDeviceArray`,
`requireFile`, `sendFile`; campos `text`, `port`, `filePath`, `succeed`, `message`.

### O que foi verificado com o A HUB Office 1.0.1 do Windows
- O `A HUB Office.exe` **não escuta em nenhuma porta TCP**, mesmo com modo LAN, cross-screen e "cut" ativados
  (`netstat -ano`). A porta 43020 aberta no Windows é do Spotify. Nos testes ele nunca abriu conexão TCP para o
  Linux. A área de transferência entre o Linux e o Windows, portanto, não tem por onde passar.
- O `appendData` do Windows pode vir vazio (`""`) mesmo com o mouse conectado a ele.
- **Quando outro nó com o mesmo mouse aparece na rede, o A HUB do Windows deixa de devolver o mouse pela borda.**
  O A HUB para Linux por isso só escuta a rede por padrão (descobre o Windows sem se anunciar); assim o
  Windows continua devolvendo o mouse normalmente.
- O Windows memoriza os nós LAN; um `nodeMarkSummary` novo a cada execução cria entradas duplicadas.
  O app usa um identificador fixo derivado do `/etc/machine-id`.
