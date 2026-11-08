import { assetAllowed, captureOptions } from "../capture";
import robotsParser from "robots-parser";
import { now, type BrowserJob, type Receipt } from "./model";
import { readFile, Store, syncFile } from "./storage";
import { requestDelay } from "./pacing";

interface Host {
  // Cooldown deadline. Older saved hosts also used this for ordinary pacing.
  next: number;
  lastRequest?: number;
  robots?: string;
  robots_at?: number;
}
export class Paused extends Error {}
export class ExcludedAsset extends Error {}
class RobotsDenied extends Error {}

export const unavailableAsset = (reason: string) =>
  reason.startsWith("robots.txt disallows ") || /^HTTP (404|410) /.test(reason);
export class Network {
  private hosts = new Map<string, Host>();
  constructor(
    private store: Store,
    private job: BrowserJob,
    private dir: FileSystemDirectoryHandle,
    private signal: AbortSignal,
    private save: (force?: boolean) => Promise<void>,
    private log: (level: string, message: string) => void,
  ) {}
  private remaining() {
    const budget = captureOptions(this.job.config).max_download_mb;
    return budget === null
      ? Infinity
      : Math.max(0, budget * 1024 * 1024 - (this.job.downloaded_bytes || 0));
  }
  private consume(bytes: number) {
    if (bytes > this.remaining())
      throw new Error(
        "Download budget reached. Increase the budget in Edit crawl to continue.",
      );
    this.job.downloaded_bytes = (this.job.downloaded_bytes || 0) + bytes;
  }
  check() {
    if (this.signal.aborted) throw new Paused("Paused");
  }
  async wait(until: number) {
    while (Date.now() < until) {
      this.check();
      await new Promise((resolve) =>
        setTimeout(resolve, Math.min(200, until - Date.now())),
      );
    }
    this.check();
  }
  private async host(origin: string) {
    if (!this.hosts.has(origin))
      this.hosts.set(
        origin,
        (await this.store.get<Host>("hosts", origin)) || { next: 0 },
      );
    return this.hosts.get(origin)!;
  }
  private async request(
    url: string,
    headers: Record<string, string> = {},
    delay = this.job.config.delay_ms,
  ): Promise<Response> {
    const origin = new URL(url).origin,
      host = await this.host(origin);
    for (let attempt = 0; attempt < 4; attempt++) {
      this.check();
      if (this.remaining() === 0)
        throw new Error(
          "Download budget reached. Increase the budget in Edit crawl to continue.",
        );
      await this.wait(
        Math.max(
          host.next,
          (host.lastRequest || 0) + delay,
          (this.job.retry_at || 0) * 1000,
        ),
      );
      this.job.retry_at = null;
      host.lastRequest = Date.now();
      await this.store.put("hosts", origin, host);
      this.job.requests++;
      this.job.updated_at = now();
      await this.save(true);
      let response: Response;
      try {
        const controller = new AbortController();
        const abort = () => controller.abort();
        this.signal.addEventListener("abort", abort, { once: true });
        const timer = setTimeout(abort, 30000);
        try {
          response = await fetch(url, {
            headers,
            credentials: "omit",
            redirect: "manual",
            referrerPolicy: "no-referrer",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timer);
          // The response body continues to use the job signal below; on a
          // paused stream its reader is canceled before the OPFS handle closes.
          this.signal.removeEventListener("abort", abort);
        }
      } catch (error) {
        this.check();
        if (attempt === 3)
          throw new Error(
            `Cannot fetch ${new URL(url).hostname}. In an extension, grant website access. On a normal web page the source must allow CORS. Network detail: ${String(error)}`,
          );
        const wait = 2000 * 2 ** attempt;
        host.next = Date.now() + wait;
        this.job.retry_at = Math.ceil(host.next / 1000);
        this.log(
          "warn",
          `Network request interrupted; retrying in ${wait / 1000}s.`,
        );
        await this.store.put("hosts", origin, host);
        await this.save(true);
        continue;
      }
      // Browser redirect responses can be opaque even with host permission.
      // Never follow an unseen target before checking its scope/robots policy.
      if (
        response.type === "opaqueredirect" ||
        (response.status >= 300 && response.status < 400)
      ) {
        await response.body?.cancel();
        throw new Error(
          "This URL redirects. Open it in a browser tab and copy its final URL, then edit this crawl. The browser engine stops before following unverified redirects.",
        );
      }
      if ([429, 500, 502, 503, 504].includes(response.status)) {
        const retry = response.headers.get("Retry-After");
        const requested =
          retry && /^\d+$/.test(retry.trim())
            ? Number(retry) * 1000
            : retry
              ? Date.parse(retry) - Date.now()
              : 0;
        const delay =
          Math.max(
            Number.isFinite(requested) ? requested : 0,
            5000 * 2 ** attempt,
          ) + Math.floor(Math.random() * 500);
        host.next = Math.max(host.next, Date.now() + delay);
        this.job.retry_at = Math.ceil(host.next / 1000);
        this.log(
          "warn",
          `HTTP ${response.status}; respecting the site's cooldown before another request.`,
        );
        await response.body?.cancel();
        await this.store.put("hosts", origin, host);
        await this.save(true);
        if (attempt === 3)
          throw new Error(
            `HTTP ${response.status}: retry limit reached. Resume after the saved cooldown.`,
          );
        continue;
      }
      const target = new URL(response.url);
      if (
        target.origin !== origin ||
        (new URL(url).protocol === "https:" && target.protocol !== "https:")
      ) {
        await response.body?.cancel();
        throw new Error(
          "The source redirected to a different origin. Start a new crawl with its final URL so its permissions, scope, and robots.txt are checked.",
        );
      }
      return response;
    }
    throw new Error("Retry limit reached");
  }
  async policy(url: string, baseline = this.job.config.delay_ms) {
    const origin = new URL(url).origin,
      host = await this.host(origin);
    let fetched = false;
    if (
      host.robots === undefined ||
      Date.now() - (host.robots_at || 0) > 3600000
    ) {
      const response = await this.request(`${origin}/robots.txt`, {}, baseline);
      if (response.status === 404 || response.status === 410) {
        await response.body?.cancel();
        host.robots = "";
      } else {
        if (!response.ok) {
          await response.body?.cancel();
          throw new Error(
            `Cannot read robots.txt (HTTP ${response.status}); crawling stopped.`,
          );
        }
        host.robots = new TextDecoder().decode(
          await this.smallBody(response, 512 * 1024),
        );
      }
      host.robots_at = Date.now();
      fetched = true;
      await this.store.put("hosts", origin, host);
    }
    const policy = robotsParser(`${origin}/robots.txt`, host.robots);
    if (policy.isAllowed(url, "Zimsmith") === false)
      throw new RobotsDenied(`robots.txt disallows ${new URL(url).pathname}`);
    const declared = Math.max(0, policy.getCrawlDelay("Zimsmith") || 0) * 1000;
    if (fetched && declared > baseline) {
      host.next = Math.max(host.next, Date.now() + declared);
      await this.store.put("hosts", origin, host);
    }
    return Math.max(baseline, declared);
  }
  private async read(reader: ReadableStreamDefaultReader<Uint8Array>) {
    this.check();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let aborted: (() => void) | undefined;
    try {
      return await Promise.race([
        reader.read(),
        new Promise<never>((_, reject) => {
          aborted = () => {
            void reader.cancel();
            reject(new Paused("Paused"));
          };
          this.signal.addEventListener("abort", aborted, { once: true });
          timer = setTimeout(() => {
            void reader.cancel();
            reject(
              new Error(
                "No bytes received for 30 seconds. Resume to retry this file.",
              ),
            );
          }, 30000);
        }),
      ]);
    } finally {
      clearTimeout(timer);
      if (aborted) this.signal.removeEventListener("abort", aborted);
    }
  }
  private async smallBody(response: Response, limit: number) {
    const reader = response.body?.getReader();
    if (!reader) return new Uint8Array();
    const parts: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const { value, done } = await this.read(reader);
        if (done) break;
        size += value.length;
        this.consume(value.length);
        this.job.bytes += value.length;
        if (size > limit)
          throw new Error("Metadata exceeds the browser's safe size limit.");
        parts.push(value);
      }
    } finally {
      await reader.cancel();
    }
    const data = new Uint8Array(size);
    let at = 0;
    for (const p of parts) {
      data.set(p, at);
      at += p.length;
    }
    return data;
  }
  async download(
    url: string,
    limit = this.job.config.max_resource_mb * 1024 * 1024,
    asset = false,
  ): Promise<Receipt> {
    const receipt = this.job.resources[url];
    if (asset && !assetAllowed(this.job.config, url, receipt?.mime))
      throw new ExcludedAsset("Excluded by capture settings");
    if (receipt?.done && !receipt.revalidate) {
      try {
        if ((await readFile(this.dir, receipt.file)).size === receipt.size)
          return receipt;
      } catch {
        /* Re-fetch a missing or evicted file. */
      }
    }
    const delay = await this.policy(
      url,
      requestDelay(url, this.job.config.delay_ms, asset),
    ).catch((error) => {
      if (asset && error instanceof RobotsDenied)
        throw new ExcludedAsset(error.message);
      throw error;
    });
    const hash = new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(url)),
    );
    const fileName = `cache-${Array.from(hash, (b) => b.toString(16).padStart(2, "0")).join("")}`;
    const file = await syncFile(this.dir, fileName);
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    try {
      let start = receipt?.etag && !receipt.done ? file.getSize() : 0;
      const headers: Record<string, string> = start
        ? { Range: `bytes=${start}-`, "If-Range": receipt!.etag! }
        : {};
      const crossOriginRevalidation =
        /^https?:$/.test(location.protocol) &&
        new URL(url).origin !== location.origin;
      const canRevalidate =
        !crossOriginRevalidation &&
        receipt?.done &&
        receipt.revalidate &&
        file.getSize() === receipt.size &&
        receipt.size <= limit;
      if (canRevalidate) {
        if (receipt.etag) headers["If-None-Match"] = receipt.etag;
        else if (receipt.last_modified)
          headers["If-Modified-Since"] = receipt.last_modified;
      }
      const response = await this.request(url, headers, delay);
      if (response.status === 304 && canRevalidate) {
        receipt.revalidate = false;
        await response.body?.cancel();
        await this.save(true);
        return receipt;
      }
      if (!response.ok) {
        await response.body?.cancel();
        const message = `HTTP ${response.status} ${response.statusText}: ${new URL(url).pathname}`;
        if (asset && (response.status === 404 || response.status === 410))
          throw new ExcludedAsset(message);
        throw new Error(message);
      }
      if (
        asset &&
        !assetAllowed(
          this.job.config,
          url,
          response.headers.get("Content-Type") || "",
        )
      ) {
        await response.body?.cancel();
        throw new ExcludedAsset("Excluded by file-type settings");
      }
      const etag = response.headers.get("ETag") || undefined;
      const range = response.headers.get("Content-Range");
      if (response.status === 206) {
        const match = range?.match(/^bytes (\d+)-(\d+)\/(\d+)$/);
        if (
          !start ||
          !match ||
          Number(match[1]) !== start ||
          !etag ||
          etag !== receipt?.etag
        ) {
          await response.body?.cancel();
          file.truncate(0);
          delete this.job.resources[url];
          throw new Error(
            "The server returned an inconsistent range or changed file. Resume to restart this download safely.",
          );
        }
      } else {
        start = 0;
        file.truncate(0);
      }
      const length = response.headers.get("Content-Length"),
        encoding = response.headers.get("Content-Encoding");
      // Fetch transparently decodes gzip/br. Its Content-Length describes the
      // wire bytes, so it cannot validate the decoded body's file length.
      const crossOriginWeb =
        /^https?:$/.test(location.protocol) &&
        new URL(url).origin !== location.origin;
      const total = range
        ? Number(range.split("/")[1])
        : length && !crossOriginWeb && (!encoding || encoding === "identity")
          ? Number(length) + start
          : undefined;
      if ((total && total > limit) || start > limit) {
        await response.body?.cancel();
        throw new Error("This file exceeds the configured file-size limit.");
      }
      if (total !== undefined && total - start > this.remaining()) {
        await response.body?.cancel();
        throw new Error(
          "Download budget reached. Increase the budget in Edit crawl to continue.",
        );
      }
      const estimate = await navigator.storage.estimate();
      if (
        total &&
        estimate.quota &&
        total - start > estimate.quota - (estimate.usage || 0)
      ) {
        await response.body?.cancel();
        throw new Error(
          "There is not enough browser storage for this file. Free space or reduce the collection.",
        );
      }
      const saved: Receipt = {
        file: fileName,
        mime: (
          response.headers.get("Content-Type") || "application/octet-stream"
        )
          .split(";")[0]
          .toLowerCase(),
        size: start,
        etag:
          etag &&
          !etag.startsWith("W/") &&
          !crossOriginWeb &&
          (!encoding || encoding === "identity")
            ? etag
            : undefined,
        last_modified: response.headers.get("Last-Modified") || undefined,
        total,
        done: false,
      };
      this.job.resources[url] = saved;
      this.job.progress = {
        label: `Receiving ${new URL(url).pathname.split("/").pop() || new URL(url).hostname}`,
        completed: this.job.entries.filter((e) => e.state === "done").length,
        total: null,
        unit: "files",
        received: start,
        expected: total || null,
        updated_at: now(),
      };
      await this.save(true);
      reader = response.body?.getReader();
      if (!reader)
        throw new Error("The server returned an empty response body.");
      while (true) {
        const { value, done } = await this.read(reader);
        if (done) break;
        if (saved.size + value.length > limit)
          throw new Error(
            "The download exceeded the configured file-size limit.",
          );
        if (file.write(value, { at: saved.size }) !== value.length)
          throw new Error(
            "Browser storage returned a short write. Free space and resume.",
          );
        saved.size += value.length;
        this.consume(value.length);
        this.job.bytes += value.length;
        this.job.progress.received = saved.size;
        this.job.progress.updated_at = now();
        this.job.updated_at = now();
        await this.save();
      }
      if (total !== undefined && saved.size !== total)
        throw new Error(
          "The transfer ended before the advertised file length. Resume to retry.",
        );
      file.flush();
      saved.done = true;
      await this.save(true);
      return saved;
    } finally {
      if (reader) await reader.cancel();
      file.flush();
      file.close();
    }
  }
}
