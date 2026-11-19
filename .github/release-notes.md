**Alpha build for testing.** Expect rough edges and please report problems in [Issues](https://github.com/jerry-token/serialbot/issues) with screenshots, your OS and USB-serial chip (CH340 / CP2102 / FT232 / …).

**Alpha 测试版。** 功能还不完整，欢迎试用并在 [Issues](https://github.com/jerry-token/serialbot/issues) 反馈问题，请附截图、操作系统和串口芯片型号。

### Downloads / 下载

Exact file sizes are listed at the end. 各文件的实际大小见文末。

| System | File | Notes |
|---|---|---|
| Windows 10/11 x64 | `SerialBot_*_x64-setup.exe` | Installer, no admin needed. 安装版，无需管理员权限 |
| Windows 10/11 x64 | `serialbot_windows_x64.exe` | Portable, no install. 免安装版，直接运行（需系统自带的 WebView2） |
| macOS 11+ (Apple Silicon & Intel) | `SerialBot_*_universal.dmg` | See note below. 见下方说明 |
| Linux x64 | `SerialBot_*_amd64.AppImage` | `chmod +x` then run. 加执行权限后直接运行 |
| Debian / Ubuntu x64 | `SerialBot_*_amd64.deb` | `sudo apt install ./SerialBot_*.deb` |

**macOS:** this alpha is not notarized yet. If macOS says the app "is damaged" or "cannot be opened", run once:
macOS 版暂未公证，若提示「已损坏」或「无法打开」，在终端执行一次：

```sh
xattr -dr com.apple.quarantine /Applications/SerialBot.app
```

**Windows:** SmartScreen may warn about an unknown publisher: click "More info" → "Run anyway".
Windows 可能提示「未知发布者」，点「更多信息」→「仍要运行」。

**Linux serial permissions / Linux 串口权限:** `sudo usermod -aG dialout $USER`, then log out and in again. 执行后重新登录。

### New in this release / 本版新增

- **AI safety / AI 安全**: confirm each change an AI agent makes (allow once, for the session, or deny); withhold a session from AI; lines an agent sent are tagged **AI**.
  AI 改动前弹窗确认；可禁止某个会话被 AI 使用；AI 发送的内容带 **AI** 标记。
- **Checksums / 校验和**: CRC-8, CRC-16 (Modbus, CCITT), CRC-32, XOR, SUM8, LRC appended to HEX data.
- **Signals / 信号线**: BREAK button and timed DTR/RTS sequences (e.g. `DTR=0 RTS=1, 100, RTS=0`).
  BREAK 按钮与带时间的 DTR/RTS 时序。
- **Log / 日志**: show only search matches, Δt between lines (frame gaps in HEX), keyword highlighting.
  只显示匹配行、行间 Δt（HEX 下即帧间隔）、关键字高亮。
- **Settings / 设置**: light / dark / system theme, log font size, lines kept per session, always on top.
  主题、日志字号、保留行数、窗口置顶。

### What works / 已有功能

- Serial ports, TCP client / server and UDP; several sessions at once; auto-reconnect after replugging
- Text / HEX receive with timestamps, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search, large logs stay smooth
- Multi-line send (text or HEX), timed send, saved command lists, file send, save or capture the log
- DTR / RTS control, CTS / DSR / RI / CD status, BREAK
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command, or the Claude Desktop config)
