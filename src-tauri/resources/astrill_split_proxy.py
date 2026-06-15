#!/usr/bin/env python3
"""
Local split proxy for Astrill OpenWeb "tunnel browser only" mode.

It exposes a local HTTP proxy and a SOCKS5 proxy. For each destination host it
chooses either DIRECT or the Astrill OpenWeb HTTP proxy upstream.
"""

from __future__ import annotations

import argparse
import asyncio
import ipaddress
import json
import logging
import signal
import socket
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import urlsplit


BUFFER_SIZE = 65536
HEADER_LIMIT = 1024 * 1024


@dataclass(frozen=True)
class ListenConfig:
    host: str
    port: int


@dataclass(frozen=True)
class UpstreamConfig:
    host: str
    port: int


@dataclass(frozen=True)
class Config:
    http_listen: ListenConfig
    socks_listen: ListenConfig
    upstream: UpstreamConfig
    default_route: str
    proxy_rules: list[str]
    direct_rules: list[str]
    log_level: str
    traffic_log: Path


def load_config(path: Path) -> Config:
    raw = json.loads(path.read_text(encoding="utf-8"))
    listen = raw.get("listen", {})
    upstream = raw.get("upstream", {})
    rules = raw.get("rules", {})
    return Config(
        http_listen=ListenConfig(
            str(listen.get("http_host", "127.0.0.1")),
            int(listen.get("http_port", 18080)),
        ),
        socks_listen=ListenConfig(
            str(listen.get("socks_host", "127.0.0.1")),
            int(listen.get("socks_port", 18081)),
        ),
        upstream=UpstreamConfig(
            str(upstream.get("host", "127.0.0.1")),
            int(upstream.get("port", 32768)),
        ),
        default_route=str(raw.get("default_route", "direct")).lower(),
        proxy_rules=[str(item).lower() for item in rules.get("proxy", [])],
        direct_rules=[str(item).lower() for item in rules.get("direct", [])],
        log_level=str(raw.get("log_level", "info")).upper(),
        traffic_log=path.parent / "traffic.jsonl",
    )


def normalize_host(host: str) -> str:
    host = host.strip().lower().strip(".")
    if host.startswith("[") and host.endswith("]"):
        return host[1:-1]
    return host


def host_matches_rule(host: str, rule: str) -> bool:
    host = normalize_host(host)
    rule = rule.strip().lower()
    if not rule:
        return False
    if rule == "*":
        return True
    if rule.startswith("*."):
        rule = rule[2:]
    if rule.startswith("."):
        rule = rule[1:]

    try:
        host_ip = ipaddress.ip_address(host)
        if "/" in rule:
            return host_ip in ipaddress.ip_network(rule, strict=False)
        return host_ip == ipaddress.ip_address(rule)
    except ValueError:
        pass

    return host == rule or host.endswith("." + rule)


def choose_route(host: str, config: Config) -> str:
    for rule in config.direct_rules:
        if host_matches_rule(host, rule):
            return "direct"
    for rule in config.proxy_rules:
        if host_matches_rule(host, rule):
            return "proxy"
    if config.default_route not in {"direct", "proxy"}:
        raise ValueError("default_route must be 'direct' or 'proxy'")
    return config.default_route


def write_traffic_event(
    config: Config,
    protocol: str,
    route: str,
    host: str,
    port: int,
    peer: object,
    method: str = "",
) -> None:
    event = {
        "ts": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "protocol": protocol,
        "method": method,
        "route": route,
        "host": normalize_host(host),
        "port": port,
        "peer": str(peer),
    }
    try:
        config.traffic_log.parent.mkdir(parents=True, exist_ok=True)
        with config.traffic_log.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(event, ensure_ascii=False, separators=(",", ":")) + "\n")
    except OSError:
        logging.debug("failed to write traffic event", exc_info=True)


def split_host_port(value: str, default_port: int) -> tuple[str, int]:
    value = value.strip()
    if value.startswith("["):
        host, _, rest = value[1:].partition("]")
        port = int(rest[1:]) if rest.startswith(":") else default_port
        return host, port
    if value.count(":") == 1:
        host, port = value.rsplit(":", 1)
        if port.isdigit():
            return host, int(port)
    return value, default_port


def parse_host_header(headers: bytes) -> str | None:
    for line in headers.split(b"\r\n")[1:]:
        key, sep, value = line.partition(b":")
        if sep and key.lower() == b"host":
            return value.decode("latin-1", errors="replace").strip()
    return None


async def open_direct(host: str, port: int) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    return await asyncio.open_connection(host, port)


async def open_via_upstream_connect(
    host: str,
    port: int,
    config: Config,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter, bytes]:
    reader, writer = await asyncio.open_connection(config.upstream.host, config.upstream.port)
    request = (
        f"CONNECT {host}:{port} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Proxy-Connection: Keep-Alive\r\n\r\n"
    ).encode("ascii", errors="ignore")
    writer.write(request)
    await writer.drain()
    response = await reader.readuntil(b"\r\n\r\n")
    status_line = response.split(b"\r\n", 1)[0].decode("latin-1", errors="replace")
    if " 200 " not in status_line and not status_line.endswith(" 200"):
        writer.close()
        await writer.wait_closed()
        raise OSError(f"upstream CONNECT failed: {status_line}")
    return reader, writer, response


async def pipe(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
    try:
        while True:
            data = await reader.read(BUFFER_SIZE)
            if not data:
                break
            writer.write(data)
            await writer.drain()
    except (ConnectionError, asyncio.IncompleteReadError, OSError):
        pass
    finally:
        try:
            writer.write_eof()
        except (OSError, RuntimeError):
            pass


async def tunnel(
    left_reader: asyncio.StreamReader,
    left_writer: asyncio.StreamWriter,
    right_reader: asyncio.StreamReader,
    right_writer: asyncio.StreamWriter,
) -> None:
    tasks = [
        asyncio.create_task(pipe(left_reader, right_writer)),
        asyncio.create_task(pipe(right_reader, left_writer)),
    ]
    await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
    for task in tasks:
        task.cancel()
    for writer in (left_writer, right_writer):
        writer.close()
    await asyncio.gather(
        left_writer.wait_closed(),
        right_writer.wait_closed(),
        return_exceptions=True,
    )


async def send_http_error(writer: asyncio.StreamWriter, status: str, detail: str = "") -> None:
    body = detail.encode("utf-8", errors="replace")
    writer.write(
        (
            f"HTTP/1.1 {status}\r\n"
            "Connection: close\r\n"
            "Content-Type: text/plain; charset=utf-8\r\n"
            f"Content-Length: {len(body)}\r\n\r\n"
        ).encode("ascii")
        + body
    )
    await writer.drain()
    writer.close()
    await writer.wait_closed()


def rewrite_absolute_request(first_line: str, header_block: bytes) -> bytes:
    method, target, version = first_line.split(" ", 2)
    parsed = urlsplit(target)
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    new_first = f"{method} {path} {version}\r\n".encode("latin-1")
    rest = header_block.split(b"\r\n", 1)[1]
    return new_first + rest


async def handle_http_client(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    config: Config,
) -> None:
    peer = writer.get_extra_info("peername")
    try:
        headers = await reader.readuntil(b"\r\n\r\n")
        if len(headers) > HEADER_LIMIT:
            await send_http_error(writer, "431 Request Header Fields Too Large")
            return
        first_line = headers.split(b"\r\n", 1)[0].decode("latin-1", errors="replace")
        parts = first_line.split(" ")
        if len(parts) < 3:
            await send_http_error(writer, "400 Bad Request", "invalid request line")
            return
        method, target, _version = parts[0].upper(), parts[1], parts[2]

        is_connect = method == "CONNECT"
        if is_connect:
            host, port = split_host_port(target, 443)
        else:
            parsed = urlsplit(target)
            if parsed.scheme and parsed.hostname:
                host = parsed.hostname
                port = parsed.port or (443 if parsed.scheme == "https" else 80)
            else:
                host_header = parse_host_header(headers)
                if not host_header:
                    await send_http_error(writer, "400 Bad Request", "missing Host header")
                    return
                host, port = split_host_port(host_header, 80)

        route = choose_route(host, config)
        logging.info("http %-6s %-5s %s:%s peer=%s", method, route, host, port, peer)
        write_traffic_event(config, "http", route, host, port, peer, method)

        if is_connect:
            if route == "proxy":
                upstream_reader, upstream_writer, _ = await open_via_upstream_connect(host, port, config)
            else:
                upstream_reader, upstream_writer = await open_direct(host, port)
            writer.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
            await writer.drain()
            await tunnel(reader, writer, upstream_reader, upstream_writer)
            return

        if route == "proxy":
            upstream_reader, upstream_writer = await open_direct(config.upstream.host, config.upstream.port)
            upstream_writer.write(headers)
        else:
            upstream_reader, upstream_writer = await open_direct(host, port)
            if "://" in target:
                upstream_writer.write(rewrite_absolute_request(first_line, headers))
            else:
                upstream_writer.write(headers)
        await upstream_writer.drain()
        await tunnel(reader, writer, upstream_reader, upstream_writer)
    except (asyncio.IncompleteReadError, ConnectionError, OSError) as exc:
        logging.debug("http client failed: %r", exc)
        if not writer.is_closing():
            await send_http_error(writer, "502 Bad Gateway", str(exc))
    except Exception:
        logging.exception("unexpected HTTP proxy error")
        if not writer.is_closing():
            await send_http_error(writer, "500 Internal Server Error")


async def socks_read_address(reader: asyncio.StreamReader) -> tuple[str, int]:
    atyp = (await reader.readexactly(1))[0]
    if atyp == 0x01:
        raw = await reader.readexactly(4)
        host = socket.inet_ntop(socket.AF_INET, raw)
    elif atyp == 0x03:
        size = (await reader.readexactly(1))[0]
        host = (await reader.readexactly(size)).decode("ascii", errors="replace")
    elif atyp == 0x04:
        raw = await reader.readexactly(16)
        host = socket.inet_ntop(socket.AF_INET6, raw)
    else:
        raise OSError(f"unsupported SOCKS address type {atyp}")
    port = int.from_bytes(await reader.readexactly(2), "big")
    return host, port


async def socks_reply(writer: asyncio.StreamWriter, code: int) -> None:
    writer.write(b"\x05" + bytes([code]) + b"\x00\x01\x00\x00\x00\x00\x00\x00")
    await writer.drain()


async def handle_socks_client(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    config: Config,
) -> None:
    peer = writer.get_extra_info("peername")
    try:
        version = (await reader.readexactly(1))[0]
        if version != 0x05:
            writer.close()
            await writer.wait_closed()
            return
        method_count = (await reader.readexactly(1))[0]
        methods = await reader.readexactly(method_count)
        if 0x00 not in methods:
            writer.write(b"\x05\xff")
            await writer.drain()
            writer.close()
            await writer.wait_closed()
            return
        writer.write(b"\x05\x00")
        await writer.drain()

        request_head = await reader.readexactly(3)
        if request_head[0] != 0x05 or request_head[1] != 0x01:
            await socks_reply(writer, 0x07)
            writer.close()
            await writer.wait_closed()
            return
        host, port = await socks_read_address(reader)
        route = choose_route(host, config)
        logging.info("socks %-6s %s:%s peer=%s", route, host, port, peer)
        write_traffic_event(config, "socks5", route, host, port, peer, "CONNECT")

        try:
            if route == "proxy":
                upstream_reader, upstream_writer, _ = await open_via_upstream_connect(host, port, config)
            else:
                upstream_reader, upstream_writer = await open_direct(host, port)
        except OSError:
            await socks_reply(writer, 0x05)
            raise

        await socks_reply(writer, 0x00)
        await tunnel(reader, writer, upstream_reader, upstream_writer)
    except (asyncio.IncompleteReadError, ConnectionError, OSError) as exc:
        logging.debug("socks client failed: %r", exc)
        if not writer.is_closing():
            writer.close()
            await writer.wait_closed()
    except Exception:
        logging.exception("unexpected SOCKS proxy error")
        if not writer.is_closing():
            writer.close()
            await writer.wait_closed()


async def serve(config: Config) -> None:
    http_server = await asyncio.start_server(
        lambda r, w: handle_http_client(r, w, config),
        config.http_listen.host,
        config.http_listen.port,
    )
    socks_server = await asyncio.start_server(
        lambda r, w: handle_socks_client(r, w, config),
        config.socks_listen.host,
        config.socks_listen.port,
    )
    logging.warning(
        "HTTP proxy on %s:%s, SOCKS5 on %s:%s, Astrill upstream %s:%s, default=%s",
        config.http_listen.host,
        config.http_listen.port,
        config.socks_listen.host,
        config.socks_listen.port,
        config.upstream.host,
        config.upstream.port,
        config.default_route,
    )

    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for signame in ("SIGINT", "SIGTERM"):
        try:
            loop.add_signal_handler(getattr(signal, signame), stop.set)
        except NotImplementedError:
            pass

    async with http_server, socks_server:
        await stop.wait()


def main() -> int:
    parser = argparse.ArgumentParser(description="Astrill OpenWeb split proxy")
    parser.add_argument(
        "-c",
        "--config",
        type=Path,
        default=Path(__file__).with_name("config.json"),
        help="path to config.json",
    )
    args = parser.parse_args()

    config = load_config(args.config)
    logging.basicConfig(
        level=getattr(logging, config.log_level, logging.INFO),
        format="%(asctime)s %(levelname)s %(message)s",
    )
    try:
        asyncio.run(serve(config))
    except KeyboardInterrupt:
        return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
