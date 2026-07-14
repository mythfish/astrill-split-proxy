#!/usr/bin/env python3
"""
Local Gemini API gateway.

It exposes an OpenAI-compatible local endpoint and forwards requests to
Google's Gemini OpenAI compatibility API with the configured Gemini API key.
"""

from __future__ import annotations

import argparse
import json
import logging
import ssl
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPSHandler, ProxyHandler, Request, build_opener


GOOGLE_OPENAI_BASE = "https://generativelanguage.googleapis.com/v1beta/openai"
HOP_BY_HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


def load_raw_config(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def gemini_config(raw: dict[str, Any]) -> dict[str, Any]:
    gemini = raw.get("gemini") or {}
    return {
        "host": str(gemini.get("host", "127.0.0.1")),
        "port": int(gemini.get("port", 18082)),
        "api_key": str(gemini.get("api_key", "")),
        "upstream": str(gemini.get("upstream", "split_proxy")),
    }


def proxy_url_for(raw: dict[str, Any], upstream: str) -> str | None:
    if upstream == "direct":
        return None
    if upstream == "astrill":
        port = int((raw.get("upstream") or {}).get("port", 32768))
        return f"http://127.0.0.1:{port}"
    listen = raw.get("listen") or {}
    port = int(listen.get("http_port", 18080))
    return f"http://127.0.0.1:{port}"


def google_path(path: str) -> str:
    parsed = urlsplit(path)
    raw_path = parsed.path.rstrip("/")
    query = f"?{parsed.query}" if parsed.query else ""
    if raw_path == "/v1":
        suffix = ""
    elif raw_path.startswith("/v1/"):
        suffix = raw_path[3:]
    elif raw_path == "/v1beta/openai":
        suffix = ""
    elif raw_path.startswith("/v1beta/openai/"):
        suffix = raw_path[len("/v1beta/openai") :]
    else:
        suffix = raw_path
    return f"{GOOGLE_OPENAI_BASE}{suffix}{query}"


def response_json(handler: BaseHTTPRequestHandler, status: int, payload: dict[str, Any]) -> None:
    data = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json; charset=utf-8")
    handler.send_header("Content-Length", str(len(data)))
    handler.end_headers()
    handler.wfile.write(data)


class GeminiGatewayHandler(BaseHTTPRequestHandler):
    server_version = "AstrillSplitProxyGemini/1.0"

    def log_message(self, fmt: str, *args: Any) -> None:
        logging.info("%s - %s", self.address_string(), fmt % args)

    @property
    def gateway(self) -> "GeminiGatewayServer":
        return self.server  # type: ignore[return-value]

    def do_GET(self) -> None:
        if self.path in {"/", "/health", "/healthz"}:
            response_json(
                self,
                200,
                {
                    "ok": True,
                    "service": "gemini-api",
                    "base_url": f"http://{self.gateway.listen_host}:{self.gateway.listen_port}/v1",
                },
            )
            return
        self.forward()

    def do_POST(self) -> None:
        self.forward()

    def do_OPTIONS(self) -> None:
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Headers", "authorization,content-type")
        self.send_header("Access-Control-Allow-Methods", "GET,POST,OPTIONS")
        self.end_headers()

    def forward(self) -> None:
        api_key = self.gateway.api_key
        if not api_key:
            response_json(self, 400, {"error": "Gemini API key is not configured."})
            return

        length = int(self.headers.get("Content-Length", "0") or "0")
        body = self.rfile.read(length) if length else None
        headers = {}
        for key, value in self.headers.items():
            lower = key.lower()
            if lower in HOP_BY_HOP_HEADERS or lower in {"host", "content-length", "authorization", "accept-encoding"}:
                continue
            headers[key] = value
        headers["Authorization"] = f"Bearer {api_key}"
        headers["Accept-Encoding"] = "identity"

        request = Request(
            google_path(self.path),
            data=body,
            headers=headers,
            method=self.command,
        )
        try:
            with self.gateway.opener.open(request, timeout=120) as response:
                self.send_response(response.status)
                self.send_forward_headers(response.headers.items())
                self.end_headers()
                while True:
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    self.wfile.write(chunk)
                    self.wfile.flush()
        except HTTPError as error:
            self.send_response(error.code)
            self.send_forward_headers(error.headers.items())
            self.end_headers()
            self.wfile.write(error.read())
        except (OSError, URLError) as error:
            response_json(self, 502, {"error": str(error)})

    def send_forward_headers(self, headers: Any) -> None:
        self.send_header("Access-Control-Allow-Origin", "*")
        for key, value in headers:
            lower = key.lower()
            if lower in HOP_BY_HOP_HEADERS or lower in {"content-length", "content-encoding"}:
                continue
            self.send_header(key, value)


class GeminiGatewayServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address: tuple[str, int], api_key: str, proxy_url: str | None):
        super().__init__(address, GeminiGatewayHandler)
        self.listen_host = address[0]
        self.listen_port = address[1]
        self.api_key = api_key
        self.ssl_context = ssl.create_default_context()
        if proxy_url:
            self.opener = build_opener(
                ProxyHandler({"http": proxy_url, "https": proxy_url}),
                HTTPSHandler(context=self.ssl_context),
            )
        else:
            self.opener = build_opener(ProxyHandler({}), HTTPSHandler(context=self.ssl_context))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("-c", "--config", required=True, type=Path)
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    raw = load_raw_config(args.config)
    config = gemini_config(raw)
    proxy_url = proxy_url_for(raw, config["upstream"])
    server = GeminiGatewayServer((config["host"], config["port"]), config["api_key"], proxy_url)
    logging.info(
        "Gemini API gateway listening on %s:%s upstream=%s",
        config["host"],
        config["port"],
        config["upstream"],
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        return 0
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
