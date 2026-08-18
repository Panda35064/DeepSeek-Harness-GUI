#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
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
    let candidates = [
        r"D:\Program Files\Dev\Node.js\node.exe",
        r"C:\Program Files\nodejs\node.exe",
        r"C:\Program Files (x86)\nodejs\node.exe",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
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

fn find_git() -> Option<PathBuf> {
    let candidates = [
        r"D:\Program Files\Dev\Git\cmd\git.exe",
        r"C:\Program Files\Git\cmd\git.exe",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    find_in_path("git.exe")
}

fn find_pnpm() -> Option<PathBuf> {
    let candidates = [
        r"D:\Program Files\Dev\Node.js\node_modules\pnpm\bin\pnpm.cjs",
        r"C:\Program Files\nodejs\node_modules\pnpm\bin\pnpm.cjs",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(appdata) = env::var_os("APPDATA") {
        let p = PathBuf::from(appdata)
            .join("npm")
            .join("node_modules")
            .join("pnpm")
            .join("bin")
            .join("pnpm.cjs");
        if p.is_file() {
            return Some(p);
        }
    }
    None
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

fn install_git(app: &AppHandle) -> Result<(), String> {
    let git = find_git().ok_or("未找到 Git，无法从源码安装")?;
    let node = find_node().ok_or("未找到 Node.js")?;
    let pnpm = find_pnpm().ok_or("未找到 pnpm，无法从源码安装")?;

    let repo = Path::new(DEFAULT_REPO);
    if repo_entry(repo).is_none() {
        if repo.exists() {
            return Err(format!(
                "{DEFAULT_REPO} 已存在但不是有效的 DeepSeek Harness 仓库，请手动处理"
            ));
        }
        emit_progress(app, 25, "正在克隆 DeepSeek Harness 源码...");
        let out = run_capture({
            let mut c = Command::new(&git);
            c.args([
                "clone",
                "--depth",
                "1",
                "https://github.com/deepseek-ai/deepseek-harness.git",
                DEFAULT_REPO,
            ]);
            c
        })
        .map_err(|e| format!("克隆执行失败：{e}"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "克隆失败：{}",
                msg.chars().take(300).collect::<String>()
            ));
        }
    }

    emit_progress(app, 45, "正在安装依赖（pnpm install）...");
    let out = run_capture({
        let mut c = Command::new(&node);
        c.arg(&pnpm).arg("install").current_dir(repo);
        c
    })
    .map_err(|e| format!("pnpm install 执行失败：{e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "pnpm install 失败：{}",
            msg.chars().take(300).collect::<String>()
        ));
    }

    emit_progress(app, 65, "正在构建（pnpm build）...");
    let out = run_capture({
        let mut c = Command::new(&node);
        c.arg(&pnpm).args(["run", "build"]).current_dir(repo);
        c
    })
    .map_err(|e| format!("构建执行失败：{e}"))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "构建失败：{}",
            msg.chars().take(300).collect::<String>()
        ));
    }

    save_config(DshKind::Repo, repo).map_err(|e| format!("保存配置失败：{e}"))?;
    emit_progress(app, 85, "源码安装完成");
    Ok(())
}

fn install_auto(app: &AppHandle) -> Result<(), String> {
    let node = find_node().ok_or("未找到 Node.js，请先安装 Node.js")?;
    let npm_cli = find_npm_cli().ok_or("未找到 npm 组件，无法自动安装")?;

    emit_progress(app, 8, "正在下载并安装 DeepSeek Harness（npm 官方包）...");
    let out = run_capture({
        let mut c = Command::new(&node);
        c.args([
            npm_cli.to_string_lossy().as_ref(),
            "install",
            "-g",
            "@deepseek-ai/dsh",
            "--no-fund",
            "--no-audit",
        ]);
        c
    })
    .map_err(|e| format!("npm 安装执行失败：{e}"))?;

    if !out.status.success() {
        emit_progress(app, 20, "npm 安装未成功，切换到源码安装...");
        return install_git(app);
    }

    let prefix_out = run_capture({
        let mut c = Command::new(&node);
        c.args([npm_cli.to_string_lossy().as_ref(), "config", "get", "prefix"]);
        c
    })
    .map_err(|e| format!("读取 npm 配置失败：{e}"))?;
    let prefix = String::from_utf8_lossy(&prefix_out.stdout).trim().to_string();
    if prefix.is_empty() {
        emit_progress(app, 20, "npm 配置读取异常，切换到源码安装...");
        return install_git(app);
    }

    let pkg_root = PathBuf::from(&prefix)
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh");
    if !pkg_root.join("lib").join("bin.js").is_file() {
        emit_progress(app, 20, "npm 安装完成但未找到入口，切换到源码安装...");
        return install_git(app);
    }

    save_config(DshKind::Npm, &pkg_root).map_err(|e| format!("保存配置失败：{e}"))?;
    emit_progress(app, 80, "安装完成，正在启动服务...");
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
fn dsh_install_auto(app: AppHandle) -> Result<(), String> {
    let handle = app.clone();
    thread::spawn(move || match install_auto(&handle) {
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
