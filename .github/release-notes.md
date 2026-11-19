**Beta.** Everything planned for the first release is in. Checks on real USB-serial adapters and macOS are under way, so please report problems in [Issues](https://github.com/jerry-token/serialbot/issues) with screenshots, your OS and USB-serial chip (CH340 / CP2102 / FT232 / …).

**Beta 测试版。** 首个正式版计划的功能已全部完成，真机（各类 USB 串口芯片、macOS）验证正在进行。欢迎试用并在 [Issues](https://github.com/jerry-token/serialbot/issues) 反馈问题，请附截图、操作系统和串口芯片型号。

### Downloads / 下载

Exact file sizes are listed at the end. 各文件的实际大小见文末。

| System | File | Notes |
|---|---|---|
| Windows 10/11 x64 | `SerialBot_*_x64-setup.exe` | Installer, no admin needed. 安装版，无需管理员权限 |
| Windows 10/11 x64 | `serialbot_windows_x64.exe` | Portable, no install. 免安装版，直接运行（需系统自带的 WebView2） |
| macOS 11+ (Apple Silicon & Intel) | `SerialBot_*_universal.dmg` | See note below. 见下方说明 |
| Linux x64 | `SerialBot_*_amd64.AppImage` | `chmod +x` then run. 加执行权限后直接运行 |
| Debian / Ubuntu x64 | `SerialBot_*_amd64.deb` | `sudo apt install ./SerialBot_*.deb` |
| Linux arm64 | `SerialBot_*_aarch64.AppImage` | Raspberry Pi 4/5 (64-bit OS) and other ARM boards. 树莓派 4/5（64 位系统）等 ARM 设备 |
| Debian / Ubuntu arm64 | `SerialBot_*_arm64.deb` | `sudo apt install ./SerialBot_*_arm64.deb` |

**macOS:** the app is not notarized yet. If macOS says the app "is damaged" or "cannot be opened", run once:
macOS 版暂未公证，若提示「已损坏」或「无法打开」，在终端执行一次：

```sh
xattr -dr com.apple.quarantine /Applications/SerialBot.app
```

**Windows:** SmartScreen may warn about an unknown publisher: click "More info" → "Run anyway".
Windows 可能提示「未知发布者」，点「更多信息」→「仍要运行」。

**Linux serial permissions / Linux 串口权限:** `sudo usermod -aG dialout $USER`, then log out and in again. 执行后重新登录。

### New in this release / 本版新增

- **Terminal mode / 终端模式** (TTY): a full terminal (VT100, colors, full-screen programs) for shells on Linux boards and MCU consoles; keys go straight to the device.
  完整终端（VT100、颜色、全屏程序），适合开发板 shell 和 MCU 控制台，按键直接发给设备。
- **Plot / 波形图**: numbers in received lines (`temp=23.6 hum=41` or `1.2, 3.4`) are drawn live, each series on its own scale.
  把接收行中的数字实时画成曲线，每条曲线独立缩放。
- **Line numbers and bookmarks / 行号与收藏**: `#` shows line numbers; ☆ keeps a line in a list you can jump back to.
  `#` 显示行号；点 ☆ 收藏一行，随时跳回。
- **TCP server with several clients / TCP 服务端多客户端**: up to 16 at once; sending goes to all, and the log shows who sent what.
  最多同时 16 个客户端；发送会发给所有客户端，日志标明数据来自谁。
- **Command list in its own window / 命令列表独立窗口**: ⧉ opens it beside the main window, with its own target port.
  ⧉ 把命令列表放到独立窗口，可单独选择发往哪个端口。
- **Help page / 帮助页**: what SerialBot is for, first steps, every control, connecting an AI agent, keys.
  软件内的使用说明：用途、快速上手、各个按钮、如何接入 AI 智能体、快捷键。

The version number moves to 0.2.0 with this beta. 自本版起版本号为 0.2.0。

### What works / 已有功能

- Serial ports, TCP client / server and UDP; several sessions at once; auto-reconnect after replugging
- Text / HEX receive with timestamps, Δt, line numbers, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search and filter, keyword highlight, bookmarks; large logs stay smooth
- Terminal mode and live plots
- Multi-line send (text or HEX), history, timed send, saved command lists (also in their own window), checksums, file send, save or capture the log
- DTR / RTS control and timed sequences, CTS / DSR / RI / CD status, BREAK
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command, or the Claude Desktop config); every change an agent makes waits for your approval, and its lines are marked AI
- English and Chinese; light and dark themes
- Tested at 2 Mbit/s for an hour on x64 and arm64 without loss
