// Checksums appended to sent HEX data.

/** The checksums the send box can append; "none" appends nothing. */
export const CHECKSUMS = [
  "none",
  "crc8",
  "crc16-modbus",
  "crc16-ccitt",
  "crc32",
  "xor",
  "sum8",
  "lrc",
] as const;

export type Checksum = (typeof CHECKSUMS)[number];

/** Names shown in the picker; the byte order is part of the name. */
export const CHECKSUM_NAMES: Record<Checksum, string> = {
  none: "",
  crc8: "CRC-8",
  "crc16-modbus": "CRC-16 Modbus (LE)",
  "crc16-ccitt": "CRC-16 CCITT (BE)",
  crc32: "CRC-32 (BE)",
  xor: "XOR",
  sum8: "SUM8",
  lrc: "LRC",
};

function crc8(data: Uint8Array): number {
  // CRC-8/SMBUS: poly 0x07, init 0, no reflection.
  let crc = 0;
  for (const byte of data) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = crc & 0x80 ? ((crc << 1) ^ 0x07) & 0xff : (crc << 1) & 0xff;
  }
  return crc;
}

function crc16Modbus(data: Uint8Array): number {
  // Reflected poly 0xA001 (0x8005), init 0xFFFF.
  let crc = 0xffff;
  for (const byte of data) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = crc & 1 ? (crc >>> 1) ^ 0xa001 : crc >>> 1;
  }
  return crc;
}

function crc16Ccitt(data: Uint8Array): number {
  // CRC-16/CCITT-FALSE (IBM-3740): poly 0x1021, init 0xFFFF, no reflection.
  let crc = 0xffff;
  for (const byte of data) {
    crc ^= byte << 8;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    }
  }
  return crc;
}

function crc32(data: Uint8Array): number {
  // CRC-32 (IEEE 802.3, zlib): reflected poly 0xEDB88320, init and final XOR 0xFFFFFFFF.
  let crc = 0xffffffff;
  for (const byte of data) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

const sum = (data: Uint8Array) => data.reduce((acc, b) => (acc + b) & 0xff, 0);

/**
 * The checksum bytes of `data`, in the order they are sent: CRC-16 Modbus
 * low byte first, the other multi-byte CRCs high byte first. SUM8 is the
 * byte sum modulo 256, LRC its two's complement (as in Modbus ASCII), XOR
 * all bytes XORed. Empty data gives each algorithm's initial value.
 */
export function checksumBytes(kind: Checksum, data: Uint8Array): Uint8Array {
  switch (kind) {
    case "none":
      return new Uint8Array();
    case "crc8":
      return Uint8Array.of(crc8(data));
    case "crc16-modbus": {
      const c = crc16Modbus(data);
      return Uint8Array.of(c & 0xff, c >>> 8);
    }
    case "crc16-ccitt": {
      const c = crc16Ccitt(data);
      return Uint8Array.of(c >>> 8, c & 0xff);
    }
    case "crc32": {
      const c = crc32(data);
      return Uint8Array.of(c >>> 24, (c >>> 16) & 0xff, (c >>> 8) & 0xff, c & 0xff);
    }
    case "xor":
      return Uint8Array.of(data.reduce((acc, b) => acc ^ b, 0));
    case "sum8":
      return Uint8Array.of(sum(data));
    case "lrc":
      return Uint8Array.of((0x100 - sum(data)) & 0xff);
  }
}

/** `data` followed by its checksum. */
export function withChecksum(kind: Checksum, data: Uint8Array): Uint8Array {
  const check = checksumBytes(kind, data);
  const out = new Uint8Array(data.length + check.length);
  out.set(data);
  out.set(check, data.length);
  return out;
}
