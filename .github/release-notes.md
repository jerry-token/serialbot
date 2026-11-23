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

**Command line / 命令行版 (`serialbot-cli`)**: a separate download, not included in the app's installers. 单独下载，App 安装包里不含。

| System | File | Notes |
|---|---|---|
| Windows 10/11 x64 | `serialbot-cli-*-windows-x64.zip` | Unzip, put `serialbot-cli.exe` on your PATH. 解压后把 `serialbot-cli.exe` 放进 PATH |
| macOS 11+ (Apple Silicon & Intel) | `serialbot-cli-*-macos-universal.tar.gz` | `tar xzf …`, then `xattr -d com.apple.quarantine serialbot-cli` once. 解压后执行一次去隔离 |
| Linux x64 / arm64 | `serialbot-cli-*-linux-x64.tar.gz` / `…-linux-arm64.tar.gz` | Any distribution. 任意发行版 |
| Debian / Ubuntu x64 / arm64 | `serialbot-cli-*-linux-amd64.deb` / `…-linux-arm64.deb` | `sudo apt install ./serialbot-cli-*.deb`, with shell completions. 含命令补全 |

**macOS:** the app is not notarized yet. If macOS says the app "is damaged" or "cannot be opened", run once:
macOS 版暂未公证，若提示「已损坏」或「无法打开」，在终端执行一次：

```sh
xattr -dr com.apple.quarantine /Applications/SerialBot.app
```

**Windows:** SmartScreen may warn about an unknown publisher: click "More info" → "Run anyway".
Windows 可能提示「未知发布者」，点「更多信息」→「仍要运行」。

**Linux serial permissions / Linux 串口权限:** `sudo usermod -aG dialout $USER`, then log out and in again. 执行后重新登录。

### New in this release / 本版新增

- **Closing a port keeps its tab / 关闭串口保留标签页**: Close (yours or an agent's `session.close`) stops the port but keeps the tab and log, so you can look back at what happened. Open continues in the same tab; × on the tab closes it.
  关闭串口（你点 Close 或 AI 调用 `session.close`）只停止收发，标签页和日志都保留，方便回看 AI 做了什么；再次打开会在原标签里接着记录，点标签上的 × 才关闭标签页。
- **MCP on the local network / MCP 支持局域网**: on the AI page, *Reachable from* can switch from this computer only (127.0.0.1, the default) to the local network (0.0.0.0). The page shows the address and command for other computers. The token is always required.
  AI 页面的"允许访问"可从仅本机（127.0.0.1，默认）改为本机和局域网（0.0.0.0），并给出其他电脑使用的地址和命令；始终需要 token。
- **Windows stay in step / 多窗口状态同步**: the MCP state is the same in every window, and an agent's request is asked only in the window that shows its port.
  所有窗口的 MCP 状态一致；AI 的确认弹窗只出现在显示对应串口的窗口里。
- **Easier to spot news on other tabs / 后台标签提醒更醒目**: an orange dot and a bold name.
  圆点改为橙色，标签名加粗。

### What works / 已有功能

- Serial ports, TCP client / server and UDP; several sessions at once; auto-reconnect after replugging
- Text / HEX receive with timestamps, Δt, line numbers, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search and filter, keyword highlight, bookmarks; large logs stay smooth
- Terminal mode and live plots
- Many ports at once: tabs, ports in their own windows, news on other tabs
- Multi-line send (text or HEX), history, timed send, saved command lists (also in their own window), checksums, file send, save or capture the log
- DTR / RTS control and timed sequences, CTS / DSR / RI / CD status, BREAK
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command, or the Claude Desktop config), on this computer or optionally the local network; every change an agent makes waits for your approval, and its lines are marked AI
- English and Chinese; light and dark themes
- Tested at 2 Mbit/s for an hour on x64 and arm64 without loss
