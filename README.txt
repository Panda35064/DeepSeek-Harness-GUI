DeepSeek Harness 桌面客户端（Tauri）
=====================================

安装位置：D:\Program Files\Dev\DeepSeek-Harness-GUI\DeepSeek-Harness-GUI.exe
启动方式：双击安装后的快捷方式，或直接运行上面的 exe。

安装程序语言：启动安装程序时可选择简体中文或 English。

功能说明：
- 双击启动后先显示加载页（DeepSeek 图标 + 进度条 + 当前步骤），同时自动拉起本地 dsh 服务
  （http://127.0.0.1:3080），就绪后在内置窗口中打开 Web UI。
- 点击窗口右上角关闭按钮时不会退出程序，而是最小化到系统托盘。
- 左键单击托盘图标：重新显示主窗口。
- 右键托盘图标菜单：
  - “显示窗口”：重新显示主窗口
  - “重启服务”：停止并重新启动本地 dsh 服务，完成后自动回到 Web UI
  - “退出进程”：完全退出程序，并自动停止本地 dsh 服务
- 已支持单实例：重复双击只会唤起已有窗口，不会重复启动服务。
- 深色模式：当 Windows 处于深色模式时，加载页与 dsh 首页会自动切换为深色主题。

安装检测与安装：
- 启动时依次检测：保存的配置（%APPDATA%\DeepSeek Harness\config.json）→ 默认安装路径
  D:\Program Files\Dev\deepseek-harness → 环境变量 DSH_HOME → PATH 中的 dsh 命令。
- 未找到时会弹出安装选择：
  - “自动安装”：优先通过 npm 全局安装官方包 @deepseek-ai/dsh（国内镜像，速度快），
    失败则自动回退为 git 克隆源码 + pnpm 构建。
  - “手动选择路径”：选择已有的 DeepSeek Harness 安装目录，校验通过后使用。
- 安装完成后自动继续启动服务并进入 Web UI，选择结果会保存，下次直接读取。

启动速度优化：
- 直接运行构建产物 apps/cli/lib/bin.js（或 npm 包入口），实测约 2~3 秒服务就绪，
  不再经过 tsx 实时转译源码的慢路径。

日志位置：%APPDATA%\DeepSeek Harness\logs\dsh.log

工程与源码：本目录（Tauri + Rust）。源码包含 `.github/workflows/build.yml`：
- 推送到 `main`/`master` 或提交 Pull Request 时自动构建 Windows 安装包并保存为 Actions artifact。
- 推送 `v*` 标签时自动创建 GitHub Release，并上传 NSIS 安装包。

卸载：删除“D:\Program Files\Dev\DeepSeek Harness”文件夹和桌面快捷方式；
如需同时移除 Harness 本体，删除“D:\Program Files\Dev\deepseek-harness”。
