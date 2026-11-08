import { AsyncIterReader, WARCParser } from "warcio";
import pako from "pako";
import type { Payload } from "./imports";
import { diskName } from "./imports";
import { syncFile } from "./storage";

const headerLimit = 64 * 1024;
// Browser DecompressionStream rejects concatenated gzip members in Chromium.
// Use zlib's member decoder with small input slices and a bounded output queue;
// validate CRC/end-of-stream instead of warcio's permissive raw-data fallback.
async function* gunzip(
  source: AsyncIterable<Uint8Array>,
  cap: number,
  check: () => void,
) {
  type Member = pako.Inflate & { ended: boolean; strm: { avail_in: number } };
  let inflator: Member | undefined;
  let decoded = 0;
  const pending: Uint8Array[] = [];
  for await (const chunk of source) {
    for (let offset = 0; offset < chunk.length; offset += 1024) {
      let input = chunk.subarray(offset, offset + 1024);
      while (input.length) {
        check();
        if (!inflator || inflator.ended) {
          inflator = new pako.Inflate({
            windowBits: 31,
            chunkSize: 64 * 1024,
          }) as Member;
          inflator.onData = (bytes) => {
            check();
            decoded += bytes.length;
            if (decoded > cap)
              throw new Error("Expanded gzip exceeds the import budget.");
            pending.push(bytes as Uint8Array);
          };
        }
        inflator.push(input, false);
        if (inflator.err)
          throw new Error(`Invalid gzip capture: ${inflator.msg}`);
        // pako exposes unconsumed member input here, but its type declarations omit it.
        const remaining = inflator.strm.avail_in;
        if (remaining && !inflator.ended)
          throw new Error("Gzip decoder made no progress.");
        input = input.subarray(input.length - remaining);
        while (pending.length) yield pending.shift()!;
      }
    }
  }
  if (!inflator?.ended || inflator.err)
    throw new Error("Truncated gzip capture.");
}
// Validate framing before the third-party parser can accumulate an unbounded
// header. Bodies stream through without buffering an entire record or archive.
async function* boundedRecords(
  file: File,
  gzip: boolean,
  cap: number,
  fileCap: number,
  check: () => void,
  budget: { expanded: number },
) {
  let header: number[] = [],
    bodyLeft = 0,
    trailer = 0,
    total = 0,
    count = 0;
  let httpHeader = false,
    httpBytes = 0,
    httpTail = "",
    httpText = "";
  const decoder = new TextDecoder();
  const raw = new AsyncIterReader(file.stream(), null);
  const stream = gzip ? gunzip(raw, cap, check) : raw;
  for await (const chunk of stream) {
    check();
    total += chunk.length;
    budget.expanded = total;
    if (total > cap)
      throw new Error("Expanded WARC exceeds the import budget.");
    for (let i = 0; i < chunk.length;) {
      if (bodyLeft > 0) {
        if (httpHeader) {
          httpText += String.fromCharCode(chunk[i++]);
          httpTail = httpText.slice(-4);
          bodyLeft--;
          httpBytes++;
          if (httpBytes > headerLimit)
            throw new Error("HTTP header exceeds 64 KB.");
          if (httpTail === "\r\n\r\n") {
            httpHeader = false;
            if (!/^Transfer-Encoding:\s*chunked\s*$/im.test(httpText)) {
              const lengths = [
                ...httpText.matchAll(/^Content-Length:[ \t]*([^\r\n]*)/gim),
              ];
              if (
                lengths.length > 1 ||
                (lengths.length === 1 &&
                  (!/^\d+$/.test(lengths[0][1].trim()) ||
                    Number(lengths[0][1]) !== bodyLeft))
              )
                throw new Error(
                  "HTTP content length does not match the WARC record.",
                );
            }
          }
        } else {
          const n = Math.min(bodyLeft, chunk.length - i);
          bodyLeft -= n;
          i += n;
        }
      } else if (trailer > 0) {
        const expected = trailer % 2 === 0 ? 13 : 10;
        if (chunk[i++] !== expected)
          throw new Error("Invalid WARC record terminator.");
        trailer--;
      } else {
        header.push(chunk[i++]);
        if (header.length > headerLimit)
          throw new Error("WARC header exceeds 64 KB.");
        const n = header.length;
        if (
          n < 4 ||
          header[n - 4] !== 13 ||
          header[n - 3] !== 10 ||
          header[n - 2] !== 13 ||
          header[n - 1] !== 10
        )
          continue;
        const text = decoder.decode(new Uint8Array(header));
        const lengths = [...text.matchAll(/^Content-Length:\s*(\d+)\s*$/gim)];
        if (!/^WARC\/1\.[01]\r\n/.test(text) || lengths.length !== 1)
          throw new Error("Invalid WARC version or content length.");
        bodyLeft = Number(lengths[0][1]);
        if (!Number.isSafeInteger(bodyLeft) || bodyLeft > fileCap + headerLimit)
          throw new Error("WARC record exceeds the per-file limit.");
        if (++count > 50000) throw new Error("WARC contains too many records.");
        httpHeader = /^WARC-Type:\s*response\s*$/im.test(text);
        httpBytes = 0;
        httpTail = "";
        httpText = "";
        trailer = 4;
        header = [];
      }
    }
    yield chunk;
  }
  if (header.length || bodyLeft || trailer)
    throw new Error("Truncated WARC record.");
}

// warcio's replay decoder deliberately tolerates malformed chunking and may
// buffer an entire chunk. Imports instead require complete bodies and bound
// each allocation, even when an HTTP chunk spans a large video.
async function* unchunk(source: AsyncIterable<Uint8Array>, check: () => void) {
  const reader = new AsyncIterReader(source, null);
  const decoder = new TextDecoder();
  async function line() {
    check();
    const bytes = await reader.readlineRaw(headerLimit);
    if (
      !bytes ||
      bytes.length < 2 ||
      bytes.at(-2) !== 13 ||
      bytes.at(-1) !== 10
    )
      throw new Error("Invalid or truncated HTTP chunk header.");
    return decoder.decode(bytes);
  }
  while (true) {
    const text = await line();
    const match = /^([0-9a-f]+)(?:;[^\r\n]*)?\r\n$/i.exec(text);
    const size = match ? Number.parseInt(match[1], 16) : NaN;
    if (!Number.isSafeInteger(size) || size < 0)
      throw new Error("Invalid HTTP chunk size.");
    if (size === 0) {
      let trailerSize = 0;
      while (true) {
        const trailer = await line();
        trailerSize += trailer.length;
        if (trailerSize > headerLimit)
          throw new Error("HTTP trailers exceed 64 KB.");
        if (trailer === "\r\n") break;
        if (!/^[^:\s]+:/.test(trailer))
          throw new Error("Invalid HTTP trailer.");
      }
      if ((await reader.readSize(1)).length)
        throw new Error("Extra data after HTTP chunked body.");
      return;
    }
    let left = size;
    while (left) {
      check();
      const chunk = await reader.readSize(Math.min(left, 64 * 1024));
      if (!chunk.length) throw new Error("Truncated HTTP chunk.");
      left -= chunk.length;
      yield chunk;
    }
    const separator = await reader.readSize(2);
    if (separator[0] !== 13 || separator[1] !== 10)
      throw new Error("Invalid HTTP chunk terminator.");
  }
}

async function* decodedBody(
  source: AsyncIterable<Uint8Array> | Iterable<Uint8Array>,
  chunked: boolean,
  encoding: string,
  check: () => void,
  cap: number,
) {
  const raw = (async function* () {
    yield* source;
  })();
  const body = chunked ? unchunk(raw, check) : raw;
  if (!encoding || encoding === "identity") {
    yield* body;
    return;
  }
  if (encoding === "gzip" || encoding === "x-gzip") {
    yield* gunzip(body, cap, check);
    return;
  }
  const iterator = body[Symbol.asyncIterator]();
  const stream = new ReadableStream<Uint8Array>({
    async pull(controller) {
      check();
      const next = await iterator.next();
      if (next.done) controller.close();
      else controller.enqueue(next.value);
    },
    async cancel() {
      await iterator.return?.();
    },
  });
  const reader = stream
    .pipeThrough(
      new DecompressionStream(encoding === "deflate" ? "deflate" : "gzip"),
    )
    .getReader();
  try {
    while (true) {
      check();
      const next = await reader.read();
      if (next.done) break;
      yield next.value;
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

export async function expandWarc(
  file: File,
  gzip: boolean,
  dir: FileSystemDirectoryHandle,
  cap: number,
  fileCap: number,
  check: () => void,
  progress: (count: number) => Promise<void>,
) {
  const budget = { expanded: 0 };
  const source = new AsyncIterReader(
    boundedRecords(file, gzip, cap, fileCap, check, budget),
    null,
  );
  const payloads = new Map<string, Payload>();
  let total = 0,
    count = 0,
    skipped = 0;
  for await (const record of new WARCParser(source)) {
    check();
    await progress(++count);
    if (!["response", "resource"].includes(record.warcType)) {
      if (["revisit", "continuation", "conversion"].includes(record.warcType))
        skipped++;
      continue;
    }
    const status = Number(record.httpHeaders?.statusCode);
    if (
      record.warcHeader("WARC-Truncated") ||
      record.warcHeader("WARC-Segment-Number") ||
      (record.warcType === "response" &&
        (!(status >= 200 && status < 300) || status === 206))
    ) {
      skipped++;
      continue;
    }
    const url = new URL((record.warcTargetURI || "").replace(/^<|>$/g, ""));
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password
    )
      throw new Error(
        "WARC target must be an HTTP(S) URL without credentials.",
      );
    url.hash = "";
    const encoding =
      record.httpHeaders?.headers.get("content-encoding")?.toLowerCase() || "";
    if (!["", "identity", "gzip", "x-gzip", "deflate"].includes(encoding))
      throw new Error(`Unsupported WARC HTTP content encoding: ${encoding}`);
    const transfer =
      record.httpHeaders?.headers.get("transfer-encoding")?.toLowerCase() || "";
    if (!["", "identity", "chunked"].includes(transfer))
      throw new Error("Unsupported WARC HTTP transfer encoding.");
    const mime = (
      record.httpHeaders?.headers.get("content-type") ||
      record.warcContentType ||
      "application/octet-stream"
    )
      .split(";")[0]
      .trim()
      .toLowerCase();
    const name = await diskName(url.href, "warc-");
    const output = await syncFile(dir, name);
    let size = 0;
    try {
      output.truncate(0);
      for await (const chunk of decodedBody(
        record.reader,
        transfer === "chunked",
        encoding,
        check,
        Math.min(fileCap, cap - total),
      )) {
        check();
        total += chunk.length;
        if (size + chunk.length > fileCap || total > cap)
          throw new Error(
            "Decoded WARC exceeds its file or total import budget.",
          );
        if (output.write(chunk, { at: size }) !== chunk.length)
          throw new Error("Browser storage returned a short write.");
        size += chunk.length;
      }
      output.flush();
    } finally {
      output.close();
    }
    payloads.set(url.href, {
      path: name,
      file: name,
      size,
      url: url.href,
      mime,
    });
  }
  return {
    payloads: [...payloads.values()],
    skipped,
    budgetUsed: Math.max(budget.expanded, total),
  };
}
