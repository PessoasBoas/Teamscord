# Coturn opcional

Este pacote fornece um fallback TURN para calls WebRTC fora da LAN. O relay libp2p em `infra/relay` continua sendo usado para descoberta e transporte dos nodes; Coturn não participa do chat nem recebe mensagens persistidas.

## Configuração

1. Copie `.env.example` para `.env` e substitua todos os valores, principalmente `TURN_EXTERNAL_IP`, usuário e senha.
2. Coloque um certificado TLS válido em `infra/turn/certs/fullchain.pem` e `infra/turn/certs/privkey.pem`. Nunca versione essa pasta.
3. Libere no firewall UDP/TCP 3478, UDP/TCP 5349 e a faixa UDP 49152–49252; encaminhe as mesmas portas no roteador/VPS.
4. Execute `docker compose up -d --build` dentro desta pasta.
5. No Teamscord, configure em `get_media_config` um endereço `turns:turn.example.com:5349?transport=tcp` com as credenciais guardadas no Windows Credential Manager. Adicione também um `stun:` se possuir um servidor STUN próprio.

O aplicativo não embute servidor STUN/TURN, não coloca credenciais em convites e tenta conexão direta antes do fallback configurado. Gere credenciais longas e rotacione-as periodicamente; para produção, prefira credenciais temporárias emitidas por um serviço de autenticação em vez de uma senha estática.

O Docker não é requisito para compilar ou usar o Teamscord: sem TURN configurado, a interface informa que a call só conseguiu (ou só tentará) conexão direta.
