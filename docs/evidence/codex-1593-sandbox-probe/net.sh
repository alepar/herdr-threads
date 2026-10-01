#!/bin/sh
# $1 probe dir, $2 daemon socket, $3 herdr server socket
python3 "$1/conn.py" "$2" "$1/neg.sock" "$3" tcp:127.0.0.1:47812 tcp:1.1.1.1:53 tcp:93.184.215.14:443
echo "env proxies: HTTP_PROXY=${HTTP_PROXY:+set} HTTPS_PROXY=${HTTPS_PROXY:+set} ALL_PROXY=${ALL_PROXY:+set}"
curl -sS -m 8 -o /dev/null -w 'curl https://example.com http=%{http_code}\n' https://example.com 2>&1 | head -3
curl -sS -m 8 --noproxy '*' -o /dev/null -w 'curl --noproxy example.com http=%{http_code}\n' https://example.com 2>&1 | head -3
