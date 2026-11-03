import { useEffect, useState } from "react";
import { LoaderCircle, Pause, AlertCircle, Clock3 } from "lucide-react";
import type { Job } from "./types";

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

export function JobProgress({
  job,
  compact = false,
}: {
  job: Job;
  compact?: boolean;
}) {
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => clearInterval(timer);
  }, []);
  if (["completed", "review"].includes(job.status)) return null;
  const progress = job.progress;
  const running = ["scanning", "downloading", "packaging", "pausing"].includes(
    job.status,
  );
  const waiting = Boolean(job.retry_at && job.retry_at > now);
  const scan = job.phase === "scan";
  const selected = job.entries.filter((entry) => entry.selected);
  const done = selected.filter((entry) => entry.state === "done").length;
  const failed = job.entries.filter((entry) =>
    ["failed", "scan_failed"].includes(entry.state),
  ).length;
  const completed = scan ? job.entries.length : (progress?.completed ?? done);
  const total = scan
    ? null
    : (progress?.total ?? (job.phase === "download" ? selected.length : null));
  const unit = scan ? "items found" : progress?.unit || "items";
  const percent =
    total && total > 0
      ? Math.min(100, Math.floor((completed / total) * 100))
      : null;
  const elapsed = Math.max(
    0,
    Math.floor(now - Math.max(progress?.updated_at || 0, job.updated_at || 0)),
  );
  const label = waiting
    ? `Waiting for server cooldown · ${Math.ceil(job.retry_at! - now)}s remaining`
    : job.status === "paused"
      ? "Paused — resume when ready"
      : job.status === "pausing"
        ? "Pausing safely…"
        : job.status === "failed"
          ? "Needs attention — check the error message"
          : progress?.label ||
            (scan
              ? "Discovering source items"
              : job.phase === "package"
                ? "Building the ZIM"
                : "Preparing the next download");
  const Icon = waiting
    ? Clock3
    : job.status === "failed"
      ? AlertCircle
      : job.status === "paused"
        ? Pause
        : LoaderCircle;
  return (
    <section
      className={`job-progress${compact ? " compact" : ""}${running && !waiting ? " active" : ""}`}
      aria-label="Job progress"
      data-status={job.status}
    >
      <div className="progress-heading">
        <Icon size={18} className={running && !waiting ? "spin" : undefined} />
        <strong>
          {scan
            ? "Scanning source"
            : job.phase === "package"
              ? "Building archive"
              : "Downloading collection"}
        </strong>
        <span>
          {percent === null
            ? `${completed} ${unit}`
            : `${completed} of ${total} ${unit} · ${percent}%`}
        </span>
      </div>
      <div
        className={`progress-track${percent === null && running && !waiting ? " indeterminate" : ""}`}
        role="progressbar"
        aria-label="Job stage progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={
          percent === null
            ? `${completed} ${unit}; total not known yet`
            : `${completed} of ${total} ${unit}`
        }
      >
        <div style={{ width: percent === null ? undefined : `${percent}%` }} />
      </div>
      <p className="progress-detail">{label}</p>
      {!compact &&
        Boolean(progress && (progress.received > 0 || progress.expected)) && (
          <div className="transfer-progress">
            <span>Current file</span>
            <strong>
              {size(progress!.received)}
              {progress!.expected
                ? ` of ${size(progress!.expected)}`
                : " received"}
            </strong>
            {progress!.expected && (
              <progress
                aria-label="Current file download"
                value={progress!.received}
                max={progress!.expected}
              />
            )}
          </div>
        )}
      {!compact && (
        <div className="progress-footer">
          <span>
            {job.requests} requests ·{" "}
            {failed
              ? `${failed} failed ${failed === 1 ? "item" : "items"}`
              : "No failed items"}
          </span>
          <span>
            {elapsed < 2
              ? "Just updated"
              : elapsed < 30
                ? `Last activity ${elapsed}s ago`
                : `No new activity for ${elapsed}s`}
          </span>
        </div>
      )}
    </section>
  );
}
