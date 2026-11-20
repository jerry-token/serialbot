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

For working with many ports at once / 同时调试多个串口：

- **A port in its own window / 串口独立窗口**: double-click a tab (or ⧉) to watch that port in a window of its own, with its full toolbar, log and send box. Close the window and the tab comes back; the port stays open. AI agents still see every port.
  双击标签（或点 ⧉）把该串口放到独立窗口，带完整的工具栏、日志和发送框。关闭窗口后标签回到主窗口，端口保持打开；AI 仍能看到所有端口。
- **Tabs for many ports / 多标签更好用**: tabs have their own row and short names (`CH9344USB8` instead of `/dev/ttyCH9344USB8`), scroll with the mouse wheel, and ▾ lists every open port.
  标签单独一行、显示短名字，鼠标滚轮横向滚动，▾ 列出所有已打开的端口。
- **News on other tabs / 后台标签提醒**: a tab you are not looking at gets a blue ring when data arrives, an agent writes, or the device is lost.
  没在看的标签收到数据、AI 发送或设备掉线时，圆点变成蓝色光圈。
- **Close closes the port you are looking at / 关闭跟着标签走**: selecting a tab selects its port in the connection bar.
  选中标签时，顶部连接栏自动切到该端口，Close 关闭的就是正在看的端口。
- Closing the main window now quits the app. 关闭主窗口即退出程序。

### What works / 已有功能

- Serial ports, TCP client / server and UDP; several sessions at once; auto-reconnect after replugging
- Text / HEX receive with timestamps, Δt, line numbers, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search and filter, keyword highlight, bookmarks; large logs stay smooth
- Terminal mode and live plots
- Many ports at once: tabs, ports in their own windows, news on other tabs
- Multi-line send (text or HEX), history, timed send, saved command lists (also in their own window), checksums, file send, save or capture the log
- DTR / RTS control and timed sequences, CTS / DSR / RI / CD status, BREAK
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command, or the Claude Desktop config); every change an agent makes waits for your approval, and its lines are marked AI
- English and Chinese; light and dark themes
- Tested at 2 Mbit/s for an hour on x64 and arm64 without loss
