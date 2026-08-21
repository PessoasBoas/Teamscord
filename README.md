# Teamscord

Teamscord 0.2.1 é um chat de grupos privados com um node local por instalação. Mensagens, eventos administrativos e épocas de chave são sincronizados entre peers; áudio e compartilhamento de tela usam uma malha WebRTC P2P opcional.

## Stack

- Tauri 2 + Rust para o desktop e o node.
- React + TypeScript + Vite para a interface.
- libp2p TCP + QUIC + Noise + Yamux + GossipSub + request-response.
- SQLite para histórico, membros, auditoria e épocas de chave; Credential Manager do Windows para identidade Ed25519, acordo X25519 e configuração ICE.

## Desenvolvimento

```bash
npm install
npm run tauri dev
```

O watcher do Vite ignora `src-tauri/target`, evitando o erro `EBUSY` durante a compilação do Cargo.

## Verificação

```bash
npm run check
```

## Release Windows

O instalador NSIS é gerado sem assinatura Authenticode na primeira versão:

```bash
npm run release
```

O comando gera o `.exe` em `src-tauri/target/release/bundle/nsis` e um arquivo `.sha256`. O Windows pode mostrar um aviso do SmartScreen até o produto possuir certificado de assinatura. O app consulta periodicamente a release estável de `PessoasBoas/Teamscord`; quando encontra uma versão maior, mostra um popup com link para o instalador NSIS e as notas da versão. A instalação continua manual e confirmada pelo usuário: não há atualização silenciosa nem auto-update embutido nesta versão.

Ao publicar uma versão, crie uma GitHub Release com uma tag semver (por exemplo `v0.2.1`) e anexe o `.exe` x64 gerado junto com o `.sha256`. O verificador só aceita releases estáveis, URLs do repositório oficial e o instalador cujo nome termina em `_x64-setup.exe`.

## Grupos e rede

Crie um grupo no app e compartilhe o convite assinado. O convite expira em 30 dias; a chave do grupo e a identidade do node ficam no armazenamento seguro do Windows. Para conectar diretamente, use um multiaddress exibido no painel de rede; o mesmo painel permite salvar bootstraps para discagem automática e relays opcionais. Depois que um peer é identificado, o node guarda seus endereços anunciados e tenta redialar automaticamente quando a conexão cai; o bootstrap continua recomendado para recuperar peers após reiniciar o aplicativo.

Para ajudar conexões atrás de NAT, há um relay opcional em [infra/relay](infra/relay/README.md). Para mídia WebRTC fora da LAN, configure um Coturn opcional em [infra/turn](infra/turn/README.md); o app aceita ICE/STUN/TURN sem embutir servidor, domínio ou credenciais.

## Controles e mídia 0.2

O grupo tem cargos fixos `Owner`, `Admin`, `Mod` e `Member`. A interface oferece membros, permissões por canal, canais, convites e auditoria; ações destrutivas exigem confirmação. O Owner é a raiz de autoridade; um Admin só pode alterar `Mod`/`Member` quando sua concessão de Admin foi registrada por um evento assinado pelo Owner. Owner, Admin e Mod podem apagar mensagens conforme a matriz de permissões. Expulsão e banimento encerram o acesso futuro ao rotacionar a chave do grupo e distribuir a nova época somente aos membros ativos.

O menu do nome do servidor concentra convite e configurações do grupo; a engrenagem do rodapé abre as preferências locais de tema, fonte, densidade, perfil e áudio. Canais de voz concentram chat, áudio e tela na mesma conversa. O Owner pode transferir a propriedade ou excluir o grupo após confirmação.

Calls têm limite de 8 participantes e uma tela compartilhada por vez, sem câmera e sem SFU. O áudio usa `getUserMedia` com seleção de microfone; a tela usa `getDisplayMedia` quando o WebView2 e as permissões do Windows disponibilizam a captura. Sem TURN configurado, a aplicação informa que a call dependerá de conexão direta; o teste de duas redes deve ser feito antes de distribuir a build beta.

## Teste de release

`npm run check` valida TypeScript, formatação/clippy/testes Rust, o smoke de malha WebRTC, a captura nativa de tela no Edge e o circuito de dois clientes através do relay local. `npm run test:browser` acrescenta um smoke test real no Edge headless com quatro nós, seis conexões P2P, microfones falsos, offer/answer, ICE, áudio em malha e renegociação de uma faixa sintética de tela com áudio, incluindo parada da captura. `npm run test:browser:screen` usa `getDisplayMedia()` nativo com seleção automática de desktop, validando faixa viva e encerramento. A faixa sintética torna o teste de renegociação reproduzível; o teste manual de dois PCs, NAT e Coturn exige Windows 10/11 em duas redes e não é substituído pelos smokes locais.

### Gate manual antes da beta de mídia

1. No PC A, instale o NSIS, crie um servidor e compartilhe o convite assinado e um endereço `listen` substituindo `0.0.0.0` pelo IP local.
2. No PC B, instale a mesma versão, entre pelo convite, conecte-se ao endereço do PC A e confirme no topo `node online`, `sincronizado` e os dois membros.
3. Envie uma mensagem em cada PC, feche e reabra um deles e confirme que o histórico e a lista de membros continuam presentes.
4. Em um canal de voz, entre nos dois PCs, aceite o microfone, navegue para outro canal sem sair da call, alterne mute/deafened, troque o dispositivo e confirme a presença/conexão de cada participante.
5. Compartilhe uma tela, valide áudio do sistema quando disponível, pare manualmente, feche a janela de captura e reconecte; a segunda tela deve ser recusada enquanto a primeira estiver ativa.
6. Repita os passos 2–5 em redes diferentes. Com Coturn configurado, confirme que a call conecta pelo fallback; sem Coturn, a interface deve explicar que a conexão direta falhou.

O resultado dessa validação deve incluir versão instalada, redes usadas, permissões concedidas, estado da call e qualquer erro de dispositivo; somente depois disso a build deve ser distribuída como beta de mídia.
