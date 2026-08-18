#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, WindowEvent,
};

const SERVER_URL: &str = "http://127.0.0.1:3080";
const SERVER_PORT: u16 = 3080;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DEFAULT_REPO: &str = r"D:\Program Files\Dev\deepseek-harness";
const NODE_VERSION: &str = "v20.20.2";
const NODE_ARCHIVE: &str = "node-v20.20.2-win-x64.zip";

#[derive(Clone, Copy, PartialEq)]
enum NodeSource {
    Official,
    Mirror,
}

impl NodeSource {
    fn label(self) -> &'static str {
        match self {
            NodeSource::Official => "官方源",
            NodeSource::Mirror => "镜像源",
        }
    }

    fn node_url(self, version: &str, archive: &str) -> String {
        match self {
            NodeSource::Official => {
                format!("https://nodejs.org/download/release/{version}/{archive}")
            }
            NodeSource::Mirror => {
                format!("https://npmmirror.com/mirrors/node/{version}/{archive}")
            }
        }
    }

    fn npm_registry(self) -> &'static str {
        match self {
            NodeSource::Official => "https://registry.npmjs.org",
            NodeSource::Mirror => "https://registry.npmmirror.com",
        }
    }
}

#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    percent: u8,
    message: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Config {
    dsh_path: String,
    dsh_kind: String,
    installed_at: String,
}

#[derive(Clone, Copy, PartialEq)]
enum DshKind {
    Repo,
    Npm,
}

impl DshKind {
    fn as_str(self) -> &'static str {
        match self {
            DshKind::Repo => "repo",
            DshKind::Npm => "npm",
        }
    }
}

#[derive(Clone)]
struct Installation {
    kind: DshKind,
    root: PathBuf,
    entry: PathBuf,
}

struct ServerState(Mutex<Option<Child>>);

enum ProgressTarget<'a> {
    App(&'a AppHandle),
    Console,
}

fn report_progress(target: &ProgressTarget<'_>, percent: u8, message: &str) {
    match target {
        ProgressTarget::App(app) => emit_progress(app, percent, message),
        ProgressTarget::Console => println!("[{percent:>3}%] {message}"),
    }
}

fn app_data_dir() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DeepSeek Harness")
}

fn config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

fn log_path() -> PathBuf {
    app_data_dir().join("logs").join("dsh.log")
}

fn runtime_node_path() -> PathBuf {
    app_data_dir()
        .join("runtime")
        .join("node")
        .join("node.exe")
}

fn load_config() -> Option<Config> {
    let path = config_path();
    if !path.is_file() {
        return None;
    }
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save_config(kind: DshKind, path: &Path) -> std::io::Result<()> {
    let cfg = Config {
        dsh_path: path.to_string_lossy().into_owned(),
        dsh_kind: kind.as_str().to_string(),
        installed_at: chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    };
    let dir = app_data_dir();
    fs::create_dir_all(&dir)?;
    fs::write(
        config_path(),
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
    )
}

fn is_server_running() -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], SERVER_PORT)),
        Duration::from_millis(300),
    )
    .is_ok()
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
    })
}

fn find_node() -> Option<PathBuf> {
    let candidates = vec![
        runtime_node_path(),
        PathBuf::from(r"D:\Program Files\Dev\Node.js\node.exe"),
        PathBuf::from(r"C:\Program Files\nodejs\node.exe"),
        PathBuf::from(r"C:\Program Files (x86)\nodejs\node.exe"),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    find_in_path("node.exe")
}

fn find_npm_cli() -> Option<PathBuf> {
    let node = find_node()?;
    let cli = node
        .parent()?
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");
    cli.is_file().then_some(cli)
}

fn repo_entry(root: &Path) -> Option<PathBuf> {
    let lib = root.join("apps").join("cli").join("lib").join("bin.js");
    if lib.is_file() {
        return Some(lib);
    }
    let src = root.join("apps").join("cli").join("src").join("bin.ts");
    src.is_file().then_some(src)
}

fn npm_global_bin() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")?;
    let bin = PathBuf::from(appdata)
        .join("npm")
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js");
    bin.is_file().then_some(bin)
}

fn valid_config(cfg: &Config) -> bool {
    let path = Path::new(&cfg.dsh_path);
    match cfg.dsh_kind.as_str() {
        "repo" => repo_entry(path).is_some(),
        "npm" => path.join("lib").join("bin.js").is_file(),
        _ => false,
    }
}

fn detect() -> Option<Installation> {
    if let Some(cfg) = load_config() {
        if valid_config(&cfg) {
            let root = PathBuf::from(&cfg.dsh_path);
            if cfg.dsh_kind == "npm" {
                let entry = root.join("lib").join("bin.js");
                return Some(Installation {
                    kind: DshKind::Npm,
                    root,
                    entry,
                });
            }
            if let Some(entry) = repo_entry(&root) {
                return Some(Installation {
                    kind: DshKind::Repo,
                    root,
                    entry,
                });
            }
        }
    }

    let default_repo = Path::new(DEFAULT_REPO);
    if let Some(entry) = repo_entry(default_repo) {
        return Some(Installation {
            kind: DshKind::Repo,
            root: default_repo.to_path_buf(),
            entry,
        });
    }

    if let Some(home) = env::var_os("DSH_HOME") {
        let root = PathBuf::from(home);
        if let Some(entry) = repo_entry(&root) {
            return Some(Installation {
                kind: DshKind::Repo,
                root,
                entry,
            });
        }
    }

    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let has_dsh = ["dsh.cmd", "dsh.exe", "dsh"]
                .iter()
                .any(|name| dir.join(name).is_file());
            if has_dsh {
                let pkg = dir
                    .join("node_modules")
                    .join("@deepseek-ai")
                    .join("dsh");
                let bin = pkg.join("lib").join("bin.js");
                if bin.is_file() {
                    return Some(Installation {
                        kind: DshKind::Npm,
                        root: pkg,
                        entry: bin,
                    });
                }
                break;
            }
        }
    }

    if let Some(bin) = npm_global_bin() {
        if let Some(pkg) = bin.parent().and_then(|p| p.parent()) {
            return Some(Installation {
                kind: DshKind::Npm,
                root: pkg.to_path_buf(),
                entry: bin,
            });
        }
    }

    None
}

fn emit_progress(app: &AppHandle, percent: u8, message: &str) {
    let _ = app.emit(
        "dsh-progress",
        ProgressPayload {
            percent,
            message: message.to_string(),
        },
    );
}

fn emit_error(app: &AppHandle, message: &str) {
    let _ = app.emit(
        "dsh-error",
        ProgressPayload {
            percent: 0,
            message: message.to_string(),
        },
    );
}

fn emit_need_install(app: &AppHandle) {
    let _ = app.emit(
        "dsh-need-install",
        ProgressPayload {
            percent: 0,
            message: String::new(),
        },
    );
}

fn start_server(app: &AppHandle, state: &ServerState, inst: &Installation) -> Result<(), String> {
    let node = find_node().ok_or_else(|| "未找到 Node.js，请安装 Node.js 后重试".to_string())?;
    let log_dir = app_data_dir().join("logs");
    fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| e.to_string())?;
    let err_log = log.try_clone().map_err(|e| e.to_string())?;

    let mut stamp = log.try_clone().map_err(|e| e.to_string())?;
    let _ = writeln!(
        stamp,
        "\n==== DeepSeek Harness client started at {} ====",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    let mut cmd = Command::new(&node);
    let is_ts = inst
        .entry
        .extension()
        .map(|e| e == "ts")
        .unwrap_or(false);
    if is_ts {
        cmd.args(["--import", "tsx/esm"]);
    }
    cmd.arg(&inst.entry).arg("web");
    cmd.current_dir(&inst.root);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdout(Stdio::from(log)).stderr(Stdio::from(err_log));

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 dsh 服务失败：{e}"))?;

    let mut guard = state.0.lock().unwrap();
    if let Some(mut old) = guard.take() {
        let _ = old.kill();
        let _ = old.wait();
    }
    *guard = Some(child);
    Ok(())
}

fn wait_for_server(app: &AppHandle) -> bool {
    let started = Instant::now();
    let mut last_pct = 15u8;
    loop {
        if is_server_running() {
            emit_progress(app, 92, "服务已就绪，正在进入...");
            return true;
        }
        let elapsed = started.elapsed().as_secs_f32();
        let pct = (15.0 + (elapsed / 30.0) * 70.0).min(90.0) as u8;
        if pct != last_pct && pct % 10 == 0 {
            last_pct = pct;
            emit_progress(app, pct, "正在启动本地服务...");
        }
        if elapsed > 90.0 {
            return false;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn enter_webui(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(url) = SERVER_URL.parse() {
            let _ = window.navigate(url);
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

fn continue_bootstrap(app: AppHandle) {
    thread::spawn(move || {
        if is_server_running() {
            emit_progress(&app, 95, "检测到服务已在运行");
            enter_webui(&app);
            return;
        }
        match detect() {
            Some(inst) => {
                emit_progress(&app, 15, "已找到 DeepSeek Harness，正在启动服务...");
                let state = app.state::<ServerState>();
                if let Err(e) = start_server(&app, &state, &inst) {
                    emit_error(&app, &e);
                    return;
                }
                if wait_for_server(&app) {
                    emit_progress(&app, 100, "服务已就绪");
                    enter_webui(&app);
                } else {
                    emit_error(&app, "服务启动超时，请查看日志确认状态");
                }
            }
            None => {
                emit_progress(&app, 5, "未检测到安装");
                emit_need_install(&app);
            }
        }
    });
}

fn run_capture(mut cmd: Command) -> std::io::Result<Output> {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdin(Stdio::null());
    cmd.output()
}

fn run_powershell_progress(
    command: &str,
    target: &ProgressTarget<'_>,
    label: &str,
) -> std::io::Result<Output> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
        .arg(command);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdin(Stdio::null());

    let err_path = std::env::temp_dir().join(format!("dsh-node-{}.err", std::process::id()));
    let err_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&err_path)?;
    cmd.stderr(Stdio::from(err_file));

    let mut child = cmd.stdout(Stdio::piped()).spawn()?;
    let mut out_buf = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if let Some(pct) = trimmed.strip_prefix("PROGRESS:") {
                        if let Ok(n) = pct.trim().parse::<u8>() {
                            let percent = (5u16 + (n as u16) * 35 / 100).min(40) as u8;
                            report_progress(
                                target,
                                percent,
                                &format!("正在从{label}下载 Node.js 运行环境... {n}%"),
                            );
                        }
                    } else if !trimmed.is_empty() {
                        out_buf.push(trimmed.to_string());
                    }
                }
                Err(_) => break,
            }
        }
    }

    let status = child.wait()?;
    let stderr = fs::read(&err_path).unwrap_or_default();
    let _ = fs::remove_file(&err_path);
    Ok(Output {
        status,
        stdout: out_buf.join("\n").into_bytes(),
        stderr,
    })
}

fn powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn ensure_node(target: &ProgressTarget<'_>, preferred: NodeSource) -> Result<PathBuf, String> {
    let force_bootstrap = env::var_os("DSH_FORCE_NODE_BOOTSTRAP").is_some();
    if !force_bootstrap {
        if let Some(node) = find_node() {
            if find_npm_cli().is_some() {
                return Ok(node);
            }
        }
    }

    let runtime_root = app_data_dir().join("runtime");
    let runtime_dir = runtime_root.join("node");
    let archive = runtime_root.join(NODE_ARCHIVE);
    let staging = runtime_root.join("node-staging");
    let sources = if preferred == NodeSource::Official {
        [NodeSource::Official, NodeSource::Mirror]
    } else {
        [NodeSource::Mirror, NodeSource::Official]
    };

    report_progress(target, 3, "未检测到可用的 Node.js，正在下载运行环境...");
    for source in sources {
        let label = source.label();
        let url = source.node_url(NODE_VERSION, NODE_ARCHIVE);
        report_progress(
            target,
            4,
            &format!("正在从{label}下载 Node.js 运行环境..."),
        );
        let script = r#"
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $runtimeRoot | Out-Null
Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $staging | Out-Null
[System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor [System.Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.Net.Http
$client = New-Object System.Net.Http.HttpClient
try {
  $response = $client.GetAsync($url, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).Result
  if (-not $response.IsSuccessStatusCode) { throw "HTTP $([int]$response.StatusCode)" }
  $total = $response.Content.Headers.ContentLength
  $stream = $response.Content.ReadAsStreamAsync().Result
  $fs = [System.IO.File]::Create($archive)
  $buffer = New-Object byte[] 81920
  $done = [long]0
  $lastPct = -1
  try {
    while ($true) {
      $read = $stream.Read($buffer, 0, $buffer.Length)
      if ($read -le 0) { break }
      $fs.Write($buffer, 0, $read)
      $done += $read
      if ($total -gt 0) {
        $pct = [int](($done * 100) / $total)
        if ($pct -ne $lastPct) { $lastPct = $pct; Write-Output "PROGRESS:$pct" }
      }
    }
  } finally {
    $fs.Dispose()
    $stream.Dispose()
    $response.Dispose()
  }
} finally {
  $client.Dispose()
}
Expand-Archive -LiteralPath $archive -DestinationPath $staging -Force
$root = Get-ChildItem -LiteralPath $staging -Directory | Select-Object -First 1
if ($null -eq $root -or -not (Test-Path -LiteralPath (Join-Path $root.FullName 'node.exe'))) {
  throw 'Downloaded Node.js archive did not contain node.exe'
}
Remove-Item -LiteralPath $runtimeDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null
Copy-Item -Path (Join-Path $root.FullName '*') -Destination $runtimeDir -Recurse -Force
Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
"#;
        let command = format!(
            "$url={}; $archive={}; $runtimeRoot={}; $runtimeDir={}; $staging={}; {}",
            powershell_literal(&url),
            powershell_literal(&archive.to_string_lossy()),
            powershell_literal(&runtime_root.to_string_lossy()),
            powershell_literal(&runtime_dir.to_string_lossy()),
            powershell_literal(&staging.to_string_lossy()),
            script
        );
        let output = run_powershell_progress(&command, target, label)
        .map_err(|e| format!("下载 Node.js 失败：{e}"))?;
        if output.status.success() && runtime_node_path().is_file() {
            if let Some(node) = find_node() {
                if find_npm_cli().is_some() {
                    report_progress(target, 42, "Node.js 运行环境准备完成");
                    return Ok(node);
                }
            }
        }
        let detail = String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap_or("未知网络或解压错误")
            .to_string();
        report_progress(target, 4, &format!("{label}下载失败：{detail}"));
    }

    Err("无法下载 Node.js 运行环境，请检查网络连接后重试".to_string())
}

fn install_auto(target: &ProgressTarget<'_>, preferred: NodeSource) -> Result<(), String> {
    let node = ensure_node(target, preferred)?;
    let npm_cli = find_npm_cli().ok_or("未找到 npm 组件，无法自动安装")?;

    let registries = if preferred == NodeSource::Official {
        [
            ("官方 npm 源", NodeSource::Official.npm_registry()),
            ("镜像 npm 源", NodeSource::Mirror.npm_registry()),
        ]
    } else {
        [
            ("镜像 npm 源", NodeSource::Mirror.npm_registry()),
            ("官方 npm 源", NodeSource::Official.npm_registry()),
        ]
    };
    let mut installed = false;
    for (index, (label, registry)) in registries.iter().enumerate() {
        report_progress(
            target,
            45 + (index as u8 * 7),
            &format!("正在从{label}下载并安装 DeepSeek Harness..."),
        );
        let out = run_capture({
            let mut c = Command::new(&node);
            c.args([
                npm_cli.to_string_lossy().as_ref(),
                "install",
                "-g",
                "@deepseek-ai/dsh",
                "--no-fund",
                "--no-audit",
                "--registry",
                registry,
            ]);
            c
        })
        .map_err(|e| format!("npm 安装执行失败：{e}"))?;
        if out.status.success() {
            installed = true;
            break;
        }
        report_progress(target, 52 + (index as u8 * 7), &format!("{label}安装失败，准备重试..."));
    }

    if !installed {
        return Err(
            "npm 安装失败：官方 npm 源和镜像源均不可用，请检查网络连接后重试".to_string(),
        );
    }

    let prefix_out = run_capture({
        let mut c = Command::new(&node);
        c.args([npm_cli.to_string_lossy().as_ref(), "config", "get", "prefix"]);
        c
    })
    .map_err(|e| format!("读取 npm 配置失败：{e}"))?;
    let prefix = String::from_utf8_lossy(&prefix_out.stdout).trim().to_string();
    if prefix.is_empty() {
        return Err("读取 npm 配置失败，无法完成自动安装".to_string());
    }

    let pkg_root = PathBuf::from(&prefix)
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    if !pkg_root.join("lib").join("bin.js").is_file() {
        return Err("npm 安装完成但未找到 dsh 入口，无法完成自动安装".to_string());
    }

    save_config(DshKind::Npm, &pkg_root).map_err(|e| format!("保存配置失败：{e}"))?;
    report_progress(target, 80, "安装完成，正在启动服务...");
    Ok(())
}

fn pick_manual(app: &AppHandle) -> Result<(), String> {
    let picked = rfd::FileDialog::new()
        .set_title("选择 DeepSeek Harness 安装位置")
        .pick_folder();
    let Some(dir) = picked else {
        return Ok(());
    };

    let dir = dir.to_path_buf();
    if let Some(_entry) = repo_entry(&dir) {
        save_config(DshKind::Repo, &dir).map_err(|e| format!("保存配置失败：{e}"))?;
        emit_progress(app, 20, "已选择安装目录，正在启动服务...");
        return Ok(());
    }
    if dir.join("lib").join("bin.js").is_file() && dir.join("package.json").is_file() {
        save_config(DshKind::Npm, &dir).map_err(|e| format!("保存配置失败：{e}"))?;
        emit_progress(app, 20, "已选择 dsh 包目录，正在启动服务...");
        return Ok(());
    }

    Err("所选目录不是有效的 DeepSeek Harness 安装目录（未找到 apps/cli 或 lib/bin.js）".to_string())
}

#[tauri::command]
fn dsh_install_auto(app: AppHandle, source: Option<String>) -> Result<(), String> {
    let preferred = match source.as_deref() {
        Some("mirror") => NodeSource::Mirror,
        _ => NodeSource::Official,
    };
    let handle = app.clone();
    thread::spawn(move || match install_auto(&ProgressTarget::App(&handle), preferred) {
        Ok(()) => continue_bootstrap(handle),
        Err(e) => emit_error(&handle, &e),
    });
    Ok(())
}

#[tauri::command]
fn dsh_pick_manual(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    thread::spawn(move || match pick_manual(&handle) {
        Ok(()) => continue_bootstrap(handle),
        Err(e) => emit_error(&handle, &e),
    });
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn kill_pid_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn kill_server(state: &ServerState) {
    if let Some(mut child) = state.0.lock().unwrap().take() {
        kill_pid_tree(child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn port_listener_pid() -> Option<u32> {
    let mut cmd = Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains(":3080") && line.contains("LISTENING") {
            if let Some(pid) = line
                .split_whitespace()
                .last()
                .and_then(|s| s.parse::<u32>().ok())
            {
                return Some(pid);
            }
        }
    }
    None
}

fn restart_server(app: AppHandle) {
    thread::spawn(move || {
        emit_progress(&app, 5, "正在停止服务...");
        let state = app.state::<ServerState>();
        kill_server(&state);
        if let Some(pid) = port_listener_pid() {
            kill_pid_tree(pid);
        }

        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if !is_server_running() {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }

        emit_progress(&app, 15, "正在重新启动服务...");
        match detect() {
            Some(inst) => {
                if let Err(e) = start_server(&app, &state, &inst) {
                    emit_error(&app, &e);
                    return;
                }
                if wait_for_server(&app) {
                    emit_progress(&app, 100, "服务已重新启动");
                    enter_webui(&app);
                } else {
                    emit_error(&app, "服务重启超时，请查看日志确认状态");
                }
            }
            None => {
                show_main(&app);
                emit_need_install(&app);
            }
        }
    });
}

fn main() {
    if std::env::args().any(|arg| arg == "--test-auto-install") {
        if std::env::args().any(|arg| arg == "--force-node-bootstrap") {
            std::env::set_var("DSH_FORCE_NODE_BOOTSTRAP", "1");
        }
        let preferred = if std::env::args().any(|arg| arg == "--source-mirror") {
            NodeSource::Mirror
        } else {
            NodeSource::Official
        };
        println!(
            "DeepSeek Harness automatic installation test (source: {})",
            preferred.label()
        );
        match install_auto(&ProgressTarget::Console, preferred) {
            Ok(()) => {
                println!("PASS: automatic installation completed");
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("FAIL: {error}");
                std::process::exit(1);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if args.iter().any(|a| a == "--restart") {
                show_main(app);
                restart_server(app.clone());
            } else {
                show_main(app);
            }
        }))
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }

            app.manage(ServerState(Mutex::new(None)));

            let show_item = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
            let restart_item = MenuItem::with_id(app, "restart", "重启服务", true, None::<&str>)?;
            let exit_item = MenuItem::with_id(app, "exit", "退出进程", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &restart_item, &exit_item])?;
            let icon = app
                .default_window_icon()
                .cloned()
                .expect("missing app icon");

            TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("DeepSeek Harness")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "restart" => {
                        show_main(app);
                        restart_server(app.clone());
                    }
                    "exit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            if std::env::args().any(|a| a == "--restart") {
                show_main(app.handle());
                restart_server(app.handle().clone());
            } else {
                continue_bootstrap(app.handle().clone());
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![dsh_install_auto, dsh_pick_manual])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<ServerState>() {
                    kill_server(&state);
                }
            }
        });
}
