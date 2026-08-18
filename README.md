<p align="center">
  <img src="src-tauri/icons/deepseek-liquid-glass.svg" alt="DeepSeek Harness GUI icon" width="280">
</p>

<h1 align="center">DeepSeek Harness GUI</h1>

<p align="center">DeepSeek Harness 的 Windows Tauri 桌面启动器</p>

## 功能

- 自动检测并启动本地 DeepSeek Harness 服务（`127.0.0.1:3080`）。
- 支持最小化到系统托盘、单实例运行和服务重启。
- 未检测到 Harness 时支持自动安装或手动选择安装目录。
- NSIS 安装程序支持选择简体中文或 English。

## 使用方法

1. 运行 `DeepSeek-Harness-GUI-Setup.exe`。
2. 在安装程序开始时选择 `简体中文` 或 `English`。
3. 安装完成后启动 DeepSeek Harness GUI。
4. 如果尚未安装 DeepSeek Harness，按界面提示选择自动安装或手动指定目录。

默认安装目录：

```text
D:\Program Files\Dev\DeepSeek-Harness-GUI
```

## 本地构建

### 环境要求

- Windows 10/11
- Node.js 20+
- Rust stable
- Tauri CLI 2
- WebView2 Runtime

### 构建步骤

```powershell
npm ci
npm exec -- tauri build
```

构建产物位于：

```text
src-tauri/target/release/deepseek-harness-client.exe
src-tauri/target/release/bundle/nsis/DeepSeek Harness_0.1.0_x64-setup.exe
```

### 开发运行

```powershell
npm ci
npm exec -- tauri dev
```

## GitHub Actions

工作流文件为 `.github/workflows/build.yml`：

- push 到 `main` 或 `master`、Pull Request、手动触发时自动构建 Windows 安装包，并保存为 Actions artifact。
- 推送 `v*` 标签时自动创建 GitHub Release，并将 NSIS 安装包上传到 Release。

发布示例：

```powershell
git tag v0.1.0
git push origin v0.1.0
```

## 项目结构

```text
dist/                         前端加载页
src-tauri/src/main.rs         Rust/Tauri 主程序
src-tauri/icons/              应用图标资源
src-tauri/tauri.conf.json     Tauri 与 NSIS 配置
.github/workflows/build.yml   自动构建与发布工作流
```
