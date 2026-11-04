import type { Job } from "./types";
import { CheckCheck } from "lucide-react";
export function CaptureReport({ job }: { job: Job }) {
  const report = job.capture_report;
  if (!report || job.status !== "completed") return null;
  return (
    <section
      className="panel capture-summary"
      aria-labelledby="capture-summary-title"
    >
      <h2 id="capture-summary-title">Capture & validation report</h2>
      {job.zstd_level !== undefined && (
        <p>Archive compression: Zstandard level {job.zstd_level}.</p>
      )}
      <dl>
        {[
          ["Captured items", report.captured_items],
          ["Browser-rendered pages", report.javascript_pages || 0],
          ["Saved assets", report.saved_assets],
          ["Omitted assets", report.omitted_assets],
          ["Excluded by settings", report.excluded_assets],
          ["Online-only links", report.online_links],
          ["Subtitle tracks", report.subtitle_tracks],
          ["Caption gaps", report.caption_gaps],
        ].map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      {job.validation?.checksum_verified &&
        job.validation?.structure_verified && (
          <p className="validation-pass">
            <CheckCheck size={18} /> Checksum and directory structure verified ·{" "}
            {job.validation.archive_entries} ZIM entries
          </p>
        )}
      {report.warnings.length > 0 && (
        <details>
          <summary>{report.warnings.length} capture notes</summary>
          <ul>
            {report.warnings.map((warning, i) => (
              <li key={i}>{warning}</li>
            ))}
          </ul>
        </details>
      )}
      <p>
        Open the ZIM’s Capture report for asset results and links outside the
        archive. Successful validation does not guarantee every source page or
        video displays correctly.
      </p>
    </section>
  );
}
