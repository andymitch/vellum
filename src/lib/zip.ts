// A minimal zip reader/writer for the browser (#221).
//
// Vault export/import is a zip of .md files (#79), produced and consumed by the
// Rust `zip` crate in the desktop/Android builds. The web build has to speak
// the same format so an archive moves either way between the app you installed
// and the app you opened in a tab — which, with no p2p sync in the browser, is
// how notes get across.
//
// Rather than add a zip library, this uses the platform's own DEFLATE
// (Compression/DecompressionStream), leaving only the container headers to
// write. Scope is exactly what a vault archive needs:
//
//   - stored (method 0) and deflated (method 8) entries; nothing else
//   - no encryption, no zip64 (a Markdown vault is nowhere near 4 GiB)
//   - reads are driven off the central directory, which is authoritative and
//     carries the real sizes even when an entry was written with a streaming
//     data descriptor

const LOCAL_SIG = 0x04034b50;
const CD_SIG = 0x02014b50;
const EOCD_SIG = 0x06054b50;
// Bit 11: names are UTF-8. Vault paths are user text, so say so explicitly
// rather than leaving readers to guess at code page 437.
const FLAG_UTF8 = 0x0800;
const STORED = 0;
const DEFLATED = 8;

/// One archive member. A directory is `{ name: "work/", data: null }` — the
/// trailing slash is what makes it one, matching what the Rust side writes for
/// an empty folder.
export type ZipEntry = { name: string; data: Uint8Array | null };

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[i] = c >>> 0;
  }
  return t;
})();

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

async function through(bytes: Uint8Array, transform: GenericTransformStream): Promise<Uint8Array> {
  const stream = new Blob([bytes.slice().buffer as ArrayBuffer]).stream().pipeThrough(transform);
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

const deflate = (bytes: Uint8Array) => through(bytes, new CompressionStream("deflate-raw"));
const inflate = (bytes: Uint8Array) => through(bytes, new DecompressionStream("deflate-raw"));

// MS-DOS date/time, the only timestamp a plain zip entry carries: 2-second
// resolution, years counted from 1980.
function dosTime(d: Date): { time: number; date: number } {
  return {
    time: (d.getHours() << 11) | (d.getMinutes() << 5) | (d.getSeconds() >> 1),
    date: ((Math.max(1980, d.getFullYear()) - 1980) << 9) | ((d.getMonth() + 1) << 5) | d.getDate(),
  };
}

/// Build a zip archive. Files are deflated; directory entries are empty and
/// stored.
export async function writeZip(entries: ZipEntry[]): Promise<Uint8Array> {
  const encoder = new TextEncoder();
  const { time, date } = dosTime(new Date());
  // Local records first (each with its header), then the central directory
  // describing them, then the end record pointing at the directory.
  const local: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;

  for (const entry of entries) {
    const isDir = entry.data === null;
    const name = encoder.encode(isDir ? withSlash(entry.name) : entry.name);
    const raw = entry.data ?? new Uint8Array(0);
    const body = isDir || raw.length === 0 ? raw : await deflate(raw);
    const method = isDir || raw.length === 0 ? STORED : DEFLATED;

    const header = new Uint8Array(30 + name.length);
    const h = new DataView(header.buffer);
    h.setUint32(0, LOCAL_SIG, true);
    h.setUint16(4, 20, true); // version needed: 2.0 (deflate)
    h.setUint16(6, FLAG_UTF8, true);
    h.setUint16(8, method, true);
    h.setUint16(10, time, true);
    h.setUint16(12, date, true);
    h.setUint32(14, crc32(raw), true);
    h.setUint32(18, body.length, true);
    h.setUint32(22, raw.length, true);
    h.setUint16(26, name.length, true);
    h.setUint16(28, 0, true); // no extra field
    header.set(name, 30);
    local.push(header, body);

    const cd = new Uint8Array(46 + name.length);
    const c = new DataView(cd.buffer);
    c.setUint32(0, CD_SIG, true);
    c.setUint16(4, 20, true); // version made by
    c.setUint16(6, 20, true); // version needed
    c.setUint16(8, FLAG_UTF8, true);
    c.setUint16(10, method, true);
    c.setUint16(12, time, true);
    c.setUint16(14, date, true);
    c.setUint32(16, crc32(raw), true);
    c.setUint32(20, body.length, true);
    c.setUint32(24, raw.length, true);
    c.setUint16(28, name.length, true);
    // External attributes: the MS-DOS directory bit, so readers that ignore the
    // trailing slash still treat the entry as a folder.
    c.setUint32(38, isDir ? 0x10 : 0, true);
    c.setUint32(42, offset, true);
    cd.set(name, 46);
    central.push(cd);

    offset += header.length + body.length;
  }

  const cdSize = central.reduce((n, c) => n + c.length, 0);
  const end = new Uint8Array(22);
  const e = new DataView(end.buffer);
  e.setUint32(0, EOCD_SIG, true);
  e.setUint16(8, entries.length, true); // entries on this disk
  e.setUint16(10, entries.length, true); // entries total
  e.setUint32(12, cdSize, true);
  e.setUint32(16, offset, true);
  return concat([...local, ...central, end]);
}

/// Read an archive's members. Throws on anything that isn't a zip we can read,
/// so the caller can report "not a vault archive" instead of importing nothing.
export async function readZip(bytes: Uint8Array): Promise<ZipEntry[]> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const eocd = findEocd(view);
  const count = view.getUint16(eocd + 10, true);
  let cursor = view.getUint32(eocd + 16, true);
  const decoder = new TextDecoder();
  const out: ZipEntry[] = [];

  for (let i = 0; i < count; i++) {
    if (view.getUint32(cursor, true) !== CD_SIG) throw new Error("corrupt zip directory");
    const method = view.getUint16(cursor + 10, true);
    const compressed = view.getUint32(cursor + 20, true);
    const size = view.getUint32(cursor + 24, true);
    const nameLen = view.getUint16(cursor + 28, true);
    const extraLen = view.getUint16(cursor + 30, true);
    const commentLen = view.getUint16(cursor + 32, true);
    const localAt = view.getUint32(cursor + 42, true);
    const name = decoder.decode(bytes.subarray(cursor + 46, cursor + 46 + nameLen));
    cursor += 46 + nameLen + extraLen + commentLen;

    if (compressed === 0xffffffff || size === 0xffffffff) throw new Error("zip64 is not supported");
    if (name.endsWith("/")) {
      out.push({ name, data: null });
      continue;
    }
    // The local header repeats the name and can carry a different extra field,
    // so the data offset has to be read from it rather than assumed.
    if (view.getUint32(localAt, true) !== LOCAL_SIG) throw new Error("corrupt zip entry");
    const start =
      localAt + 30 + view.getUint16(localAt + 26, true) + view.getUint16(localAt + 28, true);
    const body = bytes.subarray(start, start + compressed);
    if (method === STORED) out.push({ name, data: body });
    else if (method === DEFLATED) out.push({ name, data: await inflate(body) });
    else throw new Error(`unsupported zip compression method ${method}`);
  }
  return out;
}

// The end-of-central-directory record sits at the tail, after a comment of up
// to 64 KiB — so it has to be searched for, backwards from the end.
function findEocd(view: DataView): number {
  const min = Math.max(0, view.byteLength - 0xffff - 22);
  for (let i = view.byteLength - 22; i >= min; i--) {
    if (view.getUint32(i, true) === EOCD_SIG) return i;
  }
  throw new Error("not a zip archive");
}

const withSlash = (name: string) => (name.endsWith("/") ? name : `${name}/`);

function concat(chunks: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}
