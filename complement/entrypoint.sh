#!/bin/sh
# Starts phantom the way Complement expects: named after $SERVER_NAME, with a
# federation certificate for that name signed by Complement's CA.
#
# Complement may run this more than once in the same container (it restarts
# servers in some tests), so everything here is safe to repeat: the
# certificate is simply issued again, and the database is kept.

set -eu

: "${SERVER_NAME:?Complement sets SERVER_NAME to the name this server answers to}"

ca=/complement/ca
out=/run/phantom

openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -subj "/CN=${SERVER_NAME}" -keyout "${out}/tls.key" -out "${out}/tls.csr" 2>/dev/null

printf 'subjectAltName=DNS:%s\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' \
    "${SERVER_NAME%%:*}" >"${out}/tls.ext"

if [ -f "${ca}/ca.crt" ] && [ -f "${ca}/ca.key" ]; then
    # A random serial rather than -CAcreateserial, which would write a serial
    # file next to the CA, and Complement's mount need not be writable.
    openssl x509 -req -in "${out}/tls.csr" -CA "${ca}/ca.crt" -CAkey "${ca}/ca.key" \
        -set_serial "0x$(openssl rand -hex 16)" -days 7 -extfile "${out}/tls.ext" \
        -out "${out}/tls.crt" 2>/dev/null
else
    # Outside Complement (running the image by hand) there is no CA; a
    # self-signed certificate keeps the federation port up.
    openssl x509 -req -in "${out}/tls.csr" -signkey "${out}/tls.key" -days 7 \
        -extfile "${out}/tls.ext" -out "${out}/tls.crt" 2>/dev/null
fi

# The option's value is a TOML string, hence the inner quotes.
exec /usr/local/bin/phantom-server --config /complement/phantom.toml \
    --option "server_name=\"${SERVER_NAME}\"" "$@"
