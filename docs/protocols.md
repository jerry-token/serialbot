# Your own protocols

SerialBot decodes Modbus RTU and SLCAN out of the box. For anything else, such as your board's
`AA 55` frames, describe the protocol in a small TOML file and SerialBot shows every message
with its fields, checks the checksum and marks bytes that do not fit.

The quickest way is the editor: open the frames window (▤ above the log) and press **Edit
protocol…**. You get the description on one side and, on the other, the frames it makes of bytes
you paste (as hex), updated as you type, with any mistake and its line. **Save** writes it to
the protocols folder and picks it.

You can also put files in the **protocols folder** yourself: press **Protocol files…** in the
frames window. The first time, the folder is created with `example.toml` in it.
The file then shows up in the Protocol list; after you save a change, click back into the frames
window and it decodes again with the new description. In the terminal:

```sh
serialbot-cli monitor COM3 --decode thermostat          # thermostat.toml in the protocols folder
serialbot-cli monitor COM3 --decode ./thermostat.toml   # any file
```

The folder is `protocols` in SerialBot's settings folder: `%APPDATA%\cc.aiany.serialbot` on
Windows, `~/Library/Application Support/cc.aiany.serialbot` on macOS,
`~/.config/cc.aiany.serialbot` on Linux.

## An example

A frame is `AA 55`, a length byte, a command byte, the data, and a CRC-16 (Modbus, low byte
first). The length counts the bytes after it, the CRC included.

```toml
name = "Thermostat"
endian = "le"                    # multi-byte numbers: "le" (default) or "be"

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
values = { "0x01" = "Read temperature", "0x81" = "Temperature" }

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

`AA 55 05 81 E7 00 xx xx` then shows as **Temperature**, `temp=23.1 °C`, with a ✓ for the CRC.

## Cutting frames: `[frame]`

Every key is optional.

| Key | Meaning |
| --- | --- |
| `header` | Hex bytes every frame starts with (`"AA 55"`). Bytes before a header are shown as unknown. |
| `size` | Every frame is this many bytes, header and checksum included. |
| `length` | A length field: `at` (its byte in the frame, from 0), `size` (1, 2 or 4), `endian`, `counts`, `add`. |
| `end` | Hex bytes every frame ends with (`"0D 0A"`). |
| `checksum` | `kind`, and `from` (the first byte it covers, default 0) and `endian` if needed. |
| `gap_ms` | How long bytes wait for the rest of a frame before they are shown as cut short (default 50). |

What `length` counts:

| `counts` | The length field holds |
| --- | --- |
| `"rest"` (default) | the bytes after the length field, to the end of the frame |
| `"all"` | the whole frame |
| `"body"` | the bytes after the length field, up to the checksum |

`add` corrects a length that is off by a few bytes (`add = 2`, or `add = -1`).

With neither `size`, `length` nor `end`, a frame runs to the next header, or ends when the line
is quiet for `gap_ms`.

Checksum kinds are those of the send box: `crc8`, `crc16-modbus`, `crc16-ccitt`, `crc32`, `xor`,
`sum8`, `lrc`. The checksum sits at the end of the frame (before `end`). Its byte order follows
the send box (CRC-16 Modbus low byte first, CRC-16 CCITT and CRC-32 high byte first); `endian`
changes it.

## Fields: `[[field]]`

Fields follow the header, in order. Each has a `name` and a `type`.

| Type | Bytes |
| --- | --- |
| `u8` `i8` | 1 |
| `u16` `i16` | 2 |
| `u32` `i32` `f32` | 4 |
| `u64` `i64` `f64` | 8 |
| `bytes` | `len`, or the rest of the frame |
| `ascii` | `len`, or the rest of the frame (shown as text) |
| `bits` | none: bits of an earlier field |

Add `le` or `be` to a number type for its own byte order (`u16be`).

| Key | Meaning |
| --- | --- |
| `len` | For `bytes` and `ascii`: a number, or the name of an earlier number field. |
| `count` | An array: a number, or the name of an earlier number field. |
| `values` | Names for values: `{ "0x01" = "Read", "2" = "Write" }`. The first named value names the frame. |
| `scale`, `add`, `unit` | Shown as value × `scale` + `add`, then the unit. |
| `when` | Only in some frames: `{ cmd = 0x81 }` or `{ cmd = [1, 2] }`. |
| `of`, `bits` | For `bits`: the earlier field, and which bits (`"3..5"` or `"7"`). |
| `summary` | `true` puts the field in the frame's one-line summary. Without any, the first four fields are shown. |

## When something is wrong

A mistake in the file is shown with its line, in the frames window and by `serialbot-cli`:
`line 14: field "temp": type "i17" is not one of …`. Fix it, save, and click back into the
window.

Frames whose checksum is wrong are still decoded and marked ✗, with the checksum they should
have had. Bytes that are not a frame are shown as unknown; nothing is hidden.
