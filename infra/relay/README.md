# Teamscord relay opcional

Este serviço só ajuda os nodes a atravessarem NAT/CGNAT; ele não armazena grupos nem mensagens do Teamscord.

## Subir em uma VPS

1. Instale Docker e Docker Compose na VPS.
2. Copie esta pasta para a máquina, copie `.env.example` para `.env` e ajuste as portas se necessário.
3. Libere TCP `4001` e UDP `4002` no firewall.
4. Execute `docker compose up -d --build`.
5. Veja o peer id e os endereços com `docker compose logs -f`.

O endereço para configurar no app deve incluir o peer id retornado pelo relay, por exemplo:

```text
/ip4/SEU_IP/tcp/4001/p2p/PEER_ID_DO_RELAY
```

O volume `relay-data` mantém a identidade do relay entre atualizações. Para produção pública, use firewall, limites de recursos e monitoramento da VPS.

O transporte libp2p usa Noise sobre TCP e QUIC; não há endpoint HTTP neste pacote que exija certificado TLS. Se a VPS usar proxy ou painel externo para observabilidade, mantenha esse acesso separado e protegido por HTTPS/TLS; não exponha o volume de identidade nem as portas do relay sem firewall.

## Deploy no Railway

O Railway deve usar o diretório `infra/relay` como raiz do serviço, o `Dockerfile` desta pasta e um volume persistente montado em `/app/data`. Configure:

```text
TEAMSCORD_RELAY_TCP_PORT=4001
TEAMSCORD_RELAY_ENABLE_QUIC=false
TEAMSCORD_RELAY_IDENTITY_PATH=/app/data/identity.bin
TEAMSCORD_RELAY_PUBLIC_ADDRESS=/dns4/SEU_HOST_TCP_PROXY/tcp/PORTA_PUBLICA
```

Depois do deploy, habilite um TCP Proxy público para a porta interna `4001`. Configure `TEAMSCORD_RELAY_PUBLIC_ADDRESS` com o host e a porta pública do proxy, sem o `PeerId`; o relay usará esse endereço para anunciar reservas:

```text
/dns4/SEU_HOST_TCP_PROXY/tcp/PORTA_PUBLICA
```

O endereço usado no aplicativo inclui o host, a porta pública do proxy e o `PeerId` exibido nos logs:

```text
/dns4/SEU_HOST_TCP_PROXY/tcp/PORTA_PUBLICA/p2p/PEER_ID_DO_RELAY
```

O TCP Proxy do Railway é o caminho suportado para este relay; QUIC continua disponível quando o serviço for hospedado em uma VPS com UDP liberado. O relay não persiste mensagens, membros ou chamadas, e o volume `/app/data` só preserva a identidade Ed25519 para que o `PeerId` não mude após reinício. Não coloque tokens, senhas ou certificados no repositório.

## Teste remoto do circuito

Depois que o serviço estiver online, execute o teste de dois clientes contra o relay público. Use um endereço IPv4 resolvido para o TCP Proxy e mantenha o `PeerId` do relay no multiaddr:

```powershell
$env:TEAMSCORD_TEST_RELAY_ADDRESS = "/ip4/RELAY_IP/tcp/PORTA_PUBLICA/p2p/PEER_ID_DO_RELAY"
cargo test --manifest-path infra/relay/Cargo.toml -- --ignored two_clients_exchange_connection_through_remote_relay
```

Esse teste valida handshake Noise, reserva dos dois circuitos e conexão entre os clientes através do TCP Proxy; ele não transporta mensagens de produção nem grava dados no relay.
