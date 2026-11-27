# 自定义协议

SerialBot 自带 Modbus RTU 和 SLCAN 解析。其他协议（比如你板子上 `AA 55` 开头的帧），用一个小小的 TOML
文件描述出来，SerialBot 就会逐条显示消息和各字段，校验校验和，并标出对不上的字节。

文件放在**协议文件夹**里：在 App 里打开帧解析窗口（日志上方的 ▤），点 **协议文件…**。第一次会创建这个文件夹，
并放一个 `example.toml`。之后文件就会出现在“协议”列表里；保存修改后，点回帧解析窗口，就会按新的描述重新解析。
在终端里：

```sh
serialbot-cli monitor COM3 --decode thermostat          # 协议文件夹里的 thermostat.toml
serialbot-cli monitor COM3 --decode ./thermostat.toml   # 任意文件
```

协议文件夹是 SerialBot 设置目录下的 `protocols`：Windows 为 `%APPDATA%\cc.aiany.serialbot`，macOS 为
`~/Library/Application Support/cc.aiany.serialbot`，Linux 为 `~/.config/cc.aiany.serialbot`。

## 示例

一帧是 `AA 55`、一个长度字节、一个命令字节、数据，最后是 CRC-16（Modbus，低字节在前）。长度表示它之后的字节数，含 CRC。

```toml
name = "温控板"
endian = "le"                    # 多字节数值的字节序："le"（默认）或 "be"

[frame]
header = "AA 55"
length = { at = 2, size = 1, counts = "rest" }
checksum = { kind = "crc16-modbus" }

[[field]]
name = "len"
type = "u8"

[[field]]
name = "cmd"
type = "u8"
values = { "0x01" = "读温度", "0x81" = "温度回复" }

[[field]]
name = "temp"
type = "i16"
scale = 0.1
unit = "°C"
when = { cmd = 0x81 }
summary = true

[[field]]
name = "data"
type = "bytes"
```

`AA 55 05 81 E7 00 xx xx` 会显示为 **温度回复**，`temp=23.1 °C`，CRC 正确显示 ✓。

## 切帧：`[frame]`

每个键都可以不写。

| 键 | 含义 |
| --- | --- |
| `header` | 每帧开头的十六进制字节（`"AA 55"`）。帧头之前的字节显示为“未识别”。 |
| `size` | 每帧固定字节数，含帧头和校验。 |
| `length` | 长度字段：`at`（它在帧里的字节位置，从 0 起）、`size`（1、2 或 4）、`endian`、`counts`、`add`。 |
| `end` | 每帧结尾的十六进制字节（`"0D 0A"`）。 |
| `checksum` | `kind`，需要时再加 `from`（校验从哪个字节开始，默认 0）和 `endian`。 |
| `gap_ms` | 字节等待帧剩余部分的时间，超过后显示为“不完整”（默认 50）。 |

`length` 表示什么：

| `counts` | 长度字段的值 |
| --- | --- |
| `"rest"`（默认） | 长度字段之后到帧尾的字节数 |
| `"all"` | 整帧字节数 |
| `"body"` | 长度字段之后、校验之前的字节数 |

`add` 用来修正差几个字节的长度（`add = 2` 或 `add = -1`）。

`size`、`length`、`end` 都不写时，一帧到下一个帧头为止，或在线路静默 `gap_ms` 后结束。

校验类型和发送框一致：`crc8`、`crc16-modbus`、`crc16-ccitt`、`crc32`、`xor`、`sum8`、`lrc`。校验位于帧尾（`end` 之前），
字节序和发送框一致（CRC-16 Modbus 低字节在前，CRC-16 CCITT 和 CRC-32 高字节在前），可用 `endian` 改。

## 字段：`[[field]]`

字段紧跟帧头，按顺序排列。每个字段有 `name` 和 `type`。

| 类型 | 字节数 |
| --- | --- |
| `u8` `i8` | 1 |
| `u16` `i16` | 2 |
| `u32` `i32` `f32` | 4 |
| `u64` `i64` `f64` | 8 |
| `bytes` | `len`，或帧的剩余部分 |
| `ascii` | `len`，或帧的剩余部分（按文本显示） |
| `bits` | 不占字节：取前面某个字段的若干位 |

数值类型后加 `le` 或 `be` 可单独指定字节序（`u16be`）。

| 键 | 含义 |
| --- | --- |
| `len` | 用于 `bytes` 和 `ascii`：一个数，或前面某个数值字段的名字。 |
| `count` | 数组：一个数，或前面某个数值字段的名字。 |
| `values` | 给值起名：`{ "0x01" = "读", "2" = "写" }`。第一个有名字的值作为帧名。 |
| `scale`、`add`、`unit` | 显示为 值 × `scale` + `add`，再加单位。 |
| `when` | 只在某些帧里出现：`{ cmd = 0x81 }` 或 `{ cmd = [1, 2] }`。 |
| `of`、`bits` | 用于 `bits`：前面的字段，以及取哪几位（`"3..5"` 或 `"7"`）。 |
| `summary` | `true` 表示放进帧的一行摘要。都没写时，摘要显示前四个字段。 |

## 出错时

文件里的错误会带行号显示，在帧解析窗口和 `serialbot-cli` 里都一样：
`line 14: field "temp": type "i17" is not one of …`。改好、保存，再点回窗口即可。

校验错误的帧仍会解析，标 ✗，并给出正确的校验值。不属于任何帧的字节显示为“未识别”，不会隐藏任何数据。
