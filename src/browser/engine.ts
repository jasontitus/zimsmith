import { version as appVersion } from "../../package.json";
import { normalizedCapture, pageAllowed, linkedAsset } from "../capture";
import type { ArchiveTemplate, Config, Job, Settings } from "../types";
import { normalizeSourceUrl, isNativeMediaUrl } from "../source";
import {
  defaults,
  inScope,
  now,
  resolveUrl,
  validate,
  type BrowserJob,
} from "./model";
import {
  bundledAssets,
  discover,
  preview,
  renderIndex,
  renderMedia,
  rewriteCss,
  rewritePage,
  templates,
} from "./content";
import { Network, Paused, ExcludedAsset, unavailableAsset } from "./network";
import {
  approvedFiles,
  diskName,
  prepareImport,
  uploadChunk,
  type ImportFile,
} from "./imports";
import { readFile, Store, syncFile } from "./storage";
import { captureReport } from "./report";
import { writeZim, verifyZim, type ZimItem } from "./zim";

export class BrowserEngine {
  private jobs = new Map<string, BrowserJob>();
  private queue: string[] = [];
  private running = false;
  private controls = new Map<string, AbortController>();
  private mutations = new Set<string>();
  private savedAt = new Map<string, number>();
  private constructor(private store: Store) {}
  static async open() {
    // One owner across tabs prevents competing manifests and duplicate crawls.
    await new Promise<void>((resolve, reject) => {
      if (!navigator.locks) {
        reject(new Error("This browser does not support Web Locks."));
        return;
      }
      void navigator.locks
        .request(
          "zimsmith-browser-workspace",
          { ifAvailable: true },
          async (lock) => {
            if (!lock) {
              reject(
                new Error(
                  "Zimsmith is already open in another tab. Use that workspace, or close it and reload this one.",
                ),
              );
              return;
            }
            resolve();
            await new Promise(() => {});
          },
        )
        .catch(reject);
    });
    const store = await Store.open(),
      engine = new BrowserEngine(store);
    for (const job of await store.allJobs()) {
      if (job.schema !== 1) continue;
      if (
        [
          "scanning",
          "downloading",
          "packaging",
          "pausing",
          "importing",
        ].includes(job.status)
      ) {
        job.status = "paused";
        engine.log(
          job,
          "warn",
          "The previous browser session ended. Resume to continue from saved files and checkpoints.",
        );
        await store.put("jobs", job.id, job);
      }
      engine.jobs.set(job.id, job);
    }
    await store.cleanupOrphans(new Set(engine.jobs.keys()));
    return engine;
  }
  private log(job: BrowserJob, level: string, message: string) {
    job.logs.push({ time: now(), level, message });
    if (job.logs.length > 500) job.logs.splice(0, job.logs.length - 500);
    job.updated_at = now();
  }
  private async save(job: BrowserJob, force = true) {
    if (force || Date.now() - (this.savedAt.get(job.id) || 0) > 1000) {
      await this.store.put("jobs", job.id, job);
      this.savedAt.set(job.id, Date.now());
    }
  }
  private job(id: string) {
    const job = this.jobs.get(id);
    if (!job)
      throw new Error("Archive job was not found in this browser profile.");
    return job;
  }
  private idle(job: BrowserJob) {
    if (
      this.controls.has(job.id) ||
      ["scanning", "downloading", "packaging", "pausing", "importing"].includes(
        job.status,
      )
    )
      throw new Error("Pause this job before changing it.");
  }
  private view(job: BrowserJob): Job {
    const {
      import: _import,
      refresh: _refresh,
      resources: _,
      queue: _q,
      visited: _v,
      output_file: _o,
      schema: _s,
      ...view
    } = job;
    return {
      ...view,
      import: job.import
        ? {
            finished: job.import.finished,
            file_count: Math.max(
              job.import.file_count,
              job.import.files?.length || 0,
            ),
          }
        : null,
      refresh: job.refresh
        ? {
            compared: job.refresh.compared,
            complete: job.refresh.complete,
            changes: job.refresh.changes,
          }
        : null,
      entries: job.entries.map(
        ({ text: _t, file: _f, media: _m, mime: _type, ...entry }) => entry,
      ),
    };
  }
  async call(
    command: string,
    args: Record<string, unknown> = {},
  ): Promise<unknown> {
    // Pause/cancel must remain available while a long import is finalizing.
    const exclusive = [
      "resume_job",
      "edit_job",
      "build_job",
      "delete_failed_job",
      "begin_import",
      "import_chunk",
      "finish_import",
      "refresh_job",
      "rebuild_job",
    ].includes(command);
    const id = String(args.id || args.existingId || "");
    if (!exclusive || !id) return this.dispatch(command, args);
    if (this.mutations.has(id))
      throw new Error("A job change is still in progress. Try again shortly.");
    this.mutations.add(id);
    try {
      return await this.dispatch(command, args);
    } finally {
      this.mutations.delete(id);
    }
  }
  private async dispatch(
    command: string,
    args: Record<string, unknown>,
  ): Promise<unknown> {
    const id = String(args.id || "");
    switch (command) {
      case "capabilities":
        return {
          version: appVersion,
          platform: "browser",
          local_execution: true,
          yt_dlp: {
            available: false,
            detail:
              "Direct media files only; collection extraction is not yet ported.",
          },
          zimwriterfs: {
            available: true,
            detail: "Streaming browser ZIM 6.3 writer",
          },
          data_dir: "This browser profile · local files and checkpoints",
        };
      case "archive_templates":
        return templates;
      case "preview_archive_template":
        return preview(args.template as ArchiveTemplate);
      case "get_settings":
        return {
          ...defaults,
          ...(await this.store.get<Settings>("preferences", "settings")),
        };
      case "update_settings": {
        const settings = { ...defaults, ...(args.settings as Settings) };
        if (
          typeof settings.auto_start_small_jobs !== "boolean" ||
          !Number.isInteger(settings.auto_start_item_limit) ||
          settings.auto_start_item_limit < 1 ||
          settings.auto_start_item_limit > 5000 ||
          !Number.isInteger(settings.zstd_level) ||
          settings.zstd_level < 1 ||
          settings.zstd_level > 22 ||
          !Number.isInteger(settings.cdn_parallel_requests) ||
          settings.cdn_parallel_requests < 1 ||
          settings.cdn_parallel_requests > 6 ||
          !templates.some((t) => t.id === settings.archive_template)
        )
          throw new Error("Invalid archive preferences.");
        await this.store.put("preferences", "settings", settings);
        return settings;
      }
      case "list_jobs":
        return [...this.jobs.values()]
          .sort((a, b) => b.created_at - a.created_at)
          .map((job) => this.view(job));
      case "get_job":
        return this.view(this.job(id));
      case "create_job":
        return this.create(args.config as Config);
      case "pause_job": {
        const job = this.job(id);
        if (
          ![
            "scanning",
            "downloading",
            "packaging",
            "pausing",
            "importing",
          ].includes(job.status)
        )
          return this.view(job);
        job.status =
          job.phase === "import"
            ? "paused"
            : this.controls.has(id)
              ? "pausing"
              : "paused";
        this.controls.get(id)?.abort();
        await this.save(job);
        return this.view(job);
      }
      case "resume_job": {
        const job = this.job(id);
        this.idle(job);
        if (job.import && !job.import.finished)
          throw new Error(
            "Reselect the same files in Import to continue this upload.",
          );
        if (["completed", "review"].includes(job.status)) return this.view(job);
        job.error = null;
        job.status =
          job.phase === "scan"
            ? "scanning"
            : job.phase === "download"
              ? "downloading"
              : "packaging";
        await this.save(job);
        this.launch(id);
        return this.view(job);
      }
      case "edit_job": {
        const job = this.job(id);
        this.idle(job);
        if (job.import)
          throw new Error(
            "Rebuild imported collections, or start a new import to add files.",
          );
        if (!["failed", "paused"].includes(job.status))
          throw new Error("Only failed or paused crawls can be edited.");
        const config = this.config(args.config as Config);
        const reset =
          JSON.stringify(config.capture?.seed_urls) !==
            JSON.stringify(job.config.capture?.seed_urls) ||
          JSON.stringify(config.capture?.include_urls) !==
            JSON.stringify(job.config.capture?.include_urls) ||
          JSON.stringify(config.capture?.exclude_urls) !==
            JSON.stringify(job.config.capture?.exclude_urls) ||
          (
            ["url", "kind", "max_items", "max_depth", "include_assets"] as const
          ).some((key) => config[key] !== job.config[key]);
        job.config = config;
        if (reset) {
          job.entries = [];
          job.queue = [config.url, ...(config.capture?.seed_urls || [])].map(
            (url) => ({ url, depth: 0 }),
          );
          job.visited = [];
          job.phase = "scan";
          job.scan_truncated = false;
        }
        job.error = null;
        job.status =
          job.phase === "scan"
            ? "scanning"
            : job.phase === "download"
              ? "downloading"
              : "packaging";
        this.log(
          job,
          "info",
          reset
            ? "Crawl updated; restarting discovery. Existing cache files remain available for identical URLs."
            : "Crawl settings updated. Continuing from the saved checkpoint.",
        );
        await this.save(job);
        this.launch(id);
        return this.view(job);
      }
      case "build_job": {
        const job = this.job(id);
        this.idle(job);
        if (!["review", "failed"].includes(job.status))
          throw new Error("Finish scanning before confirming the download.");
        const selected = new Set(args.selected as string[]);
        if (
          !selected.size ||
          [...selected].some((id) => !job.entries.some((e) => e.id === id))
        )
          throw new Error("Select at least one discovered item.");
        for (const entry of job.entries) {
          if (
            selected.has(entry.id) &&
            job.config.kind === "website" &&
            !entry.file
          )
            throw new Error(
              "A selected page has not been captured. Resume discovery or deselect it.",
            );
        }
        for (const entry of job.entries)
          entry.selected = selected.has(entry.id);
        job.phase = "download";
        job.status = "downloading";
        job.error = null;
        this.log(
          job,
          "info",
          `Approved ${selected.size} items for download and ZIM building.`,
        );
        await this.save(job);
        this.launch(id);
        return this.view(job);
      }
      case "delete_failed_job": {
        const job = this.job(id);
        if (
          job.status !== "failed" ||
          this.controls.has(id) ||
          this.queue.includes(id)
        )
          throw new Error("Only stopped failed attempts can be deleted.");
        const cleaned = await this.store.removeJob(id);
        this.jobs.delete(id);
        return cleaned;
      }
      case "begin_import": {
        const config = this.config({
          ...(args.config as Config),
          url: "https://local-import.invalid/",
          kind: "website",
        });
        const files = approvedFiles(args.files as ImportFile[], config);
        if (args.existingId) {
          const job = this.job(String(args.existingId));
          this.idle(job);
          if (
            !job.import ||
            job.import.finished ||
            JSON.stringify(job.import.files) !== JSON.stringify(files)
          )
            throw new Error(
              "Reselect the same files and folder paths to continue.",
            );
          job.config = config;
          job.status = "importing";
          job.error = null;
          await this.save(job);
          return this.view(job);
        }
        const created = await this.create(config, false),
          job = this.job(created.id);
        job.import = { files, finished: false, file_count: files.length };
        await this.save(job);
        return this.view(job);
      }
      case "import_offset": {
        const job = this.job(id),
          path = String(args.path);
        if (
          !job.import?.files?.some((file) => file.path === path) ||
          job.import.finished
        )
          throw new Error("Unapproved import path.");
        try {
          return (
            await readFile(await this.store.dir(id), await diskName(path))
          ).size;
        } catch {
          return 0;
        }
      }
      case "import_chunk": {
        if (this.controls.has(id))
          throw new Error("Import files are already being prepared.");
        const job = this.job(id),
          data = String(args.data);
        if (data.length > 349528)
          throw new Error("Import chunk exceeds 256 KB.");
        const size = await uploadChunk(
          job,
          this.store,
          String(args.path),
          Number(args.offset),
          Uint8Array.from(atob(data), (c) => c.charCodeAt(0)),
        );
        job.progress = {
          label: `Importing ${args.path}`,
          completed: 0,
          total: null,
          unit: "files",
          received: size,
          expected:
            job.import?.files?.find((file) => file.path === args.path)?.size ||
            null,
          updated_at: now(),
        };
        await this.save(job);
        return size;
      }
      case "cancel_import": {
        const job = this.job(id);
        if (!job.import || job.import.finished)
          throw new Error("Import is already finished.");
        this.controls.get(id)?.abort();
        job.status = "failed";
        job.error =
          "Import interrupted. Reselect the same files to continue, or delete this attempt.";
        await this.save(job);
        return;
      }
      case "finish_import": {
        const job = this.job(id);
        if (job.status !== "importing" || this.controls.has(id))
          throw new Error("Import is not ready to finalize.");
        const control = new AbortController();
        this.controls.set(id, control);
        try {
          await prepareImport(job, this.store, control.signal, () =>
            this.save(job),
          );
          return this.view(job);
        } catch (error) {
          if (this.job(id).status !== "paused") job.status = "failed";
          job.error = String(error);
          this.log(job, "error", job.error);
          await this.save(job);
          throw error;
        } finally {
          this.controls.delete(id);
        }
      }
      case "refresh_job": {
        const job = this.job(id);
        this.idle(job);
        if (
          job.status !== "completed" ||
          job.config.kind !== "website" ||
          job.import
        )
          throw new Error(
            "Browser refresh is available for completed website collections.",
          );
        const dir = await this.store.dir(id);
        const fingerprints: Record<string, string> = {};
        for (const entry of job.entries) {
          if (entry.file) {
            const file = await readFile(dir, entry.file);
            fingerprints[entry.url] = await this.fingerprint(file);
          }
        }
        job.refresh = {
          previous: structuredClone(job.entries),
          fingerprints,
          compared: false,
          complete: false,
          changes: [],
        };
        // Revalidate saved assets while discovery fetches fresh HTML.
        for (const resource of Object.values(job.resources))
          resource.revalidate = true;
        for (const entry of job.entries) delete job.resources[entry.url];
        job.omissions = {};
        job.entries = [];
        job.visited = [];
        job.queue = [
          job.config.url,
          ...(job.config.capture?.seed_urls || []),
        ].map((url) => ({ url, depth: 0 }));
        job.archive_revision = (job.archive_revision || 0) + 1;
        job.scan_truncated = false;
        job.downloaded_bytes = 0;
        job.error = null;
        job.phase = "scan";
        job.status = "scanning";
        this.log(
          job,
          "info",
          "Refreshing source pages. Review the changes before building; saved assets and the previous ZIM are retained.",
        );
        await this.save(job);
        this.launch(id);
        return this.view(job);
      }
      case "rebuild_job": {
        const job = this.job(id);
        this.idle(job);
        const template = args.template as ArchiveTemplate;
        if (
          job.phase !== "package" ||
          !["completed", "failed", "paused"].includes(job.status) ||
          !templates.some((t) => t.id === template) ||
          !job.entries.some((e) => e.selected) ||
          job.entries.some((e) => e.selected && e.state !== "done")
        )
          throw new Error(
            "Rebuild requires a valid template and all selected files to be downloaded.",
          );
        job.archive_template = template;
        job.archive_revision = (job.archive_revision || 0) + 1;
        job.phase = "package";
        job.status = "packaging";
        job.error = null;
        this.log(
          job,
          "info",
          "Rebuilding from saved files with the chosen template.",
        );
        await this.save(job);
        this.launch(id);
        return this.view(job);
      }
      case "export_archive": {
        const job = this.job(id);
        if (job.status !== "completed" || !job.output_file)
          throw new Error("This archive is not ready to download.");
        const file = await readFile(await this.store.dir(id), job.output_file);
        if (file.size !== job.output_bytes)
          throw new Error(
            "The saved archive is missing or incomplete. Rebuild it from the downloaded files.",
          );
        return {
          file,
          name: `${job.config.title.replace(/[^\p{L}\p{N} _-]/gu, "").slice(0, 80) || "Archive"}-${id.slice(0, 8)}${job.archive_revision ? `-r${job.archive_revision}` : ""}.zim`,
        };
      }
      default:
        throw new Error(`Unsupported browser command: ${command}`);
    }
  }
  private async fingerprint(file: File) {
    const digest = new Uint8Array(
      await crypto.subtle.digest("SHA-256", await file.arrayBuffer()),
    );
    return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
  }
  private async compareRefresh(
    job: BrowserJob,
    dir: FileSystemDirectoryHandle,
  ) {
    const refresh = job.refresh;
    if (!refresh || refresh.compared) return;
    const previous = new Map(
      refresh.previous.map((entry) => [entry.url, entry]),
    );
    for (const entry of job.entries) {
      const old = previous.get(entry.url);
      let kind = "new";
      if (old) {
        entry.selected = old.selected;
        kind =
          entry.file &&
          refresh.fingerprints[entry.url] ===
            (await this.fingerprint(await readFile(dir, entry.file)))
            ? "unchanged"
            : "changed";
      }
      refresh.changes.push({ url: entry.url, title: entry.title, kind });
      previous.delete(entry.url);
    }
    for (const entry of previous.values())
      refresh.changes.push({
        url: entry.url,
        title: entry.title,
        kind: "not_seen",
      });
    refresh.compared = true;
    refresh.complete = !job.scan_truncated;
    refresh.previous = [];
    refresh.fingerprints = {};
  }
  private config(config: Config) {
    const normalized = {
      ...config,
      capture: {
        ...normalizedCapture(config),
        seed_urls: normalizedCapture(config).seed_urls.map(normalizeSourceUrl),
      },
      url: normalizeSourceUrl(config.url),
      kind: isNativeMediaUrl(config.url) ? ("media" as const) : config.kind,
    };
    validate(normalized);
    return normalized;
  }
  private async create(input: Config, start = true) {
    const config = this.config(input),
      settings = (await this.call("get_settings")) as Settings;
    const job: BrowserJob = {
      schema: 1,
      id: crypto.randomUUID(),
      config,
      status: start ? "scanning" : "importing",
      phase: start ? "scan" : "import",
      archive_template: settings.archive_template,
      archive_revision: 0,
      auto_start_item_limit: settings.auto_start_small_jobs
        ? settings.auto_start_item_limit
        : null,
      scan_truncated: false,
      created_at: now(),
      updated_at: now(),
      entries: [],
      logs: [],
      error: null,
      output: null,
      output_bytes: null,
      retry_at: null,
      requests: 0,
      bytes: 0,
      queue: [config.url, ...(config.capture?.seed_urls || [])]
        .filter((url, index, all) => all.indexOf(url) === index)
        .map((url) => ({ url, depth: 0 })),
      visited: [],
      resources: {},
    };
    this.log(
      job,
      "info",
      "Browser-only engine started. Files and checkpoints stay in this browser profile. Keep this workspace tab open while the job runs.",
    );
    this.jobs.set(job.id, job);
    await this.save(job);
    if (start) this.launch(job.id);
    return this.view(job);
  }
  private launch(id: string) {
    if (!this.queue.includes(id) && !this.controls.has(id)) this.queue.push(id);
    if (!this.running) void this.drain();
  }
  private async drain() {
    this.running = true;
    try {
      while (this.queue.length) {
        const job = this.job(this.queue.shift()!);
        if (job.status === "paused" || job.status === "pausing") continue;
        const control = new AbortController();
        this.controls.set(job.id, control);
        try {
          await this.drive(job, control.signal);
        } catch (error) {
          if (error instanceof Paused || control.signal.aborted) {
            job.status = "paused";
            this.log(
              job,
              "info",
              "Paused. Saved files and recovery checkpoints are retained.",
            );
          } else {
            job.status = "failed";
            job.error = error instanceof Error ? error.message : String(error);
            this.log(job, "error", job.error);
          }
          try {
            await this.save(job);
          } catch {
            /* Keep the original quota/storage error visible in memory. */
          }
        } finally {
          this.controls.delete(job.id);
        }
      }
    } finally {
      this.running = false;
    }
  }
  private async drive(job: BrowserJob, signal: AbortSignal) {
    const dir = await this.store.dir(job.id),
      net = new Network(
        this.store,
        job,
        dir,
        signal,
        (force) => this.save(job, force ?? false),
        (level, message) => this.log(job, level, message),
      );
    if (job.phase === "scan") {
      this.log(
        job,
        "info",
        "Scanning pages and metadata. Media and linked assets wait until approval.",
      );
      if (job.config.kind === "media") {
        if (!job.entries.length)
          job.entries.push({
            id: crypto.randomUUID(),
            url: job.config.url,
            title: job.config.title,
            depth: 0,
            selected: true,
            state: "pending",
            size: null,
            duration: null,
            error: null,
            text: "Direct media saved for offline playback.",
          });
        job.queue = [];
      } else {
        const visited = new Set(job.visited);
        while (job.queue.length && job.entries.length < job.config.max_items) {
          net.check();
          const next = job.queue[0];
          if (visited.has(next.url)) {
            job.queue.shift();
            continue;
          }
          job.progress = {
            label: `Scanning ${new URL(next.url).pathname}`,
            completed: job.entries.length,
            total: null,
            unit: "pages",
            received: 0,
            expected: null,
            updated_at: now(),
          };
          let receipt;
          try {
            receipt = await net.download(next.url, 8 * 1024 * 1024);
          } catch (error) {
            if (
              error instanceof Error &&
              error.message.startsWith("robots.txt disallows") &&
              next.depth > 0
            ) {
              this.log(
                job,
                "warn",
                `Skipping ${new URL(next.url).pathname}: robots.txt disallows this page.`,
              );
              visited.add(next.url);
              job.visited.push(next.url);
              job.queue.shift();
              await this.save(job);
              continue;
            }
            throw error;
          }
          if (!["text/html", "application/xhtml+xml"].includes(receipt.mime))
            throw new Error(
              "The website source did not return HTML. Use a direct media URL with Video collection for video/audio files.",
            );
          const page = discover(
            await (await readFile(dir, receipt.file)).text(),
            next.url,
          );
          job.entries.push({
            id: crypto.randomUUID(),
            url: next.url,
            title: page.title,
            text: page.text,
            file: receipt.file,
            depth: next.depth,
            selected: pageAllowed(job.config, next.url),
            state: "pending",
            size: receipt.size,
            duration: null,
            error: null,
          });
          visited.add(next.url);
          job.visited.push(next.url);
          job.queue.shift();
          if (next.depth < job.config.max_depth)
            for (const url of page.links) {
              if (job.queue.length >= 20000) {
                job.scan_truncated = true;
                break;
              }
              if (
                [job.config.url, ...(job.config.capture?.seed_urls || [])].some(
                  (root) => inScope(url, root),
                ) &&
                pageAllowed(job.config, url) &&
                !linkedAsset(url) &&
                !visited.has(url) &&
                !job.queue.some((q) => q.url === url) &&
                !/\.(pdf|zip|png|jpe?g|gif|webp|svg|mp4|mp3|webm|m4a|css|js)(?:$|\?)/i.test(
                  url,
                )
              )
                job.queue.push({ url, depth: next.depth + 1 });
            }
          await this.save(job);
        }
        job.scan_truncated ||= job.queue.some((q) => !visited.has(q.url));
      }
      if (!job.entries.length)
        throw new Error("No accessible pages or media files were found.");
      this.log(
        job,
        "info",
        `Found ${job.entries.length} items${job.scan_truncated ? "; the scan limit was reached, so review is required" : ""}.`,
      );
      await this.compareRefresh(job, dir);
      if (
        !job.refresh &&
        job.entries.every(
          (entry) =>
            entry.selected && entry.state === "pending" && !entry.error,
        ) &&
        job.auto_start_item_limit !== null &&
        job.entries.length <= job.auto_start_item_limit &&
        !job.scan_truncated
      ) {
        job.phase = "download";
        job.status = "downloading";
        this.log(
          job,
          "info",
          "This complete scan is within your automatic-download threshold. Proceeding with the saved approval setting.",
        );
      } else {
        job.status = "review";
        await this.save(job);
        return;
      }
      await this.save(job);
    }
    if (job.phase === "download") {
      for (const entry of job.entries.filter((e) => e.selected)) {
        net.check();
        if (entry.state === "done") continue;
        try {
          if (job.config.kind === "media") {
            const receipt = await net.download(entry.url);
            if (receipt.mime === "text/html")
              throw new Error(
                "The media URL returned a web page instead of a playable file.",
              );
            const ext = new URL(entry.url).pathname
              .split(".")
              .pop()!
              .toLowerCase();
            entry.media = `media-${entry.id}.${ext}`;
            entry.file = receipt.file;
            entry.size = receipt.size;
            entry.mime =
              receipt.mime === "application/octet-stream"
                ? (
                    {
                      mp4: "video/mp4",
                      mov: "video/quicktime",
                      webm: "video/webm",
                      mp3: "audio/mpeg",
                      m4a: "audio/mp4",
                      ogg: "audio/ogg",
                      wav: "audio/wav",
                    } as Record<string, string>
                  )[ext]
                : receipt.mime;
          }
          entry.state = "done";
          entry.error = null;
        } catch (error) {
          if (!(error instanceof Paused)) {
            entry.state = "failed";
            entry.error = String(error);
          }
          throw error;
        }
        await this.save(job);
      }
      job.phase = "package";
      job.status = "packaging";
      await this.save(job);
    }
    if (job.phase !== "package") return;
    const previousOmissions = job.omissions;
    job.omissions = {};
    const items = new Map<string, ZimItem>();
    const add = (
      path: string,
      mime: string,
      content: Blob | string,
      title = path,
      namespace = "C",
    ) =>
      items.set(`${namespace}/${path}`, {
        path,
        title,
        namespace,
        mime,
        content: typeof content === "string" ? new Blob([content]) : content,
      });
    const paths = new Map<string, string>();
    const asset = async (url: string, depth = 0): Promise<string> => {
      net.check();
      if (paths.has(url)) return paths.get(url)!;
      if (paths.size >= 10000 || depth > 5)
        throw new Error(
          "The linked-asset limit was reached. Disable linked assets or use a smaller crawl.",
        );
      let receipt;
      try {
        const previous = previousOmissions?.[url];
        if (previous && unavailableAsset(previous))
          throw new ExcludedAsset(previous);
        if (job.import && !job.resources[url]?.done)
          throw new ExcludedAsset("Not available in the saved-file import");
        receipt = await net.download(url, undefined, true);
      } catch (error) {
        if (!(error instanceof ExcludedAsset)) throw error;
        (job.omissions ||= {})[url] = error.message;
        paths.set(url, "");
        this.log(
          job,
          "info",
          `Omitted asset ${new URL(url).pathname}: ${error.message}`,
        );
        return "";
      }
      if (receipt.mime === "text/html")
        throw new Error(
          `A linked asset returned HTML: ${new URL(url).pathname}`,
        );
      const ext =
        (
          {
            "text/css": ".css",
            "image/png": ".png",
            "image/jpeg": ".jpg",
            "image/svg+xml": ".svg",
            "image/webp": ".webp",
            "font/woff2": ".woff2",
          } as Record<string, string>
        )[receipt.mime] || "";
      const path = `asset-${receipt.file.slice(6)}${ext}`;
      paths.set(url, path);
      const file = await readFile(dir, receipt.file);
      if (receipt.mime === "text/css") {
        if (file.size > 2 * 1024 * 1024)
          throw new Error(
            "A stylesheet exceeds the browser's safe parsing limit.",
          );
        add(
          path,
          "text/css",
          await rewriteCss(
            await file.text(),
            url,
            (u) => asset(u, depth + 1),
            "stylesheet",
            (u) => !!job.resources?.[u]?.done,
          ),
        );
      } else add(path, receipt.mime, file);
      return path;
    };
    this.log(
      job,
      "info",
      "Preparing offline pages, linked assets, contents, and search. Downloaded files are reused.",
    );
    const selected = job.entries.filter((entry) => entry.selected);
    const updateProgress = async (
      label: string,
      completed: number,
      total: number | null,
      unit: string,
    ) => {
      job.progress = {
        label,
        completed,
        total,
        unit,
        received: 0,
        expected: null,
        updated_at: now(),
      };
      job.updated_at = now();
      await this.save(job, false);
    };
    for (const [index, entry] of selected.entries()) {
      net.check();
      await updateProgress(
        "Preparing offline pages and assets",
        index,
        selected.length,
        "items",
      );
      if (!entry.file)
        throw new Error(
          "A selected item is missing its saved file. Resume downloading first.",
        );
      if (entry.media) {
        add(
          entry.media,
          entry.mime || "application/octet-stream",
          await readFile(dir, entry.file),
        );
        add(
          `${entry.id}.html`,
          "text/html",
          renderMedia(job, entry),
          entry.title,
        );
      } else
        add(
          `${entry.id}.html`,
          "text/html",
          await rewritePage(
            await (await readFile(dir, entry.file)).text(),
            entry,
            job,
            asset,
          ),
          entry.title,
        );
    }
    await updateProgress(
      "Creating the capture report and contents",
      0,
      null,
      "",
    );
    const report = await captureReport(job, dir, paths);
    job.capture_report = report.summary;
    add("capture-report.html", "text/html", report.html, "Capture report");
    add(
      "capture-report.json",
      "application/json",
      report.json,
      "Capture report data",
    );
    add("index.html", "text/html", renderIndex(job), job.config.title);
    for (const item of await bundledAssets())
      add(item.path, item.mime, item.content);
    const counts: Record<string, number> = {};
    for (const item of items.values())
      counts[item.mime] = (counts[item.mime] || 0) + 1;
    for (const [name, value] of Object.entries({
      Title: job.config.title,
      Description: job.config.description,
      Creator: job.config.creator,
      Publisher: "Zimsmith",
      Language: job.config.language,
      Date: new Date().toISOString().slice(0, 10),
      Source: job.import ? "Local saved content" : job.config.url,
      Scraper: "Zimsmith browser 0.1",
      Counter: Object.entries(counts)
        .map(([mime, n]) => `${mime}=${n}`)
        .join(";"),
    }))
      add(name, "text/plain", value, name, "M");
    add(
      "Illustration_48x48@1",
      "image/png",
      items.get("C/favicon.png")!.content,
      "Illustration_48x48@1",
      "M",
    );
    const estimate = await navigator.storage.estimate(),
      bytes = [...items.values()].reduce(
        (sum, item) => sum + item.content.size,
        0,
      );
    if (
      estimate.quota &&
      bytes + 1024 * 1024 > estimate.quota - (estimate.usage || 0)
    )
      throw new Error(
        "There is not enough browser storage to write the ZIM alongside the cached files. Free space and Resume.",
      );
    const output = `archive-r${job.archive_revision || 0}.zim`,
      file = await syncFile(dir, output);
    const rootPage =
      job.archive_template === "website" && job.config.kind === "website"
        ? job.entries.find(
            (e) => e.url === job.config.url && e.selected && !e.media,
          )
        : undefined;
    const mainPath = rootPage ? `${rootPage.id}.html` : "index.html";
    try {
      job.output_bytes = await writeZim(
        file,
        [...items.values()],
        () => net.check(),
        (done, total, step) =>
          step === "checksum"
            ? updateProgress(
                "Calculating the ZIM checksum",
                done,
                total,
                "bytes",
              )
            : updateProgress(
                "Writing ZIM in browser storage",
                done,
                total,
                "files",
              ),
        mainPath,
      );
      job.validation = await verifyZim(
        file,
        () => net.check(),
        (done, total, step) =>
          step === "checksum"
            ? updateProgress(
                "Verifying finished ZIM checksum",
                done,
                total,
                "bytes",
              )
            : updateProgress(
                "Checking the ZIM directory",
                done,
                total,
                "entries",
              ),
        mainPath,
      );
    } finally {
      file.close();
    }
    net.check();
    job.output_file = output;
    job.output = output;
    job.status = "completed";
    job.error = null;
    job.retry_at = null;
    this.log(
      job,
      "info",
      "ZIM complete. Save ZIM file downloads it to your browser's chosen folder. No data was sent to a build server.",
    );
    await this.save(job);
  }
}
