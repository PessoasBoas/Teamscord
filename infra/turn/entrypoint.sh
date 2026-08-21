#!/bin/sh
set -eu

required_vars="TURN_REALM TURN_USERNAME TURN_PASSWORD TURN_EXTERNAL_IP TURN_MIN_PORT TURN_MAX_PORT TURN_TLS_CERT TURN_TLS_KEY"
for variable in $required_vars; do
  eval "value=\${$variable:-}"
  if [ -z "$value" ]; then
    echo "variável obrigatória ausente: $variable" >&2
    exit 1
  fi
  case "$value" in
    *[![:print:]]*)
      echo "variável contém caractere não imprimível: $variable" >&2
      exit 1
      ;;
  esac
done

escape_sed_value() {
  printf '%s' "$1" | sed 's/[\\&|]/\\&/g'
}

template=/etc/coturn/turnserver.conf
config=/tmp/turnserver.conf
realm=$(escape_sed_value "$TURN_REALM")
username=$(escape_sed_value "$TURN_USERNAME")
password=$(escape_sed_value "$TURN_PASSWORD")
external_ip=$(escape_sed_value "$TURN_EXTERNAL_IP")
min_port=$(escape_sed_value "$TURN_MIN_PORT")
max_port=$(escape_sed_value "$TURN_MAX_PORT")
tls_cert=$(escape_sed_value "$TURN_TLS_CERT")
tls_key=$(escape_sed_value "$TURN_TLS_KEY")
sed \
  -e "s|\${TURN_REALM}|${realm}|g" \
  -e "s|\${TURN_USERNAME}|${username}|g" \
  -e "s|\${TURN_PASSWORD}|${password}|g" \
  -e "s|\${TURN_EXTERNAL_IP}|${external_ip}|g" \
  -e "s|\${TURN_MIN_PORT}|${min_port}|g" \
  -e "s|\${TURN_MAX_PORT}|${max_port}|g" \
  -e "s|\${TURN_TLS_CERT}|${tls_cert}|g" \
  -e "s|\${TURN_TLS_KEY}|${tls_key}|g" \
  "$template" > "$config"

exec turnserver -c "$config"
