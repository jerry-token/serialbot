**Alpha build for testing.** Expect rough edges and please report problems in [Issues](https://github.com/jerry-token/SerialBot/issues) with screenshots, your OS and USB-serial chip (CH340 / CP2102 / FT232 / …).

**Alpha 测试版。** 功能还不完整，欢迎试用并在 [Issues](https://github.com/jerry-token/SerialBot/issues) 反馈问题，请附截图、操作系统和串口芯片型号。

### Downloads / 下载

| System | File | Notes |
|---|---|---|
| Windows 10/11 x64 | `SerialBot_*_x64-setup.exe` | Installer (no admin needed). 安装版，无需管理员权限 |
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

### What works / 已有功能

- Serial ports: open several at once, baud / data bits / parity / stop bits / flow control
- Text / HEX receive with timestamps, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search (Ctrl/Cmd+F), large logs stay smooth
- Multi-line send, text or HEX, line ending choice
- DTR / RTS control, CTS / DSR / RI / CD status
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command)
