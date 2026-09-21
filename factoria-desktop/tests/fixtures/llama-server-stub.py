#!/usr/bin/env python3
"""Sustituto de `llama-server` para las pruebas de extremo a extremo.

Implementa el mismo contrato que usa el adaptador llama.cpp de FactorIA:

  GET  /health                  -> 200 cuando el "modelo" está cargado
  POST /v1/chat/completions     -> SSE con el formato `chat.completion.chunk`

Sirve para ejercitar **el código real** de FactorIA (lanzamiento del proceso
hijo, sondeo de salud, parseo SSE, cancelación, métricas). Lo único que
sustituye es llama.cpp: no hay inferencia, se devuelve un texto fijo token a
token. Ver `docs/VALIDATION.md`.
"""

import argparse
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Retardo por token: suficiente para que la interfaz muestre el streaming y
# para que la prueba de "detener" tenga tiempo de interrumpir.
TOKEN_DELAY_S = 0.06

RESPUESTA = (
    "Claro. Estoy respondiendo desde tu propio equipo, sin enviar nada fuera.\n\n"
    "- El modelo se ejecuta en un proceso local.\n"
    "- La conversación se guarda solo en este disco.\n"
    "- Puedes detener o regenerar la respuesta cuando quieras.\n"
)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        pass  # el ruido de acceso no aporta nada a la prueba

    def do_GET(self):
        if self.path.startswith("/health"):
            body = b'{"status":"ok"}'
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self.send_error(404)
            return

        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            payload = json.loads(raw)
        except json.JSONDecodeError:
            payload = {}
        model = payload.get("model", "stub")

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

        def emit(obj):
            self.wfile.write(f"data: {json.dumps(obj)}\n\n".encode())
            self.wfile.flush()

        try:
            for token in RESPUESTA.split(" "):
                emit(
                    {
                        "id": "chatcmpl-stub",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [{"index": 0, "delta": {"content": token + " "}}],
                    }
                )
                time.sleep(TOKEN_DELAY_S)
            emit({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            # El cliente canceló: es exactamente lo que prueba "Detener".
            pass


def main():
    # llama-server recibe muchos argumentos; la prueba solo necesita el puerto.
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--model", default="")
    args, _unknown = parser.parse_known_args()

    print(f"stub pid={os.getpid()} arrancando puerto={args.port} t={time.time():.3f}", file=sys.stderr, flush=True)
    # Simula la carga de pesos: FactorIA debe esperar a /health.
    time.sleep(0.35)

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"stub pid={os.getpid()} escuchando en {args.host}:{args.port} t={time.time():.3f}", file=sys.stderr, flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
