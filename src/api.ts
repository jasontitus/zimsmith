import { invoke, isTauri } from "@tauri-apps/api/core";
import { browserApi, browserOnly, browserCommand } from "./browser/client";
import type {
  ArchiveTemplate,
  ArchiveTemplateOption,
  Capabilities,
  Config,
  Job,
  Settings,
} from "./types";

export const native = isTauri();
export async function refreshJavascript(id: string): Promise<Job> {
  if (!native)
    throw new Error(
      "JavaScript rendering requires the installed Zimsmith app.",
    );
  return invoke<Job>("refresh_javascript", { id });
}
// This HTTP bridge is only for developing the UI in a browser. Installed apps
// always use their own Rust engine through Tauri IPC, including phones.
const base = import.meta.env.VITE_API_URL || "http://127.0.0.1:8788";
async function request<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${base}/api${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || "Request failed");
  return value;
}
const bridgedApi = {
  archiveTemplates: () =>
    native
      ? invoke<ArchiveTemplateOption[]>("archive_templates")
      : request<ArchiveTemplateOption[]>("/templates"),
  previewArchiveTemplate: (template: ArchiveTemplate) =>
    native
      ? invoke<string>("preview_archive_template", { template })
      : request<string>(`/templates/${template}/preview`),
  settings: () =>
    native ? invoke<Settings>("get_settings") : request<Settings>("/settings"),
  updateSettings: (settings: Settings) =>
    native
      ? invoke<Settings>("update_settings", { settings })
      : request<Settings>("/settings", settings),
  capabilities: () =>
    native
      ? invoke<Capabilities>("capabilities")
      : request<Capabilities>("/health"),
  list: () => (native ? invoke<Job[]>("list_jobs") : request<Job[]>("/jobs")),
  get: (id: string) =>
    native ? invoke<Job>("get_job", { id }) : request<Job>(`/jobs/${id}`),
  create: (config: Config) =>
    native
      ? invoke<Job>("create_job", { config })
      : request<Job>("/jobs", config),
  pause: (id: string) =>
    native
      ? invoke<Job>("pause_job", { id })
      : request<Job>(`/jobs/${id}/pause`, {}),
  resume: (id: string) =>
    native
      ? invoke<Job>("resume_job", { id })
      : request<Job>(`/jobs/${id}/resume`, {}),
  editRetry: (id: string, config: Config) =>
    native
      ? invoke<Job>("edit_job", { id, config })
      : request<Job>(`/jobs/${id}/edit`, config),
  build: (id: string, selected: string[]) =>
    native
      ? invoke<Job>("build_job", { id, selected })
      : request<Job>(`/jobs/${id}/build`, { selected }),
  rebuild: (id: string, template: ArchiveTemplate) =>
    native
      ? invoke<Job>("rebuild_job", { id, template })
      : request<Job>(`/jobs/${id}/rebuild`, { template }),
  refresh: (id: string) =>
    native
      ? invoke<Job>("refresh_job", { id })
      : request<Job>(`/jobs/${id}/refresh`, {}),
  open: async (id: string): Promise<void> => {
    if (!native)
      throw new Error(
        "Save the ZIM file and open it with a ZIM viewer on this device.",
      );
    await invoke("open_archive", { id });
  },
  export: async (id: string): Promise<string> => {
    if (native) return invoke<string>("export_archive", { id });
    const anchor = document.createElement("a");
    anchor.href = `${base}/api/jobs/${id}/artifact`;
    anchor.download = `${id}.zim`;
    anchor.click();
    return "Download started";
  },
};
export const api = browserOnly ? browserApi : bridgedApi;

export async function repairYouTubeAudio(id: string): Promise<Job> {
  if (!native) throw new Error("YouTube audio repair requires the native app.");
  return invoke<Job>("repair_youtube_audio", { id });
}

export async function buildAvailable(id: string): Promise<Job> {
  if (!native)
    throw new Error("Building available files requires the native app.");
  return invoke<Job>("build_available_job", { id });
}

export async function deleteFailed(id: string): Promise<boolean> {
  if (browserOnly) return browserCommand<boolean>("delete_failed_job", { id });
  if (!native)
    throw new Error("Deleting failed attempts requires the native app.");
  return invoke<boolean>("delete_failed_job", { id });
}

function importCommand<T>(
  command: string,
  args: Record<string, unknown>,
): Promise<T> {
  if (browserOnly) return browserCommand<T>(command, args);
  if (!native)
    throw new Error(
      "Local file import requires the installed app or the browser-only build.",
    );
  return invoke<T>(command, args);
}
export const savedImports = {
  begin: (
    config: Config,
    files: { path: string; size: number }[],
    existingId: string | null,
  ) => importCommand<Job>("begin_import", { config, files, existingId }),
  offset: (id: string, path: string) =>
    importCommand<number>("import_offset", { id, path }),
  chunk: (id: string, path: string, offset: number, data: string) =>
    importCommand<number>("import_chunk", { id, path, offset, data }),
  finish: (id: string) => importCommand<Job>("finish_import", { id }),
  cancel: (id: string) => importCommand<void>("cancel_import", { id }),
};
