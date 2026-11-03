export type SourceKind = "website" | "media";
export type ArchiveTemplate =
  "atlas" | "field_notes" | "midnight" | "screening_room";
export interface ArchiveTemplateOption {
  id: ArchiveTemplate;
  name: string;
  description: string;
}
export interface Settings {
  auto_start_small_jobs: boolean;
  auto_start_item_limit: number;
  archive_template: ArchiveTemplate;
}
export interface Config {
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
export interface Job {
  id: string;
  config: Config;
  status: string;
  phase: string;
  auto_start_item_limit: number | null;
  scan_truncated: boolean;
  archive_template: ArchiveTemplate;
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
  asset_summary?: { downloaded: number; missing: number; omitted: number };
  progress?: {
    label: string;
    completed: number;
    total: number | null;
    unit: string;
    received: number;
    expected: number | null;
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
