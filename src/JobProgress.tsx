import { LoaderCircle, Pause, AlertCircle, Clock3 } from "lucide-react";
import { progressCount, useProgressTiming } from "./progressTiming";
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
  const progress = job.progress;
  const running = [
    "scanning",
    "downloading",
    "packaging",
    "pausing",
    "importing",
  ].includes(job.status);
  const scan = job.phase === "scan";
  const packaging = job.phase === "package";
  const selected = job.entries.filter((entry) => entry.selected);
  const done = selected.filter((entry) => entry.state === "done").length;
  const failedAssets = job.asset_summary?.failed || 0;
  const failed = job.entries.filter((entry) =>
    ["failed", "scan_failed"].includes(entry.state),
  ).length;
  const completed = scan ? job.entries.length : (progress?.completed ?? done);
  const total = scan
    ? null
    : (progress?.total ?? (job.phase === "download" ? selected.length : null));
  const unit = scan
    ? job.config.kind === "website"
      ? "items scanned"
      : "items found"
    : (progress?.unit ?? "items");
  const showTiming = running && (packaging || job.phase === "import");
  const { now, timing } = useProgressTiming(
    running
      ? `${job.id}:${job.phase}:${showTiming ? progress?.label || "" : ""}`
      : null,
    showTiming ? completed : 0,
    showTiming ? total : null,
  );
  if (["completed", "review"].includes(job.status)) return null;
  const waiting =
    running && !packaging && Boolean(job.retry_at && job.retry_at > now);
  const finishing = running && total != null && total > 0 && completed >= total;
  const percent =
    total && total > 0 && !finishing
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
  const count = scan
    ? `${completed} ${unit}${job.queued_pages ? ` · ${job.queued_pages} pages queued` : ""}`
    : progressCount(completed, total, unit);
  return (
    <section
      className={`job-progress${compact ? " compact" : ""}${running && !waiting ? " active" : ""}`}
      aria-label="Job progress"
      data-status={job.status}
    >
      <div className="progress-heading">
        <Icon size={18} className={running && !waiting ? "spin" : undefined} />
        <strong>
          {job.phase === "import"
            ? "Importing saved files"
            : scan
              ? "Scanning source"
              : job.phase === "package"
                ? "Building archive"
                : "Downloading collection"}
        </strong>
        <span>
          {count}
          {percent != null ? ` · ${percent}%` : ""}
        </span>
      </div>
      <div
        className={`progress-track${percent === null && running && !waiting ? " indeterminate" : ""}`}
        role="progressbar"
        aria-label="Job stage progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={finishing ? "Finishing this step" : count || undefined}
      >
        <div style={{ width: percent === null ? undefined : `${percent}%` }} />
      </div>
      <p className="progress-detail">
        {label}
        {finishing && " · Finishing this step…"}
      </p>
      {showTiming && !waiting && job.status !== "pausing" && (
        <p className="progress-detail">{timing}</p>
      )}
      {job.status === "downloading" &&
        progress?.active_asset_requests != null && (
          <p className="progress-detail">
            {progress.active_asset_requests} active asset{" "}
            {progress.active_asset_requests === 1 ? "request" : "requests"}
            {(progress.peak_asset_requests ?? 0) > 1 &&
              ` · Peak ${progress.peak_asset_requests}`}
          </p>
        )}
      {!compact &&
        job.status === "scanning" &&
        job.config.kind === "website" && (
          <p className="progress-detail">
            Each queued page is opened and captured before review.
          </p>
        )}
      {!compact && scan && progress?.unit === "render_items" && (
        <div className="transfer-progress">
          <span>Current page’s list items</span>
          <strong>
            {progress.completed}
            {progress.total ? ` of ${progress.total}` : " captured"}
          </strong>
          {progress.total && (
            <progress
              aria-label="Current page list capture"
              value={progress.completed}
              max={progress.total}
            />
          )}
        </div>
      )}
      {!compact &&
        Boolean(progress && (progress.received > 0 || progress.expected)) && (
          <div className="transfer-progress">
            <span>{packaging ? "Current file written" : "Current file"}</span>
            <strong>
              {size(progress!.received)}
              {progress!.expected
                ? ` of ${size(progress!.expected)}`
                : " received"}
            </strong>
            {progress!.expected && (
              <progress
                aria-label={
                  packaging ? "Current file written" : "Current file download"
                }
                value={progress!.received}
                max={progress!.expected}
              />
            )}
          </div>
        )}
      {!compact &&
        running &&
        !waiting &&
        !packaging &&
        job.phase !== "import" &&
        elapsed >= 45 && (
          <p className="alert warning">
            No new activity for {elapsed}s. The current step may be stalled.
            Pause and resume to retry from saved checkpoints.
          </p>
        )}
      {!compact && (
        <div className="progress-footer">
          <span>
            {job.requests} requests ·{" "}
            {failed
              ? `${failed} failed ${failed === 1 ? "item" : "items"}`
              : failedAssets
                ? `${failedAssets} failed assets`
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
