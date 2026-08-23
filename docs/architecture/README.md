# Teamscord — arquitetura e operação

Esta pasta contém a documentação visual da versão 1.0.0. Cada diagrama tem duas formas:

- .mmd: fonte Mermaid versionada e editável.
- .png: renderização compilada para leitura rápida, revisão e compartilhamento.

## Mapa dos diagramas

| Arquivo | O que explica |
| --- | --- |
| 01-system-architecture | Desktop nodes, relay TCP, protocolos Rust, SQLite, Credential Manager e GossipSub |
| 02-message-sync-sequence | Convite assinado, conexão, cursores, outbox e entrega de mensagem |
| 03-call-sequence | Presença, sinalização SDP/ICE e mídia WebRTC DTLS-SRTP |
| 04-local-storage-erd | Tabelas SQLite e cardinalidades principais |
| 05-node-connection-state | Estados do node e reconexão com backoff |
| 06-release-and-deploy-flow | Checks, NSIS, checksum, GitHub Release e caminho de deploy Railway |

## Componentes

### Aplicação Windows

O executável Tauri inicializa o node Rust local e a interface React/TypeScript no WebView2. A identidade Ed25519, a chave X25519 e as chaves históricas de grupos são protegidas pelo Windows Credential Manager.

O bridge TypeScript chama os comandos Tauri e recebe eventos node://event, incluindo:

- snapshot, sync-progress e connection-diagnostic;
- relay-state, peer-updated e peer-presence;
- message, group-control, member-updated e key-epoch-changed;
- call-signal, call-state e media-error.

### Node P2P

Cada instalação é um node com:

- TCP, QUIC, Noise, Yamux e libp2p relay client;
- mDNS para descoberta na mesma LAN;
- Identify e Ping;
- GossipSub para entrega em tempo real;
- request-response para sincronização paginada e sinalização de calls;
- endereços conhecidos persistidos e redial com backoff;
- outbox SQLite para mensagens e eventos administrativos criados offline.

O relay não armazena mensagens. Ele auxilia conexão e circuito; o histórico continua nos SQLite dos nodes.

### Mensagens e sincronização

Mensagens são cifradas com a chave da época do grupo e assinadas pela identidade do autor. O event_id é a chave de deduplicação. Após reconexão, o node pede páginas por grupo/canal usando (created_at, event_id) como cursor determinístico.

Eventos administrativos seguem o mesmo princípio de assinatura, auditoria e deduplicação. Kick/ban registra o evento e dispara rotação de chave para bloquear acesso às mensagens futuras.

### Calls

Calls têm presença e signaling pelo node Rust. A mídia não passa pelo relay: o WebView2 cria uma RTCPeerConnection por participante remoto, usa DTLS-SRTP e captura microfone/tela pelas APIs WebRTC. TURN é opcional e permanece configurável no Credential Manager.

## Deploy do relay

O código do relay está em infra/relay/README.md. Em Railway:

1. raiz do serviço: infra/relay;
2. build: Dockerfile;
3. variável TEAMSCORD_RELAY_TCP_PORT=4001;
4. variável TEAMSCORD_RELAY_ENABLE_QUIC=false;
5. volume persistente em /app/data;
6. TCP Proxy público apontando para a porta interna 4001;
7. variável TEAMSCORD_RELAY_PUBLIC_ADDRESS com o host e a porta pública do proxy, sem PeerId;
8. endereço final no formato libp2p com o PeerId do relay;
9. build desktop com TEAMSCORD_DEFAULT_RELAY_ADDRESS definido.

### Estado verificado nesta documentação

O GitHub mantém a release publicada [v1.0.4](https://github.com/PessoasBoas/Teamscord/releases/tag/v1.0.4); a build `1.0.5` usa uma única reserva IPv4 e atualiza o estado quando o relay aceita a reserva.

Na verificação desta documentação, o projeto Teamscord-Relay está publicado no workspace pessoal, com volume persistente e TCP Proxy ativo. O relay anuncia o endereço público e o app recebe o multiaddr padrão no build de release; o relay não armazena mensagens e o histórico continua local nos nodes.

Dados verificados da implantação atual:

- TCP Proxy: `altaria.proxy.rlwy.net:46712` para a porta interna `4001`.
- PeerId do relay: `12D3KooWNw8qUoVxFy8XcRkXhwPF4rdGjz4mqRf3hgqnoJbBvtwt`.
- Multiaddr do aplicativo: `/dns4/altaria.proxy.rlwy.net/tcp/46712/p2p/12D3KooWNw8qUoVxFy8XcRkXhwPF4rdGjz4mqRf3hgqnoJbBvtwt`.
- Volume Railway: `/app/data`, usado para preservar a identidade do relay.

## Comandos

~~~powershell
npm run check
npm run release
npx --yes @mermaid-js/mermaid-cli -i docs/architecture/01-system-architecture.mmd -o docs/architecture/01-system-architecture.png
~~~

Para recompilar todos os PNGs:

~~~powershell
$diagramDir = "docs/architecture"
Get-ChildItem $diagramDir -Filter *.mmd | ForEach-Object {
  npx --yes @mermaid-js/mermaid-cli -i $_.FullName -o (Join-Path $diagramDir ($_.BaseName + ".png")) -t neutral -b white -w 2400
}
~~~

## Fontes de implementação

- Node e comandos Tauri: src-tauri/src/lib.rs
- Protocolos e envelopes: src-tauri/src/protocol.rs
- Migrações SQLite: src-tauri/src/storage.rs
- Criptografia e convites: src-tauri/src/crypto.rs
- Bridge frontend: src/lib/tauri.ts
- WebRTC: src/lib/webrtc.ts
- Relay: infra/relay/src/main.rs
