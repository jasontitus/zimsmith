// Browser implementation of the ZIM 6.3 layout used by zimru (MIT). It writes
// standard raw clusters, including extended offsets for large blobs. No WASM,
// native process, whole-archive buffer, or virtual filesystem is required.
import SparkMD5 from "spark-md5";
import type { SyncFile } from "./storage";

export interface ZimItem {
  path: string;
  title: string;
  mime: string;
  namespace?: string;
  content: Blob;
}
const encoder = new TextEncoder();
const text = (s: string) => encoder.encode(s);
function numbers(values: number[], width: 4 | 8) {
  const data = new Uint8Array(values.length * width),
    view = new DataView(data.buffer);
  values.forEach((n, i) =>
    width === 8
      ? view.setBigUint64(i * 8, BigInt(n), true)
      : view.setUint32(i * 4, n, true),
  );
  return data;
}
function compare(a: string, b: string) {
  const x = text(a),
    y = text(b);
  for (let i = 0; i < Math.min(x.length, y.length); i++)
    if (x[i] !== y[i]) return x[i] - y[i];
  return x.length - y.length;
}
export async function writeZim(
  file: SyncFile,
  source: ZimItem[],
  check: () => void,
  progress: (done: number, total: number) => Promise<void>,
) {
  const items = [...source];
  const listing = {
    path: "listing/titleOrdered/v1",
    title: "listing/titleOrdered/v1",
    mime: "application/octet-stream",
    namespace: "X",
    content: new Blob(),
  };
  items.push(listing);
  items.sort(
    (a, b) =>
      compare(a.namespace || "C", b.namespace || "C") ||
      compare(a.path, b.path),
  );
  const keys = new Set<string>();
  for (const item of items) {
    const key = `${item.namespace || "C"}/${item.path}`;
    if (
      keys.has(key) ||
      /\0/.test(item.path + item.title + item.mime) ||
      !item.path ||
      (item.namespace || "C").length !== 1
    )
      throw new Error("Invalid or duplicate ZIM entry.");
    keys.add(key);
  }
  const order = items
    .map((_, i) => i)
    .sort(
      (a, b) =>
        compare(items[a].namespace || "C", items[b].namespace || "C") ||
        compare(items[a].title, items[b].title) ||
        a - b,
    );
  listing.content = new Blob([
    numbers(
      order.filter(
        (i) =>
          (items[i].namespace || "C") === "C" &&
          items[i].mime.split(";")[0] === "text/html",
      ),
      4,
    ),
  ]);
  const mimes = [...new Set(items.map((i) => i.mime))];
  const mimeBytes = text(mimes.join("\0") + "\0\0");
  let position = 0;
  function write(bytes: Uint8Array, at = position) {
    check();
    if (file.write(bytes, { at }) !== bytes.length)
      throw new Error("The browser could not write the complete ZIM file.");
    if (at === position) position += bytes.length;
  }
  file.truncate(0);
  write(new Uint8Array(80));
  write(mimeBytes);
  const clusters: number[] = [];
  for (let i = 0; i < items.length; i++) {
    const blob = items[i].content,
      extended = blob.size + 8 > 0xffffffff,
      width = extended ? 8 : 4;
    clusters.push(position);
    write(Uint8Array.of(extended ? 0x11 : 0x01));
    write(numbers([width * 2, width * 2 + blob.size], width));
    for (let start = 0; start < blob.size; start += 1024 * 1024) {
      write(
        new Uint8Array(
          await blob.slice(start, start + 1024 * 1024).arrayBuffer(),
        ),
      );
      await progress(i, items.length);
    }
    await progress(i + 1, items.length);
  }
  const dirents = items.map((item, i) => {
    const name = text(item.path + "\0" + item.title + "\0"),
      bytes = new Uint8Array(16 + name.length),
      view = new DataView(bytes.buffer);
    view.setUint16(0, mimes.indexOf(item.mime), true);
    bytes[3] = (item.namespace || "C").charCodeAt(0);
    view.setUint32(8, i, true);
    bytes.set(name, 16);
    return bytes;
  });
  const urlPos = position,
    titlePos = urlPos + items.length * 8,
    clusterPos = titlePos + items.length * 4;
  let cursor = clusterPos + clusters.length * 8;
  const offsets = dirents.map((d) => {
    const value = cursor;
    cursor += d.length;
    return value;
  });
  write(numbers(offsets, 8));
  write(numbers(order, 4));
  write(numbers(clusters, 8));
  for (const dirent of dirents) write(dirent);
  const checksumPos = position,
    header = new Uint8Array(80),
    view = new DataView(header.buffer);
  view.setUint32(0, 0x044d495a, true);
  view.setUint16(4, 6, true);
  view.setUint16(6, 3, true);
  crypto.getRandomValues(header.subarray(8, 24));
  header[14] = (header[14] & 15) | 64;
  header[16] = (header[16] & 63) | 128;
  view.setUint32(24, items.length, true);
  view.setUint32(28, clusters.length, true);
  for (const [at, n] of [
    [32, urlPos],
    [40, titlePos],
    [48, clusterPos],
    [56, 80],
    [72, checksumPos],
  ])
    view.setBigUint64(at, BigInt(n), true);
  const main = items.findIndex(
    (i) => (i.namespace || "C") === "C" && i.path === "index.html",
  );
  if (main < 0) throw new Error("ZIM contents page is missing.");
  view.setUint32(64, main, true);
  view.setUint32(68, 0xffffffff, true);
  write(header, 0);
  file.flush();
  const md5 = new SparkMD5.ArrayBuffer(),
    buffer = new Uint8Array(1024 * 1024);
  for (let at = 0; at < checksumPos; at += buffer.length) {
    check();
    const part = buffer.subarray(0, Math.min(buffer.length, checksumPos - at));
    if (file.read(part, { at }) !== part.length)
      throw new Error("ZIM verification encountered a truncated file.");
    md5.append(part.slice().buffer);
    await progress(items.length, items.length);
  }
  write(Uint8Array.from(md5.end(true), (c) => c.charCodeAt(0)));
  file.flush();
  if (file.getSize() !== checksumPos + 16)
    throw new Error("ZIM length verification failed.");
  return file.getSize();
}

// Re-read the finished file, independent of the writer's in-memory tables.
// Only directory metadata and a fixed checksum buffer enter memory.
export async function verifyZim(
  file: SyncFile,
  check: () => void,
  progress: (done: number, total: number) => Promise<void>,
) {
  const size = file.getSize(),
    header = new Uint8Array(80);
  if (size < 96 || file.read(header, { at: 0 }) !== 80)
    throw new Error("Finished ZIM header is truncated.");
  const h = new DataView(header.buffer);
  if (
    h.getUint32(0, true) !== 0x044d495a ||
    h.getUint16(4, true) !== 6 ||
    h.getUint16(6, true) !== 3
  )
    throw new Error("Finished file is not a supported ZIM.");
  const number = (view: DataView, at: number) => {
    const n = view.getBigUint64(at, true);
    if (n > BigInt(size) || n > BigInt(Number.MAX_SAFE_INTEGER))
      throw new Error("ZIM table offset is invalid.");
    return Number(n);
  };
  const entries = h.getUint32(24, true),
    clusters = h.getUint32(28, true),
    urlPos = number(h, 32),
    titlePos = number(h, 40),
    clusterPos = number(h, 48),
    mimePos = number(h, 56),
    end = number(h, 72),
    main = h.getUint32(64, true);
  if (
    !entries ||
    entries > 100000 ||
    !clusters ||
    clusters > 100000 ||
    main >= entries ||
    end + 16 !== size ||
    urlPos + entries * 8 > end ||
    titlePos + entries * 4 > end ||
    clusterPos + clusters * 8 > end ||
    mimePos < 80
  )
    throw new Error("ZIM directory tables are out of bounds.");
  const read = (at: number, length: number) => {
    if (at < 80 || at + length > end)
      throw new Error("ZIM directory points outside the file.");
    const bytes = new Uint8Array(length);
    if (file.read(bytes, { at }) !== length)
      throw new Error("ZIM directory is truncated.");
    return bytes;
  };
  const string = (start: number) => {
    let at = start,
      result = "";
    const decoder = new TextDecoder("utf-8", { fatal: true });
    for (let n = 0; n < 8192; n++) {
      const part = read(at, Math.min(128, end - at));
      const zero = part.indexOf(0);
      if (zero >= 0) {
        result += decoder.decode(part.subarray(0, zero), { stream: false });
        return { value: result, next: at + zero + 1 };
      }
      result += decoder.decode(part, { stream: true });
      at += part.length;
    }
    throw new Error("ZIM directory string exceeds its size limit.");
  };
  let mimeCount = 0,
    at = mimePos;
  while (mimeCount < 65535) {
    const item = string(at);
    at = item.next;
    if (!item.value) break;
    mimeCount++;
  }
  if (!mimeCount) throw new Error("ZIM has no MIME types.");
  const records: { key: string; title: string; namespace: string }[] = [];
  for (let i = 0; i < entries; i++) {
    check();
    const pointer = number(new DataView(read(urlPos + i * 8, 8).buffer), 0),
      item = new DataView(read(pointer, 16).buffer);
    if (
      item.getUint16(0, true) >= mimeCount ||
      item.getUint32(8, true) >= clusters
    )
      throw new Error("ZIM entry has an invalid MIME or cluster reference.");
    const path = string(pointer + 16),
      title = string(path.next),
      namespace = String.fromCharCode(item.getUint8(3)),
      key = namespace + "/" + path.value;
    if (i && compare(records[i - 1].key, key) >= 0)
      throw new Error("ZIM path index is not sorted.");
    records.push({ key, title: title.value, namespace });
    if (i % 500 === 0) await progress(i, entries);
  }
  if (records[main].key !== "C/index.html")
    throw new Error("ZIM is missing its contents page.");
  const seen = new Set<number>();
  let previous = -1;
  for (let i = 0; i < entries; i++) {
    const id = new DataView(read(titlePos + i * 4, 4).buffer).getUint32(
      0,
      true,
    );
    if (id >= entries || seen.has(id))
      throw new Error("ZIM title index is invalid.");
    seen.add(id);
    if (previous >= 0) {
      const a = records[previous],
        b = records[id];
      if (
        compare(a.namespace, b.namespace) > 0 ||
        (a.namespace === b.namespace && compare(a.title, b.title) > 0)
      )
        throw new Error("ZIM title index is not sorted.");
    }
    previous = id;
  }
  let previousCluster = 0;
  for (let i = 0; i < clusters; i++) {
    const position = number(
      new DataView(read(clusterPos + i * 8, 8).buffer),
      0,
    );
    if (position < mimePos || position <= previousCluster || position >= urlPos)
      throw new Error("ZIM cluster index is invalid.");
    previousCluster = position;
  }
  const hash = new SparkMD5.ArrayBuffer(),
    buffer = new Uint8Array(1024 * 1024);
  for (let offset = 0; offset < end; offset += buffer.length) {
    check();
    const part = buffer.subarray(0, Math.min(buffer.length, end - offset));
    if (file.read(part, { at: offset }) !== part.length)
      throw new Error("Finished ZIM is truncated.");
    hash.append(part.slice().buffer);
    await progress(offset + part.length, end);
  }
  const expected = new Uint8Array(16);
  if (
    file.read(expected, { at: end }) !== 16 ||
    hash.end(true) !== String.fromCharCode(...expected)
  )
    throw new Error("Finished ZIM checksum does not match.");
  return {
    checksum_verified: true,
    structure_verified: true,
    checked_at: Math.floor(Date.now() / 1000),
    archive_entries: entries,
  };
}
