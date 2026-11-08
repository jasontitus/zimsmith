import { test, expect } from "@playwright/test";
import { createServer, type ViteDevServer } from "vite";
import { gzipSync, deflateSync } from "node:zlib";
import { File } from "node:buffer";

let server: ViteDevServer, BrowserEngine: any, diskName: any;
test.beforeAll(async () => {
  server = await createServer({ server: { middlewareMode: true, ws: false } });
  ({ BrowserEngine } = await server.ssrLoadModule("/src/browser/engine.ts"));
  ({ diskName } = await server.ssrLoadModule("/src/browser/imports.ts"));
});
test.afterAll(async () => {
  await server?.close();
});
function record(url: string, mime: string, bytes: Buffer, kind = "resource") {
  return Buffer.concat([
    Buffer.from(
      `WARC/1.1\r\nWARC-Type: ${kind}\r\nWARC-Record-ID: <urn:uuid:00000000-0000-0000-0000-000000000001>\r\nWARC-Date: 2026-01-01T00:00:00Z\r\nWARC-Target-URI: ${url}\r\nContent-Type: ${mime}\r\nContent-Length: ${bytes.length}\r\n\r\n`,
    ),
    bytes,
    Buffer.from("\r\n\r\n"),
  ]);
}
async function fixture(bytes: Buffer, compressed = false) {
  const name = compressed ? "fixture.warc.gz" : "fixture.warc";
  const files = new Map<string, Buffer>([[await diskName(name), bytes]]);
  const dir = {
    getFileHandle: async (key: string, options?: { create?: boolean }) => {
      if (!files.has(key) && !options?.create) throw new Error("Missing file");
      if (!files.has(key)) files.set(key, Buffer.alloc(0));
      return {
        getFile: async () => new File([files.get(key)!], key),
        createSyncAccessHandle: async () => ({
          truncate: (size: number) => files.set(key, Buffer.alloc(size)),
          write: (bytes: Uint8Array, { at }: { at: number }) => {
            const old = files.get(key)!;
            const next = Buffer.alloc(Math.max(old.length, at + bytes.length));
            old.copy(next);
            next.set(bytes, at);
            files.set(key, next);
            return bytes.length;
          },
          flush() {},
          close() {},
        }),
      };
    },
  };
  const store = { put: async () => {}, dir: async () => dir };
  const engine = new BrowserEngine(store);
  const job = {
    id: "warc",
    status: "importing",
    phase: "import",
    config: {
      url: "https://local-import.invalid/",
      title: "WARC fixture",
      kind: "website",
      max_items: 10,
      max_resource_mb: 2,
      max_depth: 1,
      delay_ms: 1000,
      language: "eng",
      description: "fixture",
      creator: "fixture",
      include_assets: true,
    },
    logs: [],
    entries: [],
    resources: {},
    import: {
      files: [{ path: name, size: bytes.length }],
      finished: false,
      file_count: 1,
    },
  };
  engine.jobs.set(job.id, job);
  return { engine, job, files, store };
}
for (const compressed of [false, true]) {
  test(`browser streams ${compressed ? "multi-member gzip" : "plain"} WARC preserving URLs and HTTP decoding`, async () => {
    const html = Buffer.from("<title>Captured page</title><img src='/image'>");
    const zipped = gzipSync(html);
    const http = Buffer.concat([
      Buffer.from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: Chunked\r\nContent-Encoding: X-Gzip\r\n\r\n",
      ),
      Buffer.from(zipped.length.toString(16) + "\r\n"),
      zipped,
      Buffer.from("\r\n0\r\n\r\n"),
    ]);
    const records = [
      record("https://capture.invalid/", "application/http", http, "response"),
      record(
        "https://capture.invalid/image",
        "image/svg+xml",
        Buffer.from("<svg xmlns='http://www.w3.org/2000/svg'/>"),
      ),
    ];
    const { engine, job, files } = await fixture(
      Buffer.concat(records.map((r) => (compressed ? gzipSync(r) : r))),
      compressed,
    );
    await engine.call("finish_import", { id: job.id });
    expect(job.status).toBe("review");
    const result: any = job;
    expect(result.entries[0].url).toBe("https://capture.invalid/");
    expect(result.entries[0].title).toBe("Captured page");
    expect(result.resources["https://capture.invalid/image"].mime).toBe(
      "image/svg+xml",
    );
    expect(files.get(result.entries[0].file)).toEqual(html);
  });
}
test("browser WARC rejects truncated records, oversized headers, and decoded bombs", async () => {
  const truncated = record(
    "https://capture.invalid/",
    "text/html",
    Buffer.from("<title>Page</title>"),
  ).subarray(0, -8);
  const hugeHeader = Buffer.from(
    "WARC/1.1\r\nX-Header: " + "x".repeat(70 * 1024),
  );
  const http = Buffer.concat([
    Buffer.from(
      "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\n\r\n",
    ),
    gzipSync(Buffer.alloc(3 * 1024 * 1024, 120)),
  ]);
  for (const bytes of [
    truncated,
    hugeHeader,
    record(
      "https://capture.invalid/",
      "application/http",
      Buffer.from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 999\r\n\r\n<title>Short</title>",
      ),
      "response",
    ),
    record("https://capture.invalid/", "application/http", http, "response"),
  ]) {
    const { engine, job } = await fixture(bytes);
    await expect(
      engine.call("finish_import", { id: job.id }),
    ).rejects.toThrow();
    expect(job.status).toBe("failed");
    expect(job.import.finished).toBe(false);
  }
});

test("browser WARC uses the last complete capture and reports unresolved revisits", async () => {
  const { engine, job } = await fixture(
    Buffer.concat([
      record(
        "https://capture.invalid/",
        "text/html",
        Buffer.from("<title>Old</title>"),
      ),
      record(
        "https://capture.invalid/",
        "text/html",
        Buffer.from("<title>Latest</title>"),
      ),
      record(
        "https://capture.invalid/missing",
        "application/http",
        Buffer.alloc(0),
        "revisit",
      ),
    ]),
  );
  await engine.call("finish_import", { id: job.id });
  expect(job.entries).toHaveLength(1);
  expect((job.entries[0] as any).title).toBe("Latest");
  expect(job.logs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        message: expect.stringContaining("WARC import omitted 1"),
      }),
    ]),
  );
});

test("browser WARC rejects damaged compression and chunk framing instead of importing raw bytes", async () => {
  for (const payload of [
    "xyz\r\n<title>bad</title>",
    "5\r\nabc",
    "3\r\nabc\r\n",
    "3\r\nabc\r\n0\r\n\r\nextra",
  ]) {
    const http = Buffer.from(
      "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\n\r\n" +
        payload,
    );
    const { engine, job } = await fixture(
      record("https://capture.invalid/", "application/http", http, "response"),
    );
    await expect(
      engine.call("finish_import", { id: job.id }),
    ).rejects.toThrow();
    expect(job.import.finished).toBe(false);
  }
  const good = record(
    "https://capture.invalid/",
    "text/html",
    Buffer.from("<title>Page</title>"),
  );
  const { engine, job } = await fixture(gzipSync(good).subarray(0, -8), true);
  await expect(engine.call("finish_import", { id: job.id })).rejects.toThrow();
  expect(job.import.finished).toBe(false);
});

test("browser WARC decodes deflate and chunk trailers", async () => {
  const html = Buffer.from("<title>Decoded</title>");
  const body = deflateSync(html);
  const http = Buffer.concat([
    Buffer.from(
      "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: deflate\r\nTransfer-Encoding: chunked\r\n\r\n",
    ),
    Buffer.from(body.length.toString(16) + ";ext=value\r\n"),
    body,
    Buffer.from("\r\n0\r\nX-Fixture: complete\r\n\r\n"),
  ]);
  const { engine, job, files } = await fixture(
    record("https://capture.invalid/", "application/http", http, "response"),
  );
  await engine.call("finish_import", { id: job.id });
  expect(files.get((job.entries[0] as any).file)).toEqual(html);
});

test("browser WARC pause keeps uploaded data and reselect retries after the finalizer exits", async () => {
  const bytes = record(
    "https://capture.invalid/",
    "text/html",
    Buffer.from("<title>Retained</title>"),
  );
  const { engine, job, store } = await fixture(bytes);
  let entered!: () => void, release!: () => void;
  const reached = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let first = true;
  store.put = async () => {
    if (first) {
      first = false;
      entered();
      await held;
    }
  };
  const finishing = engine.call("finish_import", { id: job.id });
  const rejected = expect(finishing).rejects.toThrow();
  await reached;
  await engine.call("pause_job", { id: job.id });
  expect(job.status).toBe("paused");
  await expect(
    engine.call("begin_import", {
      existingId: job.id,
      config: job.config,
      files: job.import.files,
    }),
  ).rejects.toThrow();
  release();
  await rejected;
  expect(job.import.finished).toBe(false);
  await engine.call("begin_import", {
    existingId: job.id,
    config: job.config,
    files: job.import.files,
  });
  expect(
    await engine.call("import_offset", { id: job.id, path: "fixture.warc" }),
  ).toBe(bytes.length);
  await engine.call("finish_import", { id: job.id });
  expect(job.status).toBe("review");
  expect((job.entries[0] as any).title).toBe("Retained");
});

test("multiple browser WARCs share expansion budget even for ignored metadata", async () => {
  const bytes = gzipSync(
    record(
      "https://capture.invalid/",
      "application/warc-fields",
      Buffer.alloc(1100 * 1024, 120),
      "warcinfo",
    ),
  );
  const { engine, job, files } = await fixture(bytes, true);
  files.set(await diskName("second.warc.gz"), bytes);
  job.import.files.push({ path: "second.warc.gz", size: bytes.length });
  (job.config as any).capture = { max_download_mb: 2 };
  await expect(engine.call("finish_import", { id: job.id })).rejects.toThrow(
    /budget/,
  );
  expect(job.import.finished).toBe(false);
});
