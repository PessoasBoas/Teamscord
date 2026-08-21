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
