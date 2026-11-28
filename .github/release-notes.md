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

- **beta.6: frames window / 帧解析窗口** (▤ above the log): what a port sends and receives, one message per row, as **Modbus RTU** (function, slave, registers by address, exceptions, CRC checked) or **SLCAN** (CAN ID, DLC, data). A row opens its fields and the bytes each takes; ∿ plots any number field as a curve. Choosing a protocol later still decodes what came before.
  日志上方的 ▤ 打开。端口收发的数据按 **Modbus RTU**（功能码、从站、按地址列出的寄存器、异常码、CRC 校验）或 **SLCAN**（CAN ID、DLC、数据）逐条显示，一行一条消息；点开一行可看各字段及对应字节，点 ∿ 可把任何数值字段画成曲线。先收数据、后选协议，之前的数据也会被解析。
- **beta.6: your own protocols / 自定义协议**: describe a protocol in a small TOML file (header, length, checksum, fields with names, units and scaling) and write it in the built-in editor (*Edit protocol…*), which decodes pasted bytes as you type and shows mistakes with their line. See [docs/protocols.md](https://github.com/jerry-token/serialbot/blob/main/docs/protocols.md).
  用一个小 TOML 文件描述协议（帧头、长度、校验、带名称/单位/缩放的字段），在内置编辑器（「编辑协议…」）里编写，粘贴字节即时显示解析结果，写错会提示行号。说明见 [docs/protocols.zh-CN.md](https://github.com/jerry-token/serialbot/blob/main/docs/protocols.zh-CN.md)。
- **beta.6: decoding in the terminal and for AI / 命令行与 AI 也能解析**: `serialbot-cli monitor <port> --decode modbus-rtu` (or `slcan`, or your protocol file) prints one line per message, `--json` gives the fields; MCP agents call `io.read` with `decode` and get fields and checksum results instead of bytes.
  `serialbot-cli monitor <端口> --decode modbus-rtu`（或 `slcan`、你的协议文件）每条消息一行，`--json` 输出字段；MCP 智能体调用 `io.read` 时加 `decode`，拿到的是字段和校验结果而不是原始字节。
- **beta.6: test sequences / 测试序列** (🔁 above the log): a script of `send`, `expect`, `wait`, `dtr` / `rts` and `mark` steps run N times or until stopped, with passed / failed counts, run times, the failed steps and a report to copy; a command group becomes a script with one click. In the terminal: `serialbot-cli run <port> test.txt --runs 100`.
  日志上方的 🔁 打开。由 `send`、`expect`、`wait`、`dtr` / `rts`、`mark` 组成的脚本跑 N 轮或直到停止，统计通过和失败、每轮耗时和失败的步骤，可复制报告；命令分组一键转成脚本。终端里：`serialbot-cli run <端口> test.txt --runs 100`。
- **beta.6: MCP setup is copy-only / MCP 配置改为复制**: the AI page now shows, for each client, the file it reads by default and the settings to copy into it; SerialBot no longer writes other programs' files. `serialbot-cli mcp install` became `serialbot-cli mcp config <client>`.
  AI 页按客户端显示默认配置文件位置和要复制进去的内容，SerialBot 不再改写其他程序的文件。`serialbot-cli mcp install` 改为 `serialbot-cli mcp config <客户端>`。
- **beta.5: layout fixes / 界面修复**: with HEX on, the checksum dropdown no longer pushes the window wider; the status bar and the pop-up panels (e.g. the DTR/RTS sequence) stay inside small windows, and the waveform leaves the terminal room. A test in a real browser now checks every panel at sizes from 640×420 up.
  打开 HEX 时校验下拉框不再把窗口撑出横向滚动条；小窗口下状态栏和弹出面板（如 DTR/RTS 时序）不再超出窗口，波形图也会给终端留出空间。新增真实浏览器测试，在 640×420 起的各种窗口尺寸下检查每个面板。
- **beta.4: XMODEM / YMODEM** to MCU bootloaders: XMODEM (128, CRC or checksum), XMODEM-1K and YMODEM (1K or 128, with name and size). Pick the protocol in the app's file send panel; `/send-file --ymodem fw.bin` in `open` / `tui`; `serialbot-cli send <port> --file fw.bin --protocol ymodem`. Tested against lrzsz; please report how it works with your bootloader.
  发送文件到 MCU bootloader：XMODEM（128，CRC 或校验和）、XMODEM-1K、YMODEM（1K 或 128，带文件名和大小）。App 文件发送面板里选协议；`open` / `tui` 里用 `/send-file --ymodem fw.bin`；命令行 `serialbot-cli send <端口> --file fw.bin --protocol ymodem`。已和 lrzsz 互通测试，欢迎反馈你的 bootloader 是否正常。
- **beta.4: Copy for AI / 复制给 AI**: copies the newest 200 lines shown, with times and a question, to paste into any AI chat.
  复制当前显示的最新 200 行，带上时间和一个问题，可直接粘贴到任意 AI 聊天里。
- **beta.4: VOFA+ waveforms / VOFA+ 波形**: FireWater lines (`prefix:1,2,3`) plot as they are; JustFloat binary frames with the plot's format picker, or `/plot justfloat` and `--plot justfloat` in `tui`.
  FireWater 文本（`prefix:1,2,3`）直接画曲线；JustFloat 二进制帧在波形图的格式选择里切换，`tui` 里用 `/plot justfloat` 或 `--plot justfloat`。
- **beta.3**: the first build with a Microsoft Store package (MSIX); on Windows, `serialbot mcp` for Claude Desktop also works from the Store version through its `serialbot.exe` alias. The site has a privacy policy: SerialBot collects no personal data.
  首次附带微软商店安装包（MSIX）；商店版也能通过 `serialbot.exe` 别名给 Claude Desktop 提供 `serialbot mcp`。官网新增隐私政策：SerialBot 不收集任何个人数据。
- **Fixed in beta.2 / beta.2 修复** (serialbot-cli): Enter in `open` and `tui` sends CR only, as a terminal does, so shells that take CR and LF each as Enter no longer answer twice (#108; `--ending crlf` if a device wants it). Backspace deletes in the input box also in terminals that send ^H (#109). In the app's send box, pick `\r` as the ending for such shells.
  `open` 和 `tui` 的回车只发 CR，和终端一致，把 CR、LF 各当一次回车的 shell 不再响应两次（#108；设备需要时用 `--ending crlf`）。终端的退格键发 ^H 时，输入框也能删字了（#109）。App 发送框遇到这类 shell 时，行尾选 `\r` 即可。
- **serialbot-cli, SerialBot in the terminal / 命令行版 serialbot-cli**: a separate download (see the second table). One-shot `ports`, `send --expect`, `monitor`, `signal` and `run` (scripts or the app's command export) for scripts and CI, with exit codes; `open`, an interactive terminal with minicom's `Ctrl+A` keys, an input box with history, `/` commands and raw mode; `mcp serve`, the MCP server without the app, asking in the terminal, and `mcp stdio` for Claude Desktop; completions for bash, zsh, fish and PowerShell.
  单独下载（见第二张表）。一次性命令 `ports`、`send --expect`、`monitor`、`signal`、`run`（脚本或 App 导出的命令分组），带退出码，适合脚本和 CI；`open` 是按键兼容 minicom 的交互终端，带输入框、历史、`/` 命令和原始模式；`mcp serve` 不开 App 也能提供 MCP 服务并在终端里确认，`mcp stdio` 给 Claude Desktop 用；支持 bash、zsh、fish、PowerShell 命令补全。
- **Full screen: `serialbot-cli tui` / 全屏界面**: up to four ports side by side or stacked (drag the borders), a session list with news dots and scrollback; a waveform of the numbers in the data with up to six curves told apart by color and point shape; `--mcp` adds an AI panel where you allow (`y`/`s`) or refuse (`n`) an agent's changes; the app's command groups, sent with a click.
  最多四个串口左右或上下分屏（可拖动边框），会话列表带新数据提示和回看；把数据里的数值画成波形，最多 6 条曲线，用颜色和点形区分；`--mcp` 打开 AI 面板，按 `y`/`s` 允许、`n` 拒绝智能体的操作；显示 App 的命令分组，点击即发送。
- **Copyright and name / 版权与名称**: every source file now carries the GPL-3.0 and copyright lines, NOTICE says what copies and changed versions must keep, and TRADEMARKS.md says forks are welcome under their own name. Official releases come only from this page and serialbot.aiany.cc.
  所有源文件都加上了 GPL-3.0 和版权声明；NOTICE 说明复制和修改后再发布必须保留什么；TRADEMARKS.md 说明欢迎 fork，但请用自己的名字。正式版本只从本页面和 serialbot.aiany.cc 发布。

### What works / 已有功能

- Serial ports, TCP client / server and UDP; several sessions at once; auto-reconnect after replugging
- Text / HEX receive with timestamps, Δt, line numbers, ANSI colors, encodings (UTF-8, GBK, GB18030, Big5, …), auto frame split, pause, search and filter, keyword highlight, bookmarks; large logs stay smooth
- Terminal mode and live plots
- Many ports at once: tabs, ports in their own windows, news on other tabs
- Multi-line send (text or HEX), history, timed send, saved command lists (also in their own window), checksums, file send (also XMODEM / YMODEM), test sequences, save or capture the log
- Frame decoding: Modbus RTU, SLCAN and your own TOML protocols, with plots of any field
- DTR / RTS control and timed sequences, CTS / DSR / RI / CD status, BREAK
- MCP server for AI agents (AI page → enable → copy the `claude mcp add …` command, or the Claude Desktop config), on this computer or optionally the local network; every change an agent makes waits for your approval, and its lines are marked AI
- `serialbot-cli` for the terminal: one-shot commands, an interactive and a full-screen terminal, scripts, the MCP server
- English and Chinese; light and dark themes
- Tested at 2 Mbit/s for an hour on x64 and arm64 without loss
