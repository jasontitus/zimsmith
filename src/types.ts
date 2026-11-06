export type SourceKind = "website" | "media";
export type ArchiveTemplate =
  "atlas" | "field_notes" | "midnight" | "screening_room" | "website";
export interface ArchiveTemplateOption {
  id: ArchiveTemplate;
  name: string;
  description: string;
}
export interface Settings {
  auto_start_small_jobs: boolean;
  auto_start_item_limit: number;
  archive_template: ArchiveTemplate;
  zstd_level: number;
  cdn_parallel_requests: number;
}
export interface CaptureOptions {
  auto_render_javascript: boolean;
  render_javascript: boolean;
  render_timeout_secs: number;
  embedded_videos: boolean;
  discover_sitemaps: boolean;
  include_urls: string[];
  seed_urls: string[];
  exclude_urls: string[];
  images: boolean;
  styles: boolean;
  fonts: boolean;
  media: boolean;
  documents: boolean;
  max_download_mb: number | null;
  video_quality: "small" | "balanced" | "high";
  subtitles: boolean;
  subtitle_language: string;
  auto_captions: boolean;
}
export interface Config {
  capture?: CaptureOptions;
  url: string;
  title: string;
  kind: SourceKind;
  max_items: number;
  max_depth: number;
  delay_ms: number;
  max_resource_mb: number;
  language: string;
  description: string;
  creator: string;
  include_assets: boolean;
}
export interface Entry {
  rendered?: {
    settled: boolean;
    resource_count: number;
    known_transfer_bytes: number;
    warnings: string[];
  } | null;
  embedded_in?: string[];
  id: string;
  url: string;
  title: string;
  depth: number;
  selected: boolean;
  state: string;
  size: number | null;
  duration: number | null;
  error: string | null;
}
export interface CaptureSummary {
  javascript_pages?: number;
  captured_items: number;
  saved_assets: number;
  omitted_assets: number;
  excluded_assets: number;
  failed_assets: number;
  online_links: number;
  subtitle_tracks: number;
  caption_gaps: number;
  search_limited: boolean;
  warnings: string[];
}
export interface Job {
  queued_pages?: number;
  javascript_needed?: boolean;
  capture_report?: CaptureSummary | null;
  validation?: {
    checksum_verified: boolean;
    structure_verified: boolean;
    checked_at: number;
    archive_entries: number;
  } | null;
  id: string;
  config: Config;
  status: string;
  phase: string;
  auto_start_item_limit: number | null;
  scan_truncated: boolean;
  archive_template: ArchiveTemplate;
  zstd_level?: number;
  archive_revision?: number;
  created_at: number;
  updated_at: number;
  entries: Entry[];
  logs: { time: number; level: string; message: string }[];
  error: string | null;
  output: string | null;
  output_bytes: number | null;
  retry_at: number | null;
  requests: number;
  bytes: number;
  downloaded_bytes?: number;
  asset_summary?: {
    downloaded: number;
    missing: number;
    omitted: number;
    failed?: number;
  };
  import?: {
    files?: { path: string; size: number }[];
    finished: boolean;
    file_count: number;
  } | null;
  refresh?: {
    compared: boolean;
    complete: boolean;
    changes: { url: string; title: string; kind: string }[];
  } | null;
  progress?: {
    label: string;
    completed: number;
    total: number | null;
    unit: string;
    received: number;
    expected: number | null;
    active_asset_requests?: number | null;
    peak_asset_requests?: number | null;
    updated_at: number;
  };
}
export interface Capabilities {
  version: string;
  platform: string;
  local_execution: boolean;
  yt_dlp: { available: boolean; detail: string };
  zimwriterfs: { available: boolean; detail: string };
  data_dir: string;
}
