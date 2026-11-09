import { expandWarc } from "./warc";
import {
  BlobReader,
  ZipReader,
  configure,
} from "@zip.js/zip.js/index-native.js";
import { assetAllowed, captureOptions, pageAllowed } from "../capture";
import type { Config } from "../types";
import { discover } from "./content";
import type { BrowserEntry, BrowserJob, Receipt } from "./model";
import { readFile, Store, syncFile, writeFile } from "./storage";
configure({ useWebWorkers: false });
export interface ImportFile {
  path: string;
  size: number;
}
export function logicalPath(value: string) {
  if (
    !value ||
    value.length > 1024 ||
    /^\//.test(value) ||
    /[\\:\x00-\x1f\x7f]/.test(value)
  )
    throw new Error("Unsupported or unsafe import path.");
  const parts = value.split("/").filter((part) => part && part !== ".");
  if (!parts.length || parts.includes(".."))
    throw new Error("Import paths cannot contain parent traversal.");
  return parts.join("/");
}
export async function diskName(path: string, prefix = "upload-") {
  const digest = new Uint8Array(
    await crypto.subtle.digest("SHA-256", new TextEncoder().encode(path)),
  );
  return (
    prefix +
    Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("")
  );
}
export function importLimit(config: Config) {
  return Math.min(
    20 * 1024 ** 3,
    (captureOptions(config).max_download_mb ?? 20480) * 1024 ** 2,
  );
}
export function approvedFiles(files: ImportFile[], config: Config) {
  if (!files.length || files.length > 50000)
    throw new Error("Choose 1–50000 files.");
  const names = new Set<string>();
  let size = 0;
  const normalized = files.map((file) => {
    const path = logicalPath(file.path);
    if (names.has(path.toLowerCase()))
      throw new Error(`Duplicate import filename: ${path}`);
    names.add(path.toLowerCase());
    if (
      !Number.isSafeInteger(file.size) ||
      file.size < 0 ||
      file.size >
        (/\.warc(?:\.gz)?$/i.test(path)
          ? importLimit(config)
          : config.max_resource_mb * 1024 ** 2)
    )
      throw new Error(`File exceeds the per-file limit: ${path}`);
    size += file.size;
    if (size > importLimit(config))
      throw new Error("Import exceeds its total budget (maximum 20 GB).");
    return { path, size: file.size };
  });
  return normalized.sort((a, b) => a.path.localeCompare(b.path));
}
export interface Payload {
  url?: string;
  mime?: string;
  path: string;
  file: string;
  size: number;
}
function mime(path: string) {
  const ext = path.toLowerCase().split(".").pop() || "";
  return (
    (
      {
        html: "text/html",
        htm: "text/html",
        xhtml: "application/xhtml+xml",
        css: "text/css",
        png: "image/png",
        jpg: "image/jpeg",
        jpeg: "image/jpeg",
        gif: "image/gif",
        webp: "image/webp",
        avif: "image/avif",
        svg: "image/svg+xml",
        woff: "font/woff",
        woff2: "font/woff2",
        ttf: "font/ttf",
        otf: "font/otf",
        mp4: "video/mp4",
        webm: "video/webm",
        mov: "video/quicktime",
        mp3: "audio/mpeg",
        m4a: "audio/mp4",
        ogg: "audio/ogg",
        wav: "audio/wav",
        pdf: "application/pdf",
        vtt: "text/vtt",
      } as Record<string, string>
    )[ext] || "application/octet-stream"
  );
}
export async function prepareImport(
  job: BrowserJob,
  store: Store,
  signal: AbortSignal,
  save: () => Promise<void>,
) {
  if (!job.import || job.import.finished)
    throw new Error("Import is already finished.");
  const dir = await store.dir(job.id),
    payloads: Payload[] = [],
    seen = new Set<string>();
  let total = 0,
    warcBudgetUsed = 0;
  const check = () => {
    if (signal.aborted)
      throw new Error("Import stopped. Reselect these files to continue.");
  };
  const add = (payload: Payload) => {
    check();
    payload.path = logicalPath(payload.path);
    if (seen.has(payload.path.toLowerCase()))
      throw new Error(`Imports contain overlapping paths: ${payload.path}`);
    seen.add(payload.path.toLowerCase());
    total += payload.size;
    if (payloads.length >= 50000 || total > importLimit(job.config))
      throw new Error("Expanded import exceeds its file or storage budget.");
    payloads.push(payload);
  };
  for (const upload of job.import.files || []) {
    check();
    const fileName = await diskName(upload.path),
      file = await readFile(dir, fileName);
    if (file.size !== upload.size)
      throw new Error(`Import file is incomplete: ${upload.path}`);
    if (/\.warc(?:\.gz)?$/i.test(upload.path)) {
      const result = await expandWarc(
        file,
        /\.gz$/i.test(upload.path),
        dir,
        importLimit(job.config) - warcBudgetUsed,
        job.config.max_resource_mb * 1024 ** 2,
        check,
        async (count) => {
          job.progress = {
            label: `Reading WARC: ${upload.path}`,
            completed: count,
            total: null,
            unit: "records",
            received: 0,
            expected: null,
            updated_at: Math.floor(Date.now() / 1000),
          };
          await save();
        },
      );
      warcBudgetUsed += result.budgetUsed;
      for (const payload of result.payloads) add(payload);
      if (result.skipped)
        job.logs.push({
          time: Math.floor(Date.now() / 1000),
          level: "warning",
          message: `WARC import omitted ${result.skipped} redirect, error, partial, revisit, conversion or segmented records. Only complete captured response/resource bodies are imported.`,
        });
      continue;
    }
    if (!/\.zip$/i.test(upload.path)) {
      add({ path: upload.path, file: fileName, size: file.size });
      continue;
    }
    const zip = new ZipReader(new BlobReader(file), {
      checkSignature: true,
      useWebWorkers: false,
    });
    try {
      let count = 0;
      for await (const entry of zip.getEntriesGenerator()) {
        check();
        if (++count > 50000) throw new Error("ZIP contains too many entries.");
        if (entry.encrypted || entry.symlink)
          throw new Error(
            "Encrypted ZIPs and symbolic links are not supported.",
          );
        if (entry.directory) continue;
        const path = logicalPath(entry.filename),
          target = await diskName(upload.path + "/" + path, "expanded-");
        if (
          !Number.isSafeInteger(entry.uncompressedSize) ||
          entry.uncompressedSize < 0 ||
          entry.uncompressedSize > job.config.max_resource_mb * 1024 ** 2 ||
          total + entry.uncompressedSize > importLimit(job.config)
        )
          throw new Error("Expanded ZIP entry exceeds the import budget.");
        const output = await syncFile(dir, target);
        let copied = 0;
        try {
          output.truncate(0);
          await entry.getData(
            new WritableStream<Uint8Array>({
              write(chunk) {
                check();
                if (copied + chunk.length > entry.uncompressedSize)
                  throw new Error("ZIP entry exceeded its declared size.");
                if (output.write(chunk, { at: copied }) !== chunk.length)
                  throw new Error("Browser storage returned a short write.");
                copied += chunk.length;
              },
            }),
            { signal, checkSignature: true, useWebWorkers: false },
          );
          if (copied !== entry.uncompressedSize)
            throw new Error("ZIP entry ended early.");
          output.flush();
        } finally {
          output.close();
        }
        add({ path, file: target, size: copied });
        job.progress = {
          label: `Extracting ZIP: ${upload.path}`,
          completed: count,
          total: null,
          unit: "files",
          received: copied,
          expected: entry.uncompressedSize,
          updated_at: Math.floor(Date.now() / 1000),
        };
        await save();
      }
    } finally {
      await zip.close();
    }
  }
  const entries: BrowserEntry[] = [],
    resources: Record<string, Receipt> = {};
  for (const [index, payload] of payloads.entries()) {
    check();
    const url =
      payload.url ||
      "https://local-import.invalid/" +
        payload.path.split("/").map(encodeURIComponent).join("/");
    const type = payload.mime || mime(payload.path),
      id = crypto.randomUUID();
    const receipt: Receipt = {
      file: payload.file,
      mime: type,
      size: payload.size,
      done: true,
    };
    resources[url] = receipt;
    if (type === "text/html" || type === "application/xhtml+xml") {
      if (payload.size > 8 * 1024 ** 2)
        throw new Error(`HTML exceeds the 8 MB parsing limit: ${payload.path}`);
      const page = discover(
        await (await readFile(dir, payload.file)).text(),
        url,
      );
      entries.push({
        id,
        url,
        title: page.title,
        text: page.text,
        depth: 0,
        selected: pageAllowed(job.config, url),
        state: "done",
        size: payload.size,
        duration: null,
        error: null,
        file: payload.file,
      });
    } else if (/^(video|audio)\//.test(type)) {
      entries.push({
        id,
        url,
        title: payload.path,
        text: "",
        depth: 0,
        selected: assetAllowed(job.config, url, type),
        state: "done",
        size: payload.size,
        duration: null,
        error: null,
        file: payload.file,
        media: `media-${id}.${payload.path.split(".").pop()}`,
        mime: type,
      });
    }
    job.progress = {
      label: "Preparing captured pages and assets",
      completed: index + 1,
      total: payloads.length,
      unit: "files",
      received: 0,
      expected: null,
      updated_at: Math.floor(Date.now() / 1000),
    };
  }
  if (!entries.length)
    throw new Error("No HTML, video, or audio files were found.");
  if (entries.length > job.config.max_items)
    throw new Error(
      "Import has more pages/media than the item limit. Raise the limit or choose fewer files.",
    );
  check();
  job.entries = entries;
  job.resources = resources;
  job.queue = [];
  job.visited = [];
  job.bytes = total;
  job.import.finished = true;
  job.import.file_count = job.import.files?.length || 0;
  job.import.files = [];
  job.phase = "scan";
  job.status = "review";
  job.error = null;
  await save();
}
export async function uploadChunk(
  job: BrowserJob,
  store: Store,
  path: string,
  offset: number,
  bytes: Uint8Array,
) {
  if (
    job.status !== "importing" ||
    !job.import ||
    job.import.finished ||
    bytes.length > 256 * 1024
  )
    throw new Error("Import is not accepting files.");
  const expected = job.import.files?.find((file) => file.path === path);
  if (!expected) throw new Error("Unapproved import path.");
  const file = await syncFile(await store.dir(job.id), await diskName(path));
  try {
    if (file.getSize() !== offset || offset + bytes.length > expected.size)
      throw new Error("Import offset changed. Reselect files to resume.");
    if (file.write(bytes, { at: offset }) !== bytes.length)
      throw new Error("Browser storage returned a short write.");
    file.flush();
    return offset + bytes.length;
  } finally {
    file.close();
  }
}
