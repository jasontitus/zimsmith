import { test, expect } from "@playwright/test";
import {
  mkdtempSync,
  openSync,
  writeSync,
  readSync,
  fstatSync,
  ftruncateSync,
  fsyncSync,
  closeSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { writeZim, verifyZim } from "../src/browser/zim";
import type { SyncFile } from "../src/browser/storage";

test("browser ZIM opens at a chosen website page and retains searchable contents", async () => {
  const root = mkdtempSync(join(tmpdir(), "zimsmith-website-"));
  const fd = openSync(join(root, "archive.zim"), "w+");
  const file: SyncFile = {
    write: (data, options) =>
      writeSync(
        fd,
        new Uint8Array(data.buffer, data.byteOffset, data.byteLength),
        0,
        data.byteLength,
        options?.at ?? 0,
      ),
    read: (data, options) =>
      readSync(
        fd,
        new Uint8Array(data.buffer, data.byteOffset, data.byteLength),
        0,
        data.byteLength,
        options?.at ?? 0,
      ),
    getSize: () => fstatSync(fd).size,
    truncate: (size) => ftruncateSync(fd, size),
    flush: () => fsyncSync(fd),
    close: () => closeSync(fd),
  };
  const check = () => {};
  const progress = async () => {};
  try {
    await writeZim(
      file,
      [
        {
          path: "home.html",
          title: "Home",
          mime: "text/html",
          content: new Blob([
            "<h1>Original website</h1><a href='child.html'>Child</a>",
          ]),
        },
        {
          path: "child.html",
          title: "Child",
          mime: "text/html",
          content: new Blob(["<h1>Child</h1><a href='home.html'>Home</a>"]),
        },
        {
          path: "index.html",
          title: "Contents",
          mime: "text/html",
          content: new Blob([
            "<script id='catalog' type='application/json'>{}</script><script src='search.js'></script>",
          ]),
        },
      ],
      check,
      progress,
      "home.html",
    );
    await verifyZim(file, check, progress, "home.html");
    // Read the header's main-page dirent without using the writer's item array.
    const header = new Uint8Array(80);
    file.read(header, { at: 0 });
    const h = new DataView(header.buffer),
      table = Number(h.getBigUint64(32, true)),
      main = h.getUint32(64, true);
    const pointer = new Uint8Array(8);
    file.read(pointer, { at: table + main * 8 });
    const entry = new Uint8Array(128);
    file.read(entry, {
      at: Number(new DataView(pointer.buffer).getBigUint64(0, true)),
    });
    expect(new TextDecoder().decode(entry.subarray(16)).split("\0")[0]).toBe(
      "home.html",
    );
    await expect(
      verifyZim(file, check, progress, "absent.html"),
    ).rejects.toThrow("starting page");
  } finally {
    file.close();
    rmSync(root, { recursive: true, force: true });
  }
});
