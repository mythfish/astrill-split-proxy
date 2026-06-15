use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};

const SHELL_START: &str = "# >>> AstrillSplitProxy";
const SHELL_END: &str = "# <<< AstrillSplitProxy";
const LAUNCH_AGENT_ID: &str = "local.astrill-split-proxy";

#[derive(Default)]
struct AppState {
    proxy: Mutex<Option<Child>>,
    logs: Mutex<Vec<String>>,
}

#[derive(Serialize)]
struct Status {
    running: bool,
    http_port: u16,
    socks_port: u16,
    upstream_port: u16,
    system_proxy: String,
    shell_proxy: String,
}

#[derive(Deserialize)]
struct SaveRequest {
    http_port: u16,
    socks_port: u16,
    upstream_port: u16,
    default_route: String,
    auto_system_proxy: bool,
    proxy_rules: Vec<String>,
    direct_rules: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct AppProxyEntry {
    id: String,
    name: String,
    path: String,
}

fn app_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|p| p.join("Library/Application Support/AstrillSplitProxy"))
        .ok_or_else(|| "home directory not found".to_string())
}

fn config_path() -> Result<PathBuf, String> {
    Ok(app_dir()?.join("config.json"))
}

fn traffic_log_path() -> Result<PathBuf, String> {
    Ok(app_dir()?.join("traffic.jsonl"))
}

fn app_proxy_list_path() -> Result<PathBuf, String> {
    Ok(app_dir()?.join("app_proxy_apps.json"))
}

fn launch_agent_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|p| p.join(format!("Library/LaunchAgents/{LAUNCH_AGENT_ID}.plist")))
        .ok_or_else(|| "home directory not found".to_string())
}

fn ensure_config(app: &AppHandle) -> Result<(), String> {
    let dir = app_dir()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = config_path()?;
    if !path.exists() {
        let default_path = app
            .path()
            .resolve(
                "resources/default_config.json",
                tauri::path::BaseDirectory::Resource,
            )
            .map_err(|e| e.to_string())?;
        fs::copy(default_path, path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn read_config_value(app: &AppHandle) -> Result<Value, String> {
    ensure_config(app)?;
    let data = fs::read_to_string(config_path()?).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

fn read_app_proxy_entries() -> Result<Vec<AppProxyEntry>, String> {
    let path = app_proxy_list_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

fn write_app_proxy_entries(entries: &[AppProxyEntry]) -> Result<(), String> {
    fs::create_dir_all(app_dir()?).map_err(|e| e.to_string())?;
    fs::write(
        app_proxy_list_path()?,
        serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn port_from_config(config: &Value, group: &str, key: &str, fallback: u16) -> u16 {
    config
        .get(group)
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_u64())
        .map(|v| v as u16)
        .unwrap_or(fallback)
}

fn bool_from_config(config: &Value, key: &str, fallback: bool) -> bool {
    config.get(key).and_then(Value::as_bool).unwrap_or(fallback)
}

fn push_log(app: &AppHandle, state: &State<Arc<AppState>>, message: impl Into<String>) {
    let message = message.into();
    if let Ok(mut logs) = state.logs.lock() {
        logs.push(message.clone());
        if logs.len() > 500 {
            logs.drain(0..100);
        }
    }
    let _ = app.emit("log", message);
}

fn command_output(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => {
                let output = child.wait_with_output().map_err(|e| e.to_string())?;
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} timed out after {}s", timeout.as_secs()));
            }
            None => thread::sleep(Duration::from_millis(40)),
        }
    }
}

fn command_output_or_empty(program: &str, args: &[&str], timeout_secs: u64) -> String {
    command_output(program, args, Duration::from_secs(timeout_secs)).unwrap_or_default()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn user_id() -> String {
    command_output_or_empty("/usr/bin/id", &["-u"], 2)
        .trim()
        .to_string()
}

fn app_name_from_path(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.trim_end_matches(".app").to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn app_executable(path: &str) -> Result<PathBuf, String> {
    let bundle = PathBuf::from(path);
    let info = bundle.join("Contents/Info.plist");
    let executable = command_output(
        "/usr/bin/plutil",
        &[
            "-extract",
            "CFBundleExecutable",
            "raw",
            "-o",
            "-",
            info.to_str()
                .ok_or_else(|| "invalid app path".to_string())?,
        ],
        Duration::from_secs(3),
    )
    .map(|value| value.trim().to_string())
    .ok()
    .filter(|value| !value.is_empty())
    .unwrap_or_else(|| app_name_from_path(path));
    let path = bundle.join("Contents/MacOS").join(executable);
    if path.exists() {
        Ok(path)
    } else {
        Err("没有找到应用可执行文件".into())
    }
}

fn login_item_plist(exe: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--background</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
</dict>
</plist>
"#,
        xml_escape(LAUNCH_AGENT_ID),
        xml_escape(exe)
    )
}

fn system_proxy_summary(service: &str) -> String {
    let checks = [
        command_output_or_empty("/usr/sbin/networksetup", &["-getwebproxy", service], 4),
        command_output_or_empty(
            "/usr/sbin/networksetup",
            &["-getsecurewebproxy", service],
            4,
        ),
        command_output_or_empty(
            "/usr/sbin/networksetup",
            &["-getsocksfirewallproxy", service],
            4,
        ),
    ];
    let enabled = checks.iter().filter(|s| s.contains("Enabled: Yes")).count();
    let disabled = checks.iter().filter(|s| s.contains("Enabled: No")).count();
    if enabled == 3 {
        "已开启".to_string()
    } else if disabled == 3 {
        "已关闭".to_string()
    } else {
        "部分开启".to_string()
    }
}

fn detect_astrill_port_sync() -> Result<Option<u16>, String> {
    let pids = command_output("/usr/bin/pgrep", &["-x", "openweb"], Duration::from_secs(2))
        .unwrap_or_default();
    for pid in pids.lines().map(str::trim).filter(|pid| !pid.is_empty()) {
        let line = command_output(
            "/bin/ps",
            &["-p", pid, "-o", "command="],
            Duration::from_secs(2),
        )?;
        let parts: Vec<&str> = line.split_whitespace().collect();
        for index in 0..parts.len().saturating_sub(1) {
            if parts[index] == "--proxy-port" {
                if let Ok(port) = parts[index + 1].parse::<u16>() {
                    return Ok(Some(port));
                }
            }
        }
    }
    Ok(None)
}

fn shell_paths() -> Result<Vec<(String, PathBuf)>, String> {
    let home = dirs::home_dir().ok_or_else(|| "home directory not found".to_string())?;
    Ok(vec![
        ("zsh".into(), home.join(".zprofile")),
        ("zsh".into(), home.join(".zshrc")),
        ("bash".into(), home.join(".bash_profile")),
        ("bash".into(), home.join(".bashrc")),
        ("fish".into(), home.join(".config/fish/config.fish")),
    ])
}

fn shell_configured() -> String {
    let Ok(paths) = shell_paths() else {
        return "未知".to_string();
    };
    let count = paths
        .iter()
        .filter(|(_, path)| {
            fs::read_to_string(path)
                .map(|text| text.contains(SHELL_START) && text.contains(SHELL_END))
                .unwrap_or(false)
        })
        .count();
    if count == paths.len() {
        "已配置".into()
    } else if count == 0 {
        "未配置".into()
    } else {
        "部分配置".into()
    }
}

fn remove_managed_block(text: &str) -> String {
    let Some(start) = text.find(SHELL_START) else {
        return text.trim_end().to_string();
    };
    let Some(end_rel) = text[start..].find(SHELL_END) else {
        return text.trim_end().to_string();
    };
    let end = start + end_rel + SHELL_END.len();
    let mut result = String::new();
    result.push_str(text[..start].trim_end());
    result.push('\n');
    result.push_str(text[end..].trim_start());
    result.trim().to_string()
}

fn shell_block(shell: &str, http_port: u16, socks_port: u16) -> String {
    if shell == "fish" {
        format!(
            r#"{SHELL_START}
set -gx http_proxy "http://127.0.0.1:{http_port}"
set -gx https_proxy "http://127.0.0.1:{http_port}"
set -gx all_proxy "socks5h://127.0.0.1:{socks_port}"
set -gx HTTP_PROXY $http_proxy
set -gx HTTPS_PROXY $https_proxy
set -gx ALL_PROXY $all_proxy
set -q no_proxy; or set -gx no_proxy "localhost,127.0.0.1,::1"
set -q NO_PROXY; or set -gx NO_PROXY $no_proxy
{SHELL_END}
"#
        )
    } else {
        format!(
            r#"{SHELL_START}
export http_proxy="http://127.0.0.1:{http_port}"
export https_proxy="http://127.0.0.1:{http_port}"
export all_proxy="socks5h://127.0.0.1:{socks_port}"
export HTTP_PROXY="$http_proxy"
export HTTPS_PROXY="$https_proxy"
export ALL_PROXY="$all_proxy"
export no_proxy="localhost,127.0.0.1,::1"
export NO_PROXY="$no_proxy"
{SHELL_END}
"#
        )
    }
}

fn apply_system_proxy(service: &str, http_port: u16, socks_port: u16, enabled: bool) {
    let http_port = http_port.to_string();
    let socks_port = socks_port.to_string();
    if enabled {
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setwebproxy", service, "127.0.0.1", &http_port])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsecurewebproxy", service, "127.0.0.1", &http_port])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsocksfirewallproxy", service, "127.0.0.1", &socks_port])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setwebproxystate", service, "on"])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsecurewebproxystate", service, "on"])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsocksfirewallproxystate", service, "on"])
            .status();
    } else {
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setwebproxystate", service, "off"])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsecurewebproxystate", service, "off"])
            .status();
        let _ = Command::new("/usr/sbin/networksetup")
            .args(["-setsocksfirewallproxystate", service, "off"])
            .status();
    }
}

#[tauri::command]
async fn load_config(app: AppHandle) -> Result<Value, String> {
    read_config_value(&app)
}

#[tauri::command]
async fn save_config(app: AppHandle, req: SaveRequest) -> Result<(), String> {
    ensure_config(&app)?;
    let config = json!({
        "listen": {
            "http_host": "127.0.0.1",
            "http_port": req.http_port,
            "socks_host": "127.0.0.1",
            "socks_port": req.socks_port
        },
        "upstream": {
            "host": "127.0.0.1",
            "port": req.upstream_port
        },
        "default_route": req.default_route,
        "auto_system_proxy": req.auto_system_proxy,
        "rules": {
            "proxy": req.proxy_rules,
            "direct": req.direct_rules
        },
        "log_level": "info"
    });
    fs::write(
        config_path()?,
        serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_status(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Status, String> {
    let config = read_config_value(&app)?;
    let running = {
        let mut child = state.proxy.lock().map_err(|e| e.to_string())?;
        match child.as_mut() {
            Some(process) => match process.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) | Err(_) => {
                    *child = None;
                    false
                }
            },
            None => false,
        }
    };
    Ok(Status {
        running,
        http_port: port_from_config(&config, "listen", "http_port", 18080),
        socks_port: port_from_config(&config, "listen", "socks_port", 18081),
        upstream_port: port_from_config(&config, "upstream", "port", 32768),
        system_proxy: system_proxy_summary("Wi-Fi"),
        shell_proxy: shell_configured(),
    })
}

#[tauri::command]
async fn detect_astrill_port(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<u16>, String> {
    push_log(&app, &state, "Detecting Astrill OpenWeb port...");
    let port = tauri::async_runtime::spawn_blocking(detect_astrill_port_sync)
        .await
        .map_err(|e| e.to_string())??;
    if let Some(port) = port {
        push_log(
            &app,
            &state,
            format!("Detected Astrill OpenWeb port: {port}"),
        );
    } else {
        push_log(&app, &state, "Astrill OpenWeb process was not found.");
    }
    Ok(port)
}

#[tauri::command]
async fn start_proxy(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let config_value = read_config_value(&app)?;
    let http_port = port_from_config(&config_value, "listen", "http_port", 18080);
    let socks_port = port_from_config(&config_value, "listen", "socks_port", 18081);
    let auto_system_proxy = bool_from_config(&config_value, "auto_system_proxy", false);
    let mut guard = state.proxy.lock().map_err(|e| e.to_string())?;
    if let Some(process) = guard.as_mut() {
        match process.try_wait() {
            Ok(None) => {
                push_log(&app, &state, "Proxy is already running.");
                return Ok(());
            }
            Ok(Some(_)) | Err(_) => {
                *guard = None;
            }
        }
    }
    let script = app
        .path()
        .resolve(
            "resources/astrill_split_proxy.py",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| e.to_string())?;
    let config = config_path()?;
    let mut child = Command::new("/usr/bin/python3")
        .arg(script)
        .arg("-c")
        .arg(config)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut out) = child.stdout.take() {
        let app_clone = app.clone();
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = out.read_to_string(&mut text);
            if !text.is_empty() {
                let _ = app_clone.emit("log", text);
            }
        });
    }
    *guard = Some(child);
    drop(guard);
    push_log(&app, &state, "Proxy started.");
    if auto_system_proxy {
        apply_system_proxy("Wi-Fi", http_port, socks_port, true);
        push_log(&app, &state, "System proxy enabled automatically.");
    }
    Ok(())
}

#[tauri::command]
async fn stop_proxy(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let mut guard = state.proxy.lock().map_err(|e| e.to_string())?;
    if let Some(child) = guard.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    *guard = None;
    push_log(&app, &state, "Proxy stopped.");
    Ok(())
}

#[tauri::command]
async fn test_country(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Value, String> {
    let config = read_config_value(&app)?;
    let http_port = port_from_config(&config, "listen", "http_port", 18080);
    push_log(&app, &state, "Testing country...");
    let (direct, proxy) = tauri::async_runtime::spawn_blocking(move || {
        let direct = command_output(
            "/usr/bin/curl",
            &["-m", "12", "-sS", "https://ipinfo.io/country"],
            Duration::from_secs(14),
        )
        .unwrap_or_else(|e| e)
        .trim()
        .to_string();
        let proxy_url = format!("http://127.0.0.1:{http_port}");
        let proxy = command_output(
            "/usr/bin/curl",
            &[
                "-m",
                "12",
                "-sS",
                "-x",
                &proxy_url,
                "https://ipinfo.io/country",
            ],
            Duration::from_secs(14),
        )
        .unwrap_or_else(|e| e)
        .trim()
        .to_string();
        (direct, proxy)
    })
    .await
    .map_err(|e| e.to_string())?;
    push_log(
        &app,
        &state,
        format!("Country test: direct={direct}, proxy={proxy}"),
    );
    Ok(json!({ "direct": direct, "proxy": proxy }))
}

#[tauri::command]
async fn set_system_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    let config = read_config_value(&app)?;
    let http_port = port_from_config(&config, "listen", "http_port", 18080);
    let socks_port = port_from_config(&config, "listen", "socks_port", 18081);
    apply_system_proxy("Wi-Fi", http_port, socks_port, enabled);
    if enabled {
        push_log(&app, &state, "System proxy enabled for Wi-Fi.");
    } else {
        push_log(&app, &state, "System proxy disabled for Wi-Fi.");
    }
    Ok(())
}

#[tauri::command]
async fn set_shell_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<(), String> {
    let config = read_config_value(&app)?;
    let http_port = port_from_config(&config, "listen", "http_port", 18080);
    let socks_port = port_from_config(&config, "listen", "socks_port", 18081);
    for (shell, path) in shell_paths()? {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let old = fs::read_to_string(&path).unwrap_or_default();
        let cleaned = remove_managed_block(&old);
        if enabled {
            let next = format!(
                "{}\n\n{}",
                cleaned.trim(),
                shell_block(&shell, http_port, socks_port)
            );
            fs::write(&path, next.trim_start()).map_err(|e| e.to_string())?;
        } else if cleaned.trim().is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            fs::write(&path, cleaned).map_err(|e| e.to_string())?;
        }
    }
    push_log(
        &app,
        &state,
        if enabled {
            "Shell proxy configured."
        } else {
            "Shell proxy removed."
        },
    );
    Ok(())
}

fn chrono_like_now() -> String {
    command_output_or_empty("/bin/date", &["+%H:%M:%S"], 2)
        .trim()
        .to_string()
}

fn read_traffic_stats_from_path(path: PathBuf) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({
            "total": 0,
            "proxy": 0,
            "direct": 0,
            "recent": [],
            "proxy_hosts": [],
            "updated_at": "",
        }));
    }

    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let max_bytes = 1024 * 1024;
    if len > max_bytes {
        file.seek(SeekFrom::Start(len - max_bytes))
            .map_err(|e| e.to_string())?;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|e| e.to_string())?;
    if len > max_bytes {
        if let Some(index) = text.find('\n') {
            text = text[index + 1..].to_string();
        }
    }

    let mut total = 0usize;
    let mut proxy = 0usize;
    let mut direct = 0usize;
    let mut proxy_hosts: HashMap<String, usize> = HashMap::new();
    let mut recent = Vec::new();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let route = event.get("route").and_then(Value::as_str).unwrap_or("");
        let host = event
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        total += 1;
        if route == "proxy" {
            proxy += 1;
            if !host.is_empty() {
                *proxy_hosts.entry(host).or_insert(0) += 1;
            }
        } else if route == "direct" {
            direct += 1;
        }
        recent.push(event);
    }

    recent.reverse();
    recent.truncate(240);
    let mut hosts: Vec<Value> = proxy_hosts
        .into_iter()
        .map(|(host, count)| json!({ "host": host, "count": count }))
        .collect();
    hosts.sort_by(|left, right| {
        right
            .get("count")
            .and_then(Value::as_u64)
            .cmp(&left.get("count").and_then(Value::as_u64))
    });
    hosts.truncate(24);

    Ok(json!({
        "total": total,
        "proxy": proxy,
        "direct": direct,
        "recent": recent,
        "proxy_hosts": hosts,
        "updated_at": chrono_like_now(),
    }))
}

#[tauri::command]
async fn get_traffic_stats() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| read_traffic_stats_from_path(traffic_log_path()?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn clear_traffic_log(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let path = traffic_log_path()?;
    fs::write(path, "").map_err(|e| e.to_string())?;
    push_log(&app, &state, "Traffic monitor cleared.");
    Ok(())
}

#[tauri::command]
async fn get_login_item_enabled() -> Result<bool, String> {
    Ok(launch_agent_path()?.exists())
}

#[tauri::command]
async fn set_login_item_enabled(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<bool, String> {
    let path = launch_agent_path()?;
    if enabled {
        let parent = path
            .parent()
            .ok_or_else(|| "invalid LaunchAgents path".to_string())?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe
            .to_str()
            .ok_or_else(|| "invalid executable path".to_string())?;
        fs::write(&path, login_item_plist(exe)).map_err(|e| e.to_string())?;
        push_log(&app, &state, "Login auto start enabled.");
    } else {
        if path.exists() {
            let uid = user_id();
            if !uid.is_empty() {
                let _ = Command::new("/bin/launchctl")
                    .args([
                        "bootout",
                        &format!("gui/{uid}"),
                        path.to_str().unwrap_or_default(),
                    ])
                    .status();
            }
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        push_log(&app, &state, "Login auto start disabled.");
    }
    Ok(path.exists())
}

#[tauri::command]
async fn list_app_proxy_entries() -> Result<Vec<AppProxyEntry>, String> {
    read_app_proxy_entries()
}

#[tauri::command]
async fn choose_app_for_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<AppProxyEntry>, String> {
    let script =
        r#"POSIX path of (choose file of type {"app"} with prompt "选择要通过代理启动的应用")"#;
    let selected = command_output(
        "/usr/bin/osascript",
        &["-e", script],
        Duration::from_secs(120),
    )?
    .trim()
    .to_string();
    if selected.is_empty() {
        return read_app_proxy_entries();
    }
    let mut entries = read_app_proxy_entries()?;
    if !entries.iter().any(|entry| entry.path == selected) {
        entries.push(AppProxyEntry {
            id: selected.clone(),
            name: app_name_from_path(&selected),
            path: selected,
        });
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        write_app_proxy_entries(&entries)?;
        push_log(&app, &state, "Application proxy entry added.");
    }
    Ok(entries)
}

#[tauri::command]
async fn remove_app_proxy_entry(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Vec<AppProxyEntry>, String> {
    let mut entries = read_app_proxy_entries()?;
    entries.retain(|entry| entry.id != id);
    write_app_proxy_entries(&entries)?;
    push_log(&app, &state, "Application proxy entry removed.");
    Ok(entries)
}

#[tauri::command]
async fn launch_app_with_proxy(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<(), String> {
    let config = read_config_value(&app)?;
    let http_port = port_from_config(&config, "listen", "http_port", 18080);
    let socks_port = port_from_config(&config, "listen", "socks_port", 18081);
    let entries = read_app_proxy_entries()?;
    let entry = entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| "应用不存在".to_string())?;
    let executable = app_executable(&entry.path)?;
    let proxy_url = format!("http://127.0.0.1:{http_port}");
    let socks_url = format!("socks5h://127.0.0.1:{socks_port}");
    Command::new(executable)
        .env("http_proxy", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("all_proxy", &socks_url)
        .env("HTTP_PROXY", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("ALL_PROXY", &socks_url)
        .env("no_proxy", "localhost,127.0.0.1,::1")
        .env("NO_PROXY", "localhost,127.0.0.1,::1")
        .arg(format!("--proxy-server={proxy_url}"))
        .arg("--proxy-bypass-list=<-loopback>")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    push_log(&app, &state, format!("Launched {} with proxy.", entry.name));
    Ok(())
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动代理", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止代理", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &start, &stop, &quit])?;
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "start" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<Arc<AppState>>();
                    let _ = start_proxy(app.clone(), state).await;
                });
            }
            "stop" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<Arc<AppState>>();
                    let _ = stop_proxy(app.clone(), state).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    if std::env::args().any(|arg| arg == "--self-test-detect") {
        let started = Instant::now();
        match detect_astrill_port_sync() {
            Ok(Some(port)) => {
                println!(
                    "detect_astrill_port=Some({port}) elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                std::process::exit(0);
            }
            Ok(None) => {
                println!(
                    "detect_astrill_port=None elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!(
                    "detect_astrill_port_error={error} elapsed_ms={}",
                    started.elapsed().as_millis()
                );
                std::process::exit(1);
            }
        }
    }
    let args = std::env::args().collect::<Vec<_>>();
    if let Some(index) = args.iter().position(|arg| arg == "--self-test-traffic") {
        let path = args.get(index + 1).map(PathBuf::from).unwrap_or_else(|| {
            traffic_log_path().unwrap_or_else(|_| PathBuf::from("traffic.jsonl"))
        });
        match read_traffic_stats_from_path(path) {
            Ok(stats) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&stats).unwrap_or_else(|_| "{}".into())
                );
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("traffic_stats_error={error}");
                std::process::exit(1);
            }
        }
    }
    if let Some(index) = args.iter().position(|arg| arg == "--self-test-app-exec") {
        let Some(path) = args.get(index + 1) else {
            eprintln!("missing .app path");
            std::process::exit(2);
        };
        match app_executable(path) {
            Ok(exe) => {
                println!("app_executable={}", exe.display());
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("app_executable_error={error}");
                std::process::exit(1);
            }
        }
    }
    if args.iter().any(|arg| arg == "--self-test-login-plist") {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_string))
            .unwrap_or_else(|| {
                "/Applications/AstrillSplitProxy.app/Contents/MacOS/astrill-split-proxy".into()
            });
        let path = std::env::temp_dir().join(format!("{LAUNCH_AGENT_ID}.selftest.plist"));
        if let Err(error) = fs::write(&path, login_item_plist(&exe)) {
            eprintln!("login_plist_write_error={error}");
            std::process::exit(1);
        }
        let result = Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&path)
            .output();
        let _ = fs::remove_file(&path);
        match result {
            Ok(output) if output.status.success() => {
                println!("login_plist=ok");
                std::process::exit(0);
            }
            Ok(output) => {
                eprintln!("{}", String::from_utf8_lossy(&output.stderr));
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!("login_plist_lint_error={error}");
                std::process::exit(1);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(Arc::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            get_status,
            detect_astrill_port,
            start_proxy,
            stop_proxy,
            test_country,
            set_system_proxy,
            set_shell_proxy,
            get_traffic_stats,
            clear_traffic_log,
            get_login_item_enabled,
            set_login_item_enabled,
            list_app_proxy_entries,
            choose_app_for_proxy,
            remove_app_proxy_entry,
            launch_app_with_proxy
        ])
        .setup(|app| {
            ensure_config(&app.handle())?;
            build_tray(app)?;
            if std::env::args().any(|arg| arg == "--background") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
