<div align="center">

<img src="app/src-tauri/icons/128x128.png" width="96" alt="SerialBot logo">

# SerialBot

**轻量、跨平台的串口调试助手，内置 MCP 服务，让 AI 也能直接操作串口** 🔌🤖

[![CI](https://github.com/jerry-token/serialbot/actions/workflows/ci.yml/badge.svg)](https://github.com/jerry-token/serialbot/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/jerry-token/serialbot?include_prereleases&label=release&color=orange)](https://github.com/jerry-token/serialbot/releases)
![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
[![License](https://img.shields.io/badge/license-GPL--3.0-green)](LICENSE)

[English](README.md) · 简体中文

<img src="docs/images/demo.gif" width="800" alt="AI 通过 MCP 打开串口、发送命令并等待回复，窗口实时显示">

<sub>☝️ Claude Code 通过 MCP 打开开发板串口、发命令、等回复，窗口实时显示每一步</sub>

</div>

> [!NOTE]
> 🚧 **当前是 Alpha 测试版**：基础功能可用，欢迎试用并[反馈问题](https://github.com/jerry-token/serialbot/issues)。

## ✨ 特性

- 🪶 **轻量**：Windows 安装版约 3.5 MB；免安装单文件版约 16 MB，不用安装直接运行。
- 💻 **跨平台**：Windows、macOS（Apple Silicon 和 Intel）、Linux 都能用。
- 🗂️ **多串口同时打开**：每个串口一个标签页，各自有日志和收发计数。
- 🌈 **接收显示**：
  - 文本 / HEX 两种显示方式，带毫秒级时间戳
  - 支持 **ANSI 颜色**（Zephyr、ESP-IDF 等彩色日志）
  - 支持 GBK、GB18030、Big5 等多种编码
  - 自动断帧、暂停、`Ctrl/Cmd+F` 搜索
  - 几万行日志依然流畅
- ✍️ **发送**：多行输入，文本或 HEX，可选结尾符；`Enter` 发送，`Shift+Enter` 换行，`↑` 调出历史；支持定时发送。
  发出去的是原始字节，和输入完全一致：不加任何帧头或协议，只加你选的结尾符（HEX 不加）。
- 🧮 **校验和**：HEX 发送时可自动追加 CRC-8、CRC-16（Modbus、CCITT）、CRC-32、XOR、SUM8、LRC。
- 🔎 **搜索与过滤**：`Ctrl/Cmd+F` 在日志中查找；勾选"只显示匹配的行"即可过滤。**Δt** 显示与上一行的时间差，HEX 视图下就是帧间隔。
- 🖍️ **关键字高亮**：在显示设置（⚙️）里填上 `ERROR`、`WARN` 这样的词，日志里出现时会用颜色标出来。
- 📋 **扩展命令列表**：常用命令分组保存，点一下发送，也可以整组按顺序带延时执行。
- 📄 **发送文件**：分块发送，可设块间隔，适合没有流控的设备。
- 💾 **保存日志**：一次性导出，或边收边写入文件。
- 🌐 **TCP 与 UDP**：除了串口，还支持 TCP 客户端、TCP 服务端（一次服务一个客户端，连入和断开会记录在日志里）和 UDP。在连接栏最左边选择类型；AI 通过 MCP 打开 `tcp://主机:端口`、`tcp-server://:端口` 或 `udp://主机:端口?local=:端口`，和打开串口一样。
- 🔁 **自动重连**：拔掉串口再插上，会话自动接上；串口列表随插拔刷新。
- 🎛️ **信号线**：DTR / RTS 开关，CTS / DSR / RI / CD 指示灯，每次切换都会在日志里记一笔。
  BRK 发送 BREAK；⏱ 执行带时间的 DTR/RTS 时序，例如 `DTR=0 RTS=1, 100, RTS=0`（复位脉冲）。
- 🤖 **AI 友好**：内置 MCP 服务，Claude Code 等 AI 工具可以直接
  - 列出、打开串口
  - 收发数据、等待匹配的回复
  - 控制 DTR / RTS

  你在窗口里打开的串口，AI 能接着用；AI 发的每条数据，窗口里也看得到，并带 **AI** 标记和底色。
  在状态栏关掉 **允许 AI**，这个会话就不让 AI 碰。
  AI 发送数据、切换信号线、打开/关闭/改配置串口之前，SerialBot 会先问你：允许一次、本会话都允许、或拒绝（60 秒不处理自动拒绝）。
  不想被问可以在 MCP 设置里关掉。

## 📸 截图

| 深色主题 | 浅色主题 |
|:---:|:---:|
| <img src="docs/images/terminal-dark.png" alt="深色主题下的终端页"> | <img src="docs/images/terminal-light.png" alt="浅色主题下的终端页"> |

<details>
<summary>🤖 AI 页：MCP 服务设置</summary>

<img src="docs/images/ai-dark.png" width="800" alt="AI 页：启用 MCP 服务并复制 claude mcp add 命令">

</details>

## 📦 下载安装

到 [Releases 页面](https://github.com/jerry-token/serialbot/releases) 下载对应系统的文件：

| 系统 | 文件 | 说明 |
|---|---|---|
| 🪟 Windows 10/11 | `SerialBot_*_x64-setup.exe` | 安装版（约 3.5 MB，已压缩），不需要管理员权限 |
| 🪟 Windows 10/11 | `serialbot_windows_x64.exe` | 免安装版（约 16 MB，未压缩），双击直接运行 |
| 🍎 macOS 11+ | `SerialBot_*_universal.dmg` | Apple Silicon 和 Intel 通用 |
| 🐧 Ubuntu / Debian | `SerialBot_*_amd64.deb` | `sudo apt install ./SerialBot_*.deb` |
| 🐧 其他 Linux | `SerialBot_*_amd64.AppImage` | `chmod +x` 后直接运行 |

<details>
<summary>⚠️ 首次打开遇到提示？</summary>

- **macOS 提示「已损坏」或「无法打开」**：Alpha 版还没做苹果公证，在终端执行一次：
  ```sh
  xattr -dr com.apple.quarantine /Applications/SerialBot.app
  ```
- **Windows 提示「未知发布者」**：点「更多信息」→「仍要运行」。
- **Linux 打不开串口（权限不够）**：执行下面这条，然后重新登录：
  ```sh
  sudo usermod -aG dialout $USER
  ```

</details>

## 🚀 快速上手

1. 🔌 插上 USB 转串口模块，在顶部选择端口，设置波特率。数据位、校验、停止位、流控在 `8N1 ▾` 里，然后点 **打开**。
2. 📥 接收区默认显示时间戳。右上角的小开关可以切换 HEX 显示、显示发送内容、自动滚动、暂停；🔍 是搜索，⚙️ 里可以设置编码、自动断帧和 ANSI 颜色。
3. 📤 在底部输入框写好命令，按 `Enter` 发送。勾选 HEX 可以发送 `01 03 00 00` 这样的十六进制数据。
4. 🎛️ 需要复位开发板时，点状态栏的 **DTR / RTS**。

> [!TIP]
> 如果开发板没有接 CTS 线，流控请保持「无」。开着 RTS/CTS 硬件流控时，数据会因为 CTS 无效而发不出去。

## 🤖 让 AI 操作串口（MCP）

1. 点左侧 **AI** → 勾选 **启用 MCP 服务**。服务只监听本机 `127.0.0.1`，访问需要 token。
2. 点 **复制**，在终端里运行复制下来的命令，例如：
   ```sh
   claude mcp add --transport http serialbot http://127.0.0.1:9633/mcp \
     --header "Authorization: Bearer <你的 token>"
   ```
3. 在 Claude Code 里直接说：*「列出串口，打开 ESP32 那个口，发送 `version` 看看固件版本」* 🎉

**Claude Desktop**（以及其他只支持 stdio 的客户端）：把 AI 页上的第二段配置复制到 `claude_desktop_config.json`。它运行 `serialbot mcp`，一个连接到正在运行的 SerialBot 的小桥接程序，所以需要先打开 SerialBot 并启用 MCP 服务：

```json
{ "mcpServers": { "serialbot": { "command": "/Applications/SerialBot.app/Contents/MacOS/serialbot", "args": ["mcp"] } } }
```

Windows 上 command 填 `serialbot.exe` 的路径（安装版或免安装版都行）；Linux 上填 AppImage 的路径或 `/usr/bin/serialbot`。

| 工具 | 作用 |
|---|---|
| `port.list` / `session.list` | 列出串口 / 已打开的会话 |
| `session.open` | 打开串口（可以起个别名，比如 `esp32`） |
| `io.write` / `io.read` | 发送数据 / 按游标读取带时间戳的日志 |
| `io.wait_for` | 等待接收内容匹配某个正则表达式 |
| `io.send_and_expect` | 发送一条命令并等待期望的回复 |
| `io.signals` | 设置 DTR / RTS，读取 CTS / DSR / RI / CD |
| `session.close` | 关闭会话的端口，日志仍可读取 |
| `session.release` / `session.reopen` | 把端口让给其他工具（如烧录器）再取回，日志接着记 |
| `session.configure` | 不断开修改波特率 |
| `log.mark` / `log.query` | 插入命名标记；汇总或搜索标记之后的日志 |
| `command.list` / `command.call` | 其余命令：改别名、自动重连、发送 BREAK |

## 🗺️ 路线图

- [x] 串口收发、多会话、时间戳、HEX
- [x] ANSI 颜色、多编码、自动断帧、搜索、大日志流畅显示
- [x] DTR / RTS 和信号状态
- [x] 内置 MCP 服务
- [x] 三平台安装包自动发布
- [x] 定时发送、发送历史
- [x] 发送文件
- [x] 扩展命令列表（分组、一键顺序发送）
- [x] 接收存文件、导出
- [x] 热插拔检测、自动重连
- [x] TCP Client / Server、UDP
- [ ] 内置 AI 助手「小波」

## 🛠️ 从源码构建

需要 Rust（版本由 `rust-toolchain.toml` 自动安装）和 Node 22+。Linux 还要装几个系统库：

```sh
sudo apt install libwebkit2gtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev libudev-dev
```

```sh
cd app && npm ci && npm run tauri dev     # 运行桌面版
cargo test --workspace                    # Rust 测试
cd app && npm test                        # 前端测试
cargo run -p serialbot-mcp                  # 不带界面，单独运行 MCP 服务
```

每次改动都要附带测试，并通过格式、clippy、测试和覆盖率检查，详见 [CLAUDE.md](CLAUDE.md)。

## 🤝 反馈

发现 bug 或有功能建议，欢迎提 [Issue](https://github.com/jerry-token/serialbot/issues)。请附上：

- 📷 截图
- 💻 操作系统
- 🔌 串口芯片型号（CH340 / CP2102 / FT232 / CH9344 …）

## 📄 许可证

核心代码使用 [GPL-3.0-only](LICENSE)。插件 SDK 和设备端库将使用 MIT 许可。
