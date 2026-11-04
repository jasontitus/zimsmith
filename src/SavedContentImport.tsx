import { useEffect, useRef, useState } from "react";
import { Archive, FolderOpen, Upload, X } from "lucide-react";
import { api, savedImports } from "./api";
import { normalizedCapture } from "./capture";
import { normalizeSourceUrl } from "./source";
import type { Config, Job } from "./types";

const size = (n: number) =>
  n < 1024 ** 2
    ? `${Math.ceil(n / 1024)} KB`
    : `${(n / 1024 ** 2).toFixed(1)} MB`;
function encoded(bytes: Uint8Array) {
  let text = "";
  for (let i = 0; i < bytes.length; i += 8192)
    text += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(text);
}
export function SavedContentImport({
  config,
  resumeId,
  onCreated,
  onBusyChange,
}: {
  config: Config;
  resumeId: string | null;
  onCreated: (job: Job) => void;
  onBusyChange: (busy: boolean) => void;
}) {
  const [mode, setMode] = useState<"files" | "urls">("files");
  const [files, setFiles] = useState<File[]>([]);
  const [urls, setUrls] = useState("");
  const [title, setTitle] = useState(resumeId ? config.title : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [received, setReceived] = useState(0);
  const active = useRef(false);
  const stop = useRef(false),
    attempt = useRef<string | null>(null);
  const filePicker = useRef<HTMLInputElement>(null),
    folderPicker = useRef<HTMLInputElement>(null);
  useEffect(
    () => () => {
      stop.current = true;
      if (active.current && attempt.current)
        void savedImports.cancel(attempt.current).catch(() => {});
      onBusyChange(false);
    },
    [],
  );
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => {
      if (attempt.current)
        void api
          .get(attempt.current)
          .then((job) => {
            if (job.progress?.label) setStatus(job.progress.label);
          })
          .catch(() => {});
    }, 1500);
    return () => clearInterval(timer);
  }, [busy]);
  const total = files.reduce((sum, file) => sum + file.size, 0);
  const path = (file: File) => file.webkitRelativePath || file.name;
  const choose = async (selected: File[]) => {
    setError("");
    setReceived(0);
    setStatus("");
    if (selected.length === 1 && /\.(txt|urls)$/i.test(selected[0].name)) {
      if (selected[0].size > 1024 * 1024) {
        setError("URL lists must be at most 1 MB.");
        return;
      }
      setUrls(await selected[0].text());
      setMode("urls");
      setTitle(title || "URL collection");
      return;
    }
    if (
      !resumeId &&
      JSON.stringify(files.map((file) => [path(file), file.size])) !==
        JSON.stringify(selected.map((file) => [path(file), file.size]))
    )
      attempt.current = null;
    setFiles(selected);
    setMode("files");
    if (!title && selected[0])
      setTitle(
        (
          selected[0].webkitRelativePath.split("/")[0] || selected[0].name
        ).replace(/\.(zip|html?)$/i, ""),
      );
  };
  const start = async () => {
    setError("");
    active.current = true;
    setBusy(true);
    onBusyChange(true);
    stop.current = false;
    try {
      if (!title.trim())
        throw new Error("Give the imported collection a title.");
      if (mode === "urls") {
        const sources = [
          ...new Set(
            urls
              .split(/\r?\n/)
              .map((line) => line.trim())
              .filter((line) => line && !line.startsWith("#"))
              .map(normalizeSourceUrl),
          ),
        ];
        if (!sources.length || sources.length > 1001)
          throw new Error("Paste 1–1001 starting URLs, one per line.");
        const job = await api.create({
          ...config,
          title: title.trim(),
          kind: "website",
          url: sources[0],
          max_items: Math.max(config.max_items, sources.length),
          capture: {
            ...normalizedCapture(config),
            seed_urls: sources.slice(1),
          },
        });
        onCreated(job);
        return;
      }
      if (!files.length)
        throw new Error("Choose saved files, a folder, or a ZIP first.");
      const job = await savedImports.begin(
        {
          ...config,
          title: title.trim(),
          url: "https://local-import.invalid/",
          kind: "website",
          capture: { ...normalizedCapture(config), seed_urls: [] },
        },
        files.map((file) => ({ path: path(file), size: file.size })),
        resumeId || attempt.current,
      );
      attempt.current = job.id;
      let completed = 0;
      for (const file of files) {
        if (stop.current)
          throw new Error(
            "Import stopped. Reselect these files to resume, or delete the failed attempt in Activity.",
          );
        let offset = await savedImports.offset(job.id, path(file));
        setReceived(completed + offset);
        if (file.size === 0)
          await savedImports.chunk(job.id, path(file), 0, "");
        while (offset < file.size) {
          if (stop.current)
            throw new Error(
              "Import stopped. Saved chunks are kept for resuming.",
            );
          const bytes = new Uint8Array(
            await file.slice(offset, offset + 256 * 1024).arrayBuffer(),
          );
          offset = await savedImports.chunk(
            job.id,
            path(file),
            offset,
            encoded(bytes),
          );
          setReceived(completed + offset);
          setStatus(`Importing ${path(file)}`);
        }
        completed += file.size;
      }
      if (stop.current)
        throw new Error("Import stopped. Reselect the saved files to resume.");
      setStatus("Preparing imported files. No internet requests.");
      const ready = await savedImports.finish(job.id);
      attempt.current = null;
      onCreated(ready);
    } catch (error) {
      if (attempt.current) {
        const current = await api.get(attempt.current).catch(() => null);
        if (
          current &&
          !["paused", "pausing", "failed", "review", "completed"].includes(
            current.status,
          )
        )
          await savedImports.cancel(attempt.current).catch(() => {});
      }
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      active.current = false;
      setBusy(false);
      onBusyChange(false);
    }
  };
  return (
    <details className="card import-panel" open={resumeId ? true : undefined}>
      <summary>
        <Upload size={18} /> Import saved content or a URL list
      </summary>
      <div className="import-content">
        <p>
          Package saved HTML with its assets, media, folders, or ZIPs. Local
          imports stay offline. A URL list starts a normal crawl.
        </p>
        {resumeId && (
          <p className="alert info">
            Reselect the same files and paths to continue this import. Uploaded
            chunks will be reused.
          </p>
        )}
        {!resumeId && (
          <div className="source-types" role="group" aria-label="Import source">
            <button
              type="button"
              className={`source-type ${mode === "files" ? "chosen" : ""}`}
              disabled={busy}
              onClick={() => setMode("files")}
            >
              Saved files
            </button>
            <button
              type="button"
              className={`source-type ${mode === "urls" ? "chosen" : ""}`}
              disabled={busy}
              onClick={() => setMode("urls")}
            >
              URL list
            </button>
          </div>
        )}
        <label htmlFor="import-title">Imported collection title</label>
        <input
          id="import-title"
          value={title}
          maxLength={120}
          disabled={busy}
          onChange={(e) => setTitle(e.target.value)}
        />
        {mode === "files" ? (
          <>
            <input
              ref={filePicker}
              type="file"
              multiple
              hidden
              aria-label="Choose saved files"
              onChange={(e) => void choose(Array.from(e.target.files || []))}
            />
            <input
              ref={(node) => {
                folderPicker.current = node;
                node?.setAttribute("webkitdirectory", "");
              }}
              type="file"
              multiple
              hidden
              aria-label="Choose saved folder"
              onChange={(e) => void choose(Array.from(e.target.files || []))}
            />
            <div className="import-actions">
              <button
                className="button secondary"
                type="button"
                disabled={busy}
                onClick={() => filePicker.current?.click()}
              >
                <Archive size={17} /> Choose files or ZIP
              </button>
              <button
                className="button secondary"
                type="button"
                disabled={busy}
                onClick={() => folderPicker.current?.click()}
              >
                <FolderOpen size={17} /> Choose folder
              </button>
            </div>
            <p className="settings-note">
              Choose a saved page with its asset folder, or ZIP them together.
              If your device can’t select folders, use a ZIP. ZIPs support
              stored/deflate files; symbolic links and encrypted archives are
              rejected.
            </p>
            {files.length > 0 && (
              <div className="import-preview" role="status">
                <strong>
                  {files.length} files · {size(total)}
                </strong>
                <ul>
                  {files.slice(0, 6).map((file) => (
                    <li key={path(file)}>{path(file)}</li>
                  ))}
                </ul>
                {files.length > 6 && <p>And {files.length - 6} more files.</p>}
                <p>
                  You’ll review the discovered pages and media before building.
                </p>
              </div>
            )}
          </>
        ) : (
          <>
            <label htmlFor="import-urls">Starting URLs, one per line</label>
            <textarea
              id="import-urls"
              rows={6}
              value={urls}
              disabled={busy}
              onChange={(e) => setUrls(e.target.value)}
              placeholder={
                "https://example.com/guide/\nhttps://another.example/articles/"
              }
            />
            <p className="settings-note">
              Each URL defines its own website scope. Normal pacing, robots
              rules, limits, and review settings apply.
            </p>
          </>
        )}
        {error && (
          <div className="alert error" role="alert">
            {error}
          </div>
        )}
        {busy && (
          <div className="import-progress" role="status">
            <p>{status || "Preparing import…"}</p>
            {mode === "files" && (
              <>
                <progress
                  aria-label="Saved file import progress"
                  max={Math.max(1, total)}
                  value={received}
                />
                <p>
                  {size(received)} of {size(total)}
                </p>
              </>
            )}
          </div>
        )}
        <div className="import-actions">
          <button
            className="button primary"
            type="button"
            disabled={
              busy ||
              !title.trim() ||
              (mode === "files" ? !files.length : !urls.trim())
            }
            onClick={() => void start()}
          >
            <Upload size={17} />{" "}
            {mode === "urls"
              ? "Scan URL list"
              : resumeId
                ? "Continue import"
                : "Import & review"}
          </button>
          {busy && (
            <button
              className="button secondary"
              type="button"
              onClick={() => {
                stop.current = true;
                if (attempt.current)
                  void savedImports.cancel(attempt.current).catch(() => {});
              }}
            >
              <X size={17} /> Stop import
            </button>
          )}
        </div>
      </div>
    </details>
  );
}
