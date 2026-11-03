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
