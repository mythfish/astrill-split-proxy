# Astrill Split Proxy

A small macOS desktop app that turns Astrill OpenWeb's browser-only tunnel into local HTTP and SOCKS5 proxies with rule-based routing.

It is built with Tauri, React, Ant Design, Rust, and a Python proxy worker.

## Features

- Detect Astrill OpenWeb's local upstream port.
- Start a local HTTP proxy and SOCKS5 proxy.
- Route traffic by domain rules through either direct or proxy paths.
- Toggle macOS system proxy settings for Wi-Fi.
- Optionally enable the macOS system proxy automatically after the local proxy starts.
- Write shell proxy config for zsh, bash, and fish.
- Monitor recent traffic and see which hosts used the proxy route.
- Expose a local OpenAI-compatible Gemini API gateway backed by a Google Gemini API key.
- Keep running from the macOS menu bar after closing the window.
- Optional login auto-start via LaunchAgent.
- Launch selected applications with proxy environment variables.

## How It Works

Astrill OpenWeb exposes a local browser tunnel. This app starts a local split proxy in front of that tunnel:

- HTTP proxy: `127.0.0.1:18080`
- SOCKS5 proxy: `127.0.0.1:18081`
- Astrill upstream default: `127.0.0.1:32768`

Rules decide whether a host goes direct or through the Astrill OpenWeb upstream. The default config proxies common global services and keeps local, LAN, and `.cn` traffic direct.

## Requirements

- macOS 11 or later
- Astrill with OpenWeb mode available
- Node.js and npm
- Rust toolchain
- Python 3

## Development

```bash
npm install
npm run dev
```

## Build

```bash
npm install
npm run build
```

The bundled app is created under:

```text
src-tauri/target/release/bundle/macos/Astrill Split Proxy.app
```

For local ad-hoc signing:

```bash
codesign --force --deep --sign - "src-tauri/target/release/bundle/macos/Astrill Split Proxy.app"
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/Astrill Split Proxy.app"
```

## Usage

1. Start Astrill and enable OpenWeb mode.
2. Open Astrill Split Proxy.
3. Click `检测` to detect the OpenWeb port.
4. Click `启动` to start the local split proxy.
5. Use the system proxy buttons, shell proxy buttons, or configure a browser/app to use `127.0.0.1:18080` / `127.0.0.1:18081`.

The `启动代理后自动开启系统代理` option can be enabled before starting the proxy. When enabled, starting the local proxy also turns on the macOS Wi-Fi system proxy.

## Application Proxy

The `应用代理` tab can add macOS `.app` bundles and launch them with proxy environment variables:

- `http_proxy`
- `https_proxy`
- `all_proxy`
- uppercase variants

For Chromium/Electron-style apps, it also passes:

```text
--proxy-server=http://127.0.0.1:18080
```

This only affects applications launched from Astrill Split Proxy. It does not transparently take over already-running applications. Some native apps may ignore proxy environment variables.

## Gemini API Gateway

The `Gemini API` tab starts a local OpenAI-compatible gateway for clients that can customize an API base URL.

Default local endpoint:

```text
http://127.0.0.1:18082/v1
```

Configure your Google Gemini API key in the app, choose an outbound path (`SplitProxy`, `Astrill`, `Custom`, or `Direct`), then start the gateway. `Custom` accepts an HTTP/HTTPS proxy URL such as `http://127.0.0.1:7890`.

```text
base_url = "http://127.0.0.1:18082/v1"
api_key = "any-non-empty-string"
model = "gemini-3.5-flash"
```

The local gateway injects the configured Google Gemini API key when forwarding requests to Google's OpenAI-compatible Gemini endpoint.

## Data Files

Runtime data is stored in:

```text
~/Library/Application Support/AstrillSplitProxy/
```

Important files include:

- `config.json`
- `traffic.jsonl`
- `app_proxy_apps.json`

Login auto-start writes a LaunchAgent at:

```text
~/Library/LaunchAgents/local.astrill-split-proxy.plist
```

## Disclaimer

This project is not affiliated with Astrill. Use it only with network services and accounts you are authorized to use, and follow applicable laws and service terms.

## License

MIT
