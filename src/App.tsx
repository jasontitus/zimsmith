import { useCallback, useEffect, useRef, useState } from "react";
import {
  Archive,
  ArrowDown,
  ArrowDownToLine,
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  Clock3,
  FileText,
  FolderArchive,
  Globe2,
  Link2,
  ExternalLink,
  ListFilter,
  LoaderCircle,
  MonitorSmartphone,
  MoreHorizontal,
  Pause,
  Pencil,
  Play,
  Plus,
  Radio,
  Search,
  Settings2,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Video,
  Trash2,
  X,
} from "lucide-react";
import { CaptureReport } from "./CaptureReport";
import { SavedContentImport } from "./SavedContentImport";
import { CaptureSettings } from "./CaptureSettings";
import { NumberInput } from "./NumberInput";
import { normalizedCapture, captureOptions } from "./capture";
import { AndroidUpdates } from "./AndroidUpdates";
import {
  api,
  native,
  repairYouTubeAudio,
  refreshJavascript,
  buildAvailable,
  deleteFailed,
} from "./api";
import { browserOnly, extensionMode } from "./browser/client";
import brandIcon from "./assets/zimsmith-icon.png";
import { ArchiveTemplates } from "./ArchiveTemplates";
import { JobProgress } from "./JobProgress";
import { SharedLinks, dismissSharedLink, type SharedLink } from "./SharedLinks";
import { isNativeMediaUrl, normalizeSourceUrl } from "./source";
import type {
  ArchiveTemplate,
  ArchiveTemplateOption,
  Capabilities,
  Config,
  Entry,
  Job,
  Settings,
  SourceKind,
} from "./types";

const defaults: Config = {
  url: "",
  title: "",
  kind: "website",
  max_items: 100,
  max_depth: 2,
  delay_ms: 2000,
  max_resource_mb: 256,
  language: "eng",
  description: "An offline collection created with Zimsmith",
  creator: "Zimsmith user",
  include_assets: true,
};
const activeStates = [
  "scanning",
  "downloading",
  "packaging",
  "pausing",
  "importing",
];
const defaultSettings: Settings = {
  auto_start_small_jobs: true,
  auto_start_item_limit: 100,
  archive_template: "atlas",
  zstd_level: 3,
  cdn_parallel_requests: 3,
};
const labels: Record<string, string> = {
  importing: "Importing saved files",
  scanning: "Scanning",
  review: "Ready for review",
  downloading: "Downloading",
  packaging: "Building archive",
  pausing: "Pausing…",
  paused: "Paused",
  failed: "Needs attention",
  completed: "Ready offline",
};
const bytes = (value: number | null) =>
  value === null
    ? "Unknown"
    : value < 1024
      ? `${value} B`
      : value < 1024 ** 2
        ? `${(value / 1024).toFixed(1)} KB`
        : value < 1024 ** 3
          ? `${(value / 1024 ** 2).toFixed(1)} MB`
          : `${(value / 1024 ** 3).toFixed(2)} GB`;
const duration = (seconds: number | null) =>
  seconds === null
    ? "—"
    : `${Math.floor(seconds / 60)}:${Math.floor(seconds % 60)
        .toString()
        .padStart(2, "0")}`;
const friendlyError = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export default function App() {
  const [tab, setTab] = useState("create");
  const [jobs, setJobs] = useState<Job[]>([]);
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsDraft, setSettingsDraft] = useState<Settings>(defaultSettings);
  const [connection, setConnection] = useState("connecting");
  const [selectedJob, setSelectedJob] = useState<string | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [config, setConfig] = useState<Config>(defaults);
  const [importResumeId, setImportResumeId] = useState<string | null>(null);
  const [editingJobId, setEditingJobId] = useState<string | null>(null);
  const [sharedLinkId, setSharedLinkId] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [toast, setToast] = useState("");
  const [selection, setSelection] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState("");
  const [collectionPage, setCollectionPage] = useState(0);
  const [confirm, setConfirm] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [templates, setTemplates] = useState<ArchiveTemplateOption[]>([]);
  const [rebuildTemplate, setRebuildTemplate] =
    useState<ArchiveTemplate | null>(null);
  const lastJob = useRef<string | null>(null);
  const deletedJobs = useRef(new Set<string>());
  const lastStatus = useRef<string | null>(null);
  const currentForm = useRef({ config, editingJobId, busy, tab });
  currentForm.current = { config, editingJobId, busy, tab };
  const reviewSharedLink = useCallback(
    (link: SharedLink, automatic: boolean) => {
      if (!link.url || currentForm.current.busy) return;
      if (
        automatic &&
        ((currentForm.current.tab === "create" &&
          currentForm.current.config.url) ||
          currentForm.current.editingJobId)
      )
        return;
      const url = normalizeSourceUrl(link.url);
      setConfig({
        ...defaults,
        url,
        title: link.title,
        kind: isNativeMediaUrl(url) ? "media" : "website",
      });
      setEditingJobId(null);
      setSharedLinkId(link.id);
      setSelectedJob(null);
      setJob(null);
      setTab("create");
      setError("");
      setToast("Ready to scan. Tap Scan shared link to begin.");
      window.scrollTo(0, 0);
    },
    [],
  );

  const refresh = useCallback(async () => {
    try {
      const list = await api.list();
      setJobs(list.filter((item) => !deletedJobs.current.has(item.id)));
      setConnection("connected");
      if (!settings) {
        const saved = { ...defaultSettings, ...(await api.settings()) };
        setSettings(saved);
        setSettingsDraft(saved);
      }
      if (selectedJob && !deletedJobs.current.has(selectedJob)) {
        const summary = list.find((item) => item.id === selectedJob);
        if (
          summary?.entry_count !== undefined &&
          lastJob.current === selectedJob &&
          lastStatus.current === summary.status
        ) {
          setJob((previous) =>
            previous?.id === selectedJob
              ? { ...summary, entries: previous.entries }
              : previous,
          );
          return;
        }
        const detail = await api.get(selectedJob);
        if (deletedJobs.current.has(selectedJob)) return;
        setJob(detail);
        if (lastJob.current !== detail.id) {
          setSelection(
            new Set(detail.entries.filter((e) => e.selected).map((e) => e.id)),
          );
          lastJob.current = detail.id;
        } else if (
          !["review", "failed"].includes(detail.status) ||
          lastStatus.current !== detail.status
        )
          setSelection(
            new Set(detail.entries.filter((e) => e.selected).map((e) => e.id)),
          );
        lastStatus.current = detail.status;
      }
    } catch (error) {
      if (selectedJob && deletedJobs.current.has(selectedJob)) return;
      setConnection("disconnected");
      if (browserOnly) setError(friendlyError(error));
    }
  }, [selectedJob, settings]);
  useEffect(() => {
    void api
      .archiveTemplates()
      .then(setTemplates)
      .catch(() => {});
    void api
      .capabilities()
      .then(setCapabilities)
      .catch(() => {});
  }, []);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 1500);
    return () => clearInterval(timer);
  }, [refresh]);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(""), 6000);
    return () => clearTimeout(timer);
  }, [toast]);
  useEffect(() => {
    if (!["ios", "android"].includes(capabilities?.platform || "")) return;
    const pauseOnHide = () => {
      if (document.hidden)
        for (const item of jobs.filter(
          (j) => activeStates.includes(j.status) && j.status !== "pausing",
        ))
          void api.pause(item.id).catch(() => {});
    };
    document.addEventListener("visibilitychange", pauseOnHide);
    return () => document.removeEventListener("visibilitychange", pauseOnHide);
  }, [capabilities, jobs]);

  async function action(run: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    try {
      await run();
      await refresh();
    } catch (error) {
      setError(friendlyError(error));
    } finally {
      setBusy(false);
    }
  }
  const openJob = (item: Job) => {
    window.scrollTo(0, 0);
    setImportResumeId(null);
    setEditingJobId(null);
    setRebuildTemplate(null);
    lastJob.current = null;
    setSelectedJob(item.id);
    setJob(item);
    setSelection(
      new Set(item.entries.filter((e) => e.selected).map((e) => e.id)),
    );
    setTab("job");
    setFilter("");
    setCollectionPage(0);
    setError("");
  };
  const editCrawl = (item: Job) => {
    setSharedLinkId(null);
    setConfig({ ...item.config });
    setEditingJobId(item.id);
    setAdvanced(true);
    setTab("create");
    setError("");
    window.scrollTo(0, 0);
  };
  const newArchive = () => {
    setImportResumeId(null);
    if (editingJobId) setConfig(defaults);
    setEditingJobId(null);
    setTab("create");
    setSelectedJob(null);
    setJob(null);
    setError("");
  };
  const update = <K extends keyof Config>(key: K, value: Config[K]) =>
    setConfig((c) => ({ ...c, [key]: value }));
  const create = () =>
    action(async () => {
      setToast("");
      const normalized = {
        ...config,
        url: normalizeSourceUrl(config.url),
        capture: normalizedCapture(config),
      };
      const result = editingJobId
        ? await api.editRetry(editingJobId, normalized)
        : await api.create(normalized);
      if (sharedLinkId) {
        try {
          await dismissSharedLink(sharedLinkId);
          setSharedLinkId(null);
        } catch {
          setToast(
            "Crawl created. The saved share could not be cleared; dismiss it from Shared links.",
          );
        }
      }
      if (editingJobId) {
        setToast(
          result.config.delay_ms === job?.config.delay_ms
            ? "Crawl settings saved. Retrying."
            : `Crawl settings saved. Page request delay is now ${result.config.delay_ms / 1000} seconds.`,
        );
      }
      setConfig(normalized);
      openJob(result);
    });
  const openArchive = (id: string) => void action(() => api.open(id));
  const deleteAttempt = async () => {
    if (!job) return;
    setBusy(true);
    setError("");
    try {
      const cleaned = await deleteFailed(job.id);
      deletedJobs.current.add(job.id);
      setJobs((items) => items.filter((item) => item.id !== job.id));
      setSelectedJob(null);
      setJob(null);
      setTab("queue");
      setConfirmDelete(false);
      setToast(
        cleaned
          ? "Failed attempt and cached files deleted."
          : "Failed attempt removed. Cached file cleanup will be retried on next launch.",
      );
    } catch (error) {
      setError(friendlyError(error));
      setConfirmDelete(false);
    } finally {
      setBusy(false);
    }
  };
  const saveSettings = (
    section: "downloads" | "appearance" | "compression" | "cdn" = "downloads",
  ) =>
    action(async () => {
      if (!settings) return;
      const changes =
        section === "appearance"
          ? { archive_template: settingsDraft.archive_template }
          : section === "compression"
            ? { zstd_level: settingsDraft.zstd_level }
            : section === "cdn"
              ? { cdn_parallel_requests: settingsDraft.cdn_parallel_requests }
              : {
                  auto_start_small_jobs: settingsDraft.auto_start_small_jobs,
                  auto_start_item_limit:
                    !settingsDraft.auto_start_small_jobs &&
                    !Number.isFinite(settingsDraft.auto_start_item_limit)
                      ? settings.auto_start_item_limit
                      : settingsDraft.auto_start_item_limit,
                };
      const saved = await api.updateSettings({ ...settings, ...changes });
      setSettings(saved);
      setSettingsDraft((previous) => ({ ...previous, ...changes }));
      setToast(
        section === "appearance"
          ? "Archive style saved. Applies to new jobs."
          : section === "compression"
            ? "Compression preference saved. Applies to new jobs and rebuilds."
            : section === "cdn"
              ? "CDN preference saved. Applies when downloads start or resume."
              : "Download preferences saved. Applies to new jobs.",
      );
    });
  const visibleJobs = jobs.filter((j) =>
    tab === "library" ? j.status === "completed" : j.status !== "completed",
  );
  const selectedEntries = job?.entries.filter((e) => selection.has(e.id)) || [];
  const knownSize = selectedEntries.reduce((sum, e) => sum + (e.size || 0), 0);
  const filterText = filter.toLowerCase();
  const matchingEntries = (job?.entries || []).filter((entry) =>
    `${entry.title} ${entry.url}`.toLowerCase().includes(filterText),
  );
  const pageSize = 100;
  const pageCount = Math.ceil(matchingEntries.length / pageSize);
  const currentPage = Math.max(0, Math.min(collectionPage, pageCount - 1));
  const visibleEntries = matchingEntries.slice(
    currentPage * pageSize,
    (currentPage + 1) * pageSize,
  );

  const running = jobs.filter((j) => activeStates.includes(j.status)).length;
  const totalOffline = jobs.filter((j) => j.status === "completed").length;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <button
          className="brand"
          onClick={newArchive}
          aria-label="Zimsmith home"
        >
          <img
            className="brand-icon"
            src={brandIcon}
            alt=""
            width={36}
            height={36}
          />
          <span>Zimsmith</span>
        </button>
        <div className="workspace-label">YOUR WORKSPACE</div>
        <nav>
          <button
            className={
              tab === "create" || tab === "job" ? "nav-item active" : "nav-item"
            }
            onClick={newArchive}
          >
            <Plus size={19} />
            Create an archive
          </button>
          <button
            className={tab === "queue" ? "nav-item active" : "nav-item"}
            onClick={() => {
              setTab("queue");
              setSelectedJob(null);
            }}
          >
            <Radio size={18} />
            Activity
            {running > 0 && <span className="nav-count">{running}</span>}
          </button>
          <button
            className={tab === "library" ? "nav-item active" : "nav-item"}
            onClick={() => {
              setTab("library");
              setSelectedJob(null);
            }}
          >
            <FolderArchive size={19} />
            My archives<span className="nav-count neutral">{totalOffline}</span>
          </button>
        </nav>
        <div className="sidebar-bottom">
          <div className="local-note">
            <div className="small-icon">
              <MonitorSmartphone size={19} />
            </div>
            <strong>Made to go with you.</strong>
            <p>Your crawls and archives stay on this device.</p>
            <span>
              <span className="tiny-dot" /> Local by design
            </span>
          </div>
          <button
            className={tab === "settings" ? "nav-item active" : "nav-item"}
            onClick={() => {
              setTab("settings");
              setSelectedJob(null);
            }}
          >
            <Settings2 size={18} />
            Settings
          </button>
          <button className="nav-item" onClick={() => setShowHelp(true)}>
            <CircleHelp size={18} />
            Quick guide
            <ChevronRight className="push" size={15} />
          </button>
          <div className="sidebar-footer">
            <span className="avatar">ZS</span>
            <div>
              <strong>Personal workspace</strong>
              <small>Zimsmith · v0.1</small>
            </div>
            <MoreHorizontal size={18} />
          </div>
        </div>
      </aside>

      <div className="main-shell">
        <header className="topbar">
          <div className="breadcrumb">
            Workspace
            <ChevronRight size={14} />
            <span>
              {tab === "create"
                ? editingJobId
                  ? "Edit crawl"
                  : "Create an archive"
                : tab === "job"
                  ? job?.config.title || "Archive"
                  : tab === "queue"
                    ? "Activity"
                    : tab === "library"
                      ? "My archives"
                      : "Settings"}
            </span>
          </div>
          <div className="engine-status">
            <span className={`status-dot ${connection}`} />
            {connection === "connected"
              ? "On-device engine"
              : connection === "connecting"
                ? "Connecting to engine"
                : "Engine unavailable"}
            <span className="platform-tag">
              {native
                ? capabilities?.platform || "Native"
                : browserOnly
                  ? extensionMode
                    ? "Extension"
                    : "Browser"
                  : "Browser preview"}
            </span>
          </div>
          <button
            className="mobile-settings"
            aria-label="Open settings"
            onClick={() => {
              setTab("settings");
              setSelectedJob(null);
            }}
          >
            <Settings2 size={17} />
          </button>
        </header>
        <main className="main-content">
          {native &&
            ["ios", "android"].includes(capabilities?.platform || "") && (
              <SharedLinks
                selected={sharedLinkId}
                onReview={reviewSharedLink}
                onScan={() => {
                  void create();
                }}
                busy={busy}
                onDismiss={(id) => {
                  if (sharedLinkId === id) setSharedLinkId(null);
                }}
              />
            )}
          {native && capabilities?.platform === "android" && (
            <AndroidUpdates
              settingsVisible={tab === "settings"}
              active={running > 0}
            />
          )}
          {error && (
            <div className="alert error" role="alert">
              <span>{error}</span>
              <button aria-label="Dismiss error" onClick={() => setError("")}>
                <X size={16} />
              </button>
            </div>
          )}
          {connection === "disconnected" && (
            <div className="alert warning">
              {native
                ? "The local engine is unavailable. Restart the app to recover saved jobs."
                : browserOnly
                  ? "The browser engine is unavailable. Close other Zimsmith workspace tabs, then reload. Saved jobs stay in this browser profile."
                  : "Browser preview needs the local development API. Run npm run worker in a second terminal, or launch the native app with npm run tauri dev."}
            </div>
          )}
          {browserOnly && (
            <div className="browser-notice" role="note">
              <Globe2 size={20} aria-hidden="true" />
              <div>
                <strong>
                  {extensionMode
                    ? "Your browser is the engine."
                    : "Browser-only workspace"}
                </strong>
                <p>
                  {extensionMode
                    ? "Grant website access when starting a crawl. Keep this tab open while jobs run; closing it saves checkpoints for Resume."
                    : "No server is used. Sources must allow CORS; use the extension to crawl other websites. Keep this tab open while jobs run."}{" "}
                  Website crawls and direct video/audio files are supported.
                  YouTube collections currently require the native app. Save
                  finished ZIMs to Downloads; clearing browser data removes
                  cached files and checkpoints.
                </p>
              </div>
            </div>
          )}

          {tab === "create" && (
            <>
              <div className="page-heading">
                <div className="eyebrow">
                  <span className="tiny-dot" /> A LITTLE LESS ONLINE. A LOT MORE
                  POSSIBLE.
                </div>
                <h1>
                  {editingJobId
                    ? "Edit your crawl."
                    : "Keep a piece of the web."}
                </h1>
                <p>
                  {editingJobId
                    ? "Update the source or settings, then retry. Source and scan-limit changes restart discovery; saved files are kept for reuse where applicable."
                    : "Turn websites and video collections into searchable offline archives."}
                </p>
              </div>
              {!editingJobId && (native || browserOnly) && (
                <SavedContentImport
                  config={config}
                  resumeId={importResumeId}
                  onCreated={openJob}
                  onBusyChange={setBusy}
                />
              )}
              <div className="create-layout" hidden={!!importResumeId}>
                <section className="card source-card">
                  <div className="card-heading">
                    <span className="step-chip">01</span>
                    <div>
                      <h2>Start with a source</h2>
                      <p>Where would you like to explore?</p>
                    </div>
                    <Link2 className="muted push" size={20} />
                  </div>
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void create();
                    }}
                  >
                    <div
                      className="source-types"
                      role="group"
                      aria-label="Source type"
                    >
                      {(["website", "media"] as SourceKind[]).map((kind) => (
                        <button
                          type="button"
                          key={kind}
                          className={
                            config.kind === kind
                              ? "source-type chosen"
                              : "source-type"
                          }
                          onClick={() => update("kind", kind)}
                          aria-pressed={config.kind === kind}
                          disabled={
                            kind === "website" && isNativeMediaUrl(config.url)
                          }
                        >
                          {kind === "website" ? (
                            <Globe2 size={23} />
                          ) : (
                            <Video size={23} />
                          )}
                          <span>
                            <strong>
                              {kind === "website"
                                ? "Website"
                                : "Video collection"}
                            </strong>
                            <small>
                              {kind === "website"
                                ? "Pages, articles & resources"
                                : "Channels, playlists & searches"}
                            </small>
                          </span>
                          <span className="radio-circle">
                            {config.kind === kind && <span />}
                          </span>
                        </button>
                      ))}
                    </div>
                    <label htmlFor="source-url">
                      {config.kind === "website"
                        ? "Website URL"
                        : "Collection or media URL"}
                    </label>
                    <div className="input-with-icon">
                      <Link2 size={18} />
                      <input
                        id="source-url"
                        type="text"
                        inputMode="url"
                        autoCapitalize="none"
                        autoCorrect="off"
                        spellCheck={false}
                        required
                        placeholder={
                          config.kind === "website"
                            ? "cnn.com or https://example.com"
                            : "youtube.com/@channel or a media URL"
                        }
                        value={config.url}
                        onBlur={() => {
                          try {
                            update("url", normalizeSourceUrl(config.url));
                          } catch {
                            /* Show validation errors on submit without changing the draft. */
                          }
                        }}
                        onChange={(e) => {
                          update("url", e.target.value);
                          if (isNativeMediaUrl(e.target.value))
                            update("kind", "media");
                          else if (isNativeMediaUrl(config.url))
                            update("kind", "website");
                          if (
                            !config.title ||
                            config.title === titleFromUrl(config.url)
                          )
                            update("title", titleFromUrl(e.target.value));
                        }}
                      />
                    </div>
                    <p className="field-hint">
                      {isNativeMediaUrl(config.url)
                        ? "Media URL detected. We’ll scan this collection locally."
                        : config.kind === "website"
                          ? "We’ll stay within this site and URL path. Start at the top to include more."
                          : "YouTube is extracted locally. Other desktop sources can use yt-dlp."}{" "}
                      Hostnames work too; HTTPS is added automatically.
                    </p>
                    <div className="form-grid">
                      <div>
                        <label htmlFor="title">Archive title</label>
                        <input
                          id="title"
                          required
                          maxLength={120}
                          placeholder="A name for your collection"
                          value={config.title}
                          onChange={(e) => update("title", e.target.value)}
                        />
                      </div>
                      <div>
                        <label htmlFor="limit">
                          Scan limit <span className="optional">items</span>
                        </label>
                        <NumberInput
                          id="limit"
                          min={1}
                          max={5000}
                          required
                          value={config.max_items}
                          onValueChange={(value) => update("max_items", value)}
                        />
                      </div>
                    </div>
                    {native && config.kind === "website" && (
                      <div>
                        <label htmlFor="page-loading">Page loading</label>
                        <select
                          id="page-loading"
                          value={
                            captureOptions(config).render_javascript
                              ? "always"
                              : captureOptions(config).auto_render_javascript
                                ? "auto"
                                : "static"
                          }
                          onChange={(e) =>
                            update("capture", {
                              ...captureOptions(config),
                              render_javascript: e.target.value === "always",
                              auto_render_javascript: e.target.value === "auto",
                            })
                          }
                        >
                          <option value="auto">
                            Automatic · render dynamic pages when needed
                          </option>
                          <option value="always">
                            Render JavaScript on every page
                          </option>
                          <option value="static">Static HTML only</option>
                        </select>
                        <p className="field-hint">
                          Automatic detects empty app shells and dynamic
                          catalogs. Rendering runs on this device and loads
                          scripts and page resources during scanning. Rendered
                          captures always require review.
                        </p>
                      </div>
                    )}
                    <button
                      type="button"
                      className="advanced-toggle"
                      aria-expanded={advanced}
                      onClick={() => setAdvanced(!advanced)}
                    >
                      <Settings2 size={16} />
                      Crawl & archive settings
                      <ChevronDown
                        size={15}
                        className={advanced ? "rotated" : ""}
                      />
                    </button>
                    {advanced && (
                      <div className="advanced-settings">
                        <div className="form-grid">
                          <div>
                            <label htmlFor="depth">Link depth</label>
                            <NumberInput
                              id="depth"
                              min={0}
                              max={20}
                              required
                              value={config.max_depth}
                              onValueChange={(value) =>
                                update("max_depth", value)
                              }
                            />
                          </div>
                          <div>
                            <label htmlFor="delay">
                              Delay between page requests (seconds)
                            </label>
                            <NumberInput
                              id="delay"
                              aria-describedby="delay-help"
                              min={1}
                              max={60}
                              step={0.5}
                              required
                              value={config.delay_ms / 1000}
                              onValueChange={(value) =>
                                update("delay_ms", value * 1000)
                              }
                            />
                            <p id="delay-help" className="settings-note">
                              Pages and ordinary hosts wait at least this long
                              between requests.{" "}
                              {!browserOnly && (
                                <>
                                  To set how many CDN files can download at
                                  once, use Settings → CDN downloads.
                                </>
                              )}
                            </p>
                          </div>
                          <div>
                            <label htmlFor="resource-limit">
                              Maximum file size (MB)
                            </label>
                            <NumberInput
                              id="resource-limit"
                              min={1}
                              max={1024}
                              required
                              value={config.max_resource_mb}
                              onValueChange={(value) =>
                                update("max_resource_mb", value)
                              }
                            />
                          </div>
                          <div>
                            <label htmlFor="language">
                              Archive language (ISO 639-3)
                            </label>
                            <input
                              id="language"
                              minLength={3}
                              maxLength={3}
                              pattern="[a-z]{3}"
                              value={config.language}
                              onChange={(e) =>
                                update("language", e.target.value)
                              }
                            />
                            <p className="settings-note">
                              Labels the ZIM; content is not translated. YouTube
                              downloads use original audio.
                            </p>
                          </div>
                        </div>
                        <CaptureSettings
                          config={config}
                          onChange={(value) => update("capture", value)}
                        />
                        <label htmlFor="description">Short description</label>
                        <input
                          id="description"
                          required
                          maxLength={80}
                          value={config.description}
                          onChange={(e) =>
                            update("description", e.target.value)
                          }
                        />
                        <label htmlFor="creator">Content creator</label>
                        <input
                          id="creator"
                          required
                          value={config.creator}
                          onChange={(e) => update("creator", e.target.value)}
                        />
                        <label className="check-label">
                          <input
                            type="checkbox"
                            checked={config.include_assets}
                            onChange={(e) =>
                              update("include_assets", e.target.checked)
                            }
                          />
                          Include images, stylesheets & linked media
                        </label>
                      </div>
                    )}
                    <div className="scan-note">
                      <ShieldCheck size={19} />
                      <p>
                        <strong>A considerate crawler.</strong> Paced requests,
                        robots.txt rules, and automatic backoff when a website
                        asks us to slow down.
                      </p>
                    </div>
                    <p className="auto-start-note">
                      {settings?.auto_start_small_jobs
                        ? `Complete, error-free scans of up to ${settings.auto_start_item_limit} items download and build automatically. Item count does not limit total download size.`
                        : "Every scan stops for review before downloading assets or media."}{" "}
                      <button
                        type="button"
                        className="text-button"
                        onClick={() => {
                          setTab("settings");
                          setSelectedJob(null);
                        }}
                      >
                        Change in Settings
                      </button>
                    </p>
                    <div
                      className={`form-footer${editingJobId ? " editing" : ""}`}
                    >
                      <span>
                        <Search size={14} />
                        {editingJobId
                          ? "Save changes and retry this crawl."
                          : settings?.auto_start_small_jobs
                            ? "Scan first. Small jobs start automatically."
                            : "Scan first. You decide what to keep."}
                      </span>
                      {editingJobId && (
                        <button
                          className="button secondary"
                          type="button"
                          disabled={busy}
                          onClick={() => {
                            if (job) openJob(job);
                          }}
                        >
                          Cancel edit
                        </button>
                      )}
                      <button
                        className="button primary"
                        type="submit"
                        disabled={
                          busy || connection !== "connected" || !settings
                        }
                      >
                        {busy ? (
                          <LoaderCircle className="spin" size={17} />
                        ) : (
                          <Search size={17} />
                        )}
                        {editingJobId ? "Save & retry" : "Scan source"}
                        {!editingJobId && <ArrowRight size={17} />}
                      </button>
                    </div>
                  </form>
                </section>
                <aside className="guide-column">
                  <div className="archive-illustration" aria-hidden="true">
                    <div className="orbit orbit-one" />
                    <div className="orbit orbit-two" />
                    <div className="floating-page page-back">
                      <div />
                      <div />
                      <div />
                    </div>
                    <div className="floating-page page-front">
                      <span className="paper-icon">
                        <Globe2 size={25} />
                      </span>
                      <div />
                      <div />
                      <div />
                    </div>
                    <span className="floating-badge">
                      <Check size={16} />
                    </span>
                    <span className="illustration-caption">
                      A WORLD OF KNOWLEDGE.
                      <br />
                      <strong>ONE FILE. ALWAYS WITH YOU.</strong>
                    </span>
                  </div>
                  <div className="guide-text">
                    <h3>From a link to your library.</h3>
                    <p>
                      Three small steps. A whole collection to take anywhere.
                    </p>
                    <ol className="steps">
                      <li>
                        <span>1</span>
                        <div>
                          <strong>Discover what’s there</strong>
                          <p>
                            Scan pages or collection metadata to see what’s
                            available.
                          </p>
                        </div>
                      </li>
                      <li>
                        <span>2</span>
                        <div>
                          <strong>Make it your collection</strong>
                          <p>
                            {settings?.auto_start_small_jobs
                              ? "Small jobs start automatically. Review larger collections and choose what to keep."
                              : "Preview the list, choose your items, and confirm the download."}
                          </p>
                        </div>
                      </li>
                      <li>
                        <span>3</span>
                        <div>
                          <strong>Take it offline</strong>
                          <p>
                            Get a ZIM file with contents and search. Open it in
                            Kiwix.
                          </p>
                        </div>
                      </li>
                    </ol>
                  </div>
                  <div className="portable-note">
                    <Archive size={18} />
                    <p>
                      <strong>Small file. Big possibilities.</strong>
                      <br />
                      An open format, ready for your next adventure.
                    </p>
                  </div>
                </aside>
              </div>
              <section className="recent-section">
                <div className="section-header">
                  <h2>
                    Recent activity <span>{jobs.length}</span>
                  </h2>
                  <button
                    className="text-button"
                    onClick={() => setTab("queue")}
                  >
                    View all
                    <ArrowRight size={15} />
                  </button>
                </div>
                {jobs.length === 0 ? (
                  <div className="empty-recent">
                    <span className="empty-icon">
                      <FolderArchive size={23} />
                    </span>
                    <div>
                      <strong>Your next discovery starts here.</strong>
                      <p>
                        Scan your first source to bring a collection offline.
                      </p>
                    </div>
                    <span className="empty-tag">Ready when you are</span>
                  </div>
                ) : (
                  <div className="job-list">
                    {jobs.slice(0, 3).map((item) => (
                      <JobRow
                        key={item.id}
                        item={item}
                        onClick={() => openJob(item)}
                        onOpen={
                          native && item.status === "completed"
                            ? () => openArchive(item.id)
                            : undefined
                        }
                      />
                    ))}
                  </div>
                )}
              </section>
              <div className="bottom-note">
                <ShieldCheck size={14} />
                Your device. Your collection.<span>•</span>
                <span>No account needed.</span>
                <span className="push">
                  Built on open standards <BookOpen size={13} />
                </span>
              </div>
            </>
          )}

          {(tab === "queue" || tab === "library") && (
            <>
              <div className="page-heading compact">
                <div className="eyebrow">YOUR WORKSPACE</div>
                <div className="heading-line">
                  <div>
                    <h1>
                      {tab === "queue"
                        ? "Every collection, in motion."
                        : "Your offline library."}
                    </h1>
                    <p>
                      {tab === "queue"
                        ? "Follow scans, manage downloads, and pick up where you left off."
                        : "Searchable collections, ready to take anywhere."}
                    </p>
                  </div>
                  <button className="button primary" onClick={newArchive}>
                    <Plus size={17} />
                    New archive
                  </button>
                </div>
              </div>
              <div className="stat-grid">
                <Stat
                  title={tab === "queue" ? "Running now" : "Archives ready"}
                  value={
                    tab === "queue"
                      ? running.toString()
                      : totalOffline.toString()
                  }
                  icon={<Radio size={20} />}
                />
                <Stat
                  title="Saved locally"
                  value={bytes(
                    jobs.reduce((s, j) => s + (j.output_bytes || 0), 0),
                  )}
                  icon={<Archive size={20} />}
                />
                <Stat
                  title="Items discovered"
                  value={jobs
                    .reduce(
                      (s, j) => s + (j.entry_count ?? j.entries.length),
                      0,
                    )
                    .toString()}
                  icon={<FileText size={20} />}
                />
              </div>
              <section className="card collection-list">
                <div className="section-header">
                  <h2>
                    {tab === "queue"
                      ? "In progress & awaiting review"
                      : "Completed archives"}
                  </h2>
                  <span className="muted">
                    {visibleJobs.length} collections
                  </span>
                </div>
                {visibleJobs.length ? (
                  visibleJobs.map((item) => (
                    <JobRow
                      key={item.id}
                      item={item}
                      onClick={() => openJob(item)}
                      onOpen={
                        native && item.status === "completed"
                          ? () => openArchive(item.id)
                          : undefined
                      }
                    />
                  ))
                ) : (
                  <div className="empty-large">
                    <FolderArchive size={38} />
                    <h3>
                      {tab === "queue"
                        ? "All quiet here."
                        : "A library waiting to happen."}
                    </h3>
                    <p>Create an archive to see it here.</p>
                    <button className="button secondary" onClick={newArchive}>
                      Create an archive
                      <ArrowRight size={16} />
                    </button>
                  </div>
                )}
              </section>
            </>
          )}

          {tab === "job" && job && (
            <>
              <button
                className="back-link"
                onClick={() => {
                  setTab("queue");
                  setSelectedJob(null);
                }}
              >
                <ArrowLeft size={15} />
                All activity
              </button>
              <div className="page-heading compact">
                <div className="heading-line">
                  <div>
                    <div className="eyebrow">
                      {job.config.kind === "website"
                        ? "WEBSITE COLLECTION"
                        : "MEDIA COLLECTION"}
                    </div>
                    <h1>{job.config.title}</h1>
                    <p className="source-address">
                      {job.import ? "Imported local files" : job.config.url}
                    </p>
                  </div>
                  <Status status={job.status} />
                </div>
              </div>
              <div className="workflow-bar">
                {[
                  "Scan source",
                  "Review selection",
                  "Download",
                  "Build ZIM",
                ].map((label, i) => {
                  const current =
                    job.phase === "scan"
                      ? job.status === "review"
                        ? 1
                        : 0
                      : job.phase === "download"
                        ? 2
                        : 3;
                  return (
                    <div
                      key={label}
                      className={
                        i === current
                          ? "workflow-step current"
                          : i < current || job.status === "completed"
                            ? "workflow-step done"
                            : "workflow-step"
                      }
                    >
                      <span>
                        {i < current || job.status === "completed" ? (
                          <Check size={14} />
                        ) : (
                          i + 1
                        )}
                      </span>
                      {label}
                      {i < 3 && <ChevronRight size={15} />}
                    </div>
                  );
                })}
              </div>
              {job.error && (
                <div className="alert error" role="alert">
                  {job.error}
                </div>
              )}
              <JobProgress job={job} />
              {["paused", "failed"].includes(job.status) &&
                (!job.import || job.import.finished) && (
                  <section className="panel rebuild-panel">
                    <div>
                      <h2>Continue this crawl</h2>
                      <p>
                        Saved files are reused. Only unfinished items are
                        retried.
                      </p>
                    </div>
                    <button
                      className="button primary"
                      disabled={busy}
                      onClick={() => void action(() => api.resume(job.id))}
                    >
                      <Play size={16} />
                      {job.status === "failed" ? "Retry & resume" : "Resume"}
                    </button>
                  </section>
                )}
              {job.import && !job.import.finished && (
                <section className="panel rebuild-panel">
                  <div>
                    <h2>Continue this saved-file import</h2>
                    <p>
                      Reselect the original files or folder to reuse uploaded
                      chunks.
                    </p>
                  </div>
                  <button
                    className="button secondary"
                    disabled={busy}
                    onClick={() => {
                      setImportResumeId(job.id);
                      setConfig(job.config);
                      setTab("create");
                    }}
                  >
                    Reselect files to continue
                  </button>
                </section>
              )}
              {native &&
                job.config.kind === "website" &&
                job.phase === "download" &&
                ["paused", "failed"].includes(job.status) &&
                job.entries.some((e) => e.selected) && (
                  <section
                    className="panel rebuild-panel"
                    aria-labelledby="available-title"
                  >
                    <div>
                      <h2 id="available-title">
                        Finish with what’s downloaded
                      </h2>
                      <p>
                        {job.asset_summary?.downloaded ?? 0} assets saved ·{" "}
                        {job.asset_summary?.missing ?? 0} not downloaded. Build
                        all selected pages without more requests. Missing
                        images, styles, or fonts will be listed in the ZIM’s
                        capture report.
                      </p>
                    </div>
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() =>
                        void action(async () => {
                          await buildAvailable(job.id);
                          setToast(
                            "Building with available files. Missing assets will be listed in the ZIM.",
                          );
                        })
                      }
                    >
                      <Archive size={17} /> Build with available files
                    </button>
                  </section>
                )}
              {(native || browserOnly) && job.status === "failed" && (
                <section
                  className="panel rebuild-panel"
                  aria-labelledby="delete-attempt-title"
                >
                  <div>
                    <h2 id="delete-attempt-title">
                      Remove this failed attempt
                    </h2>
                    <p>
                      Delete its activity entry and internal cached files. ZIM
                      copies you exported stay unchanged.
                    </p>
                  </div>
                  <button
                    className="button secondary"
                    disabled={busy}
                    onClick={() => setConfirmDelete(true)}
                  >
                    <Trash2 size={17} /> Delete failed attempt
                  </button>
                </section>
              )}
              {job.retry_at &&
                job.retry_at > Date.now() / 1000 &&
                job.status !== "completed" && (
                  <div className="alert warning">
                    <Clock3 size={18} />
                    Server requests may resume after{" "}
                    {new Date(job.retry_at * 1000).toLocaleTimeString()}.
                  </div>
                )}
              {job.status === "review" &&
                job.import &&
                job.logs
                  .filter((entry) =>
                    entry.message.startsWith("WARC import omitted"),
                  )
                  .map((entry, index) => (
                    <p className="alert warning" key={index}>
                      {entry.message}
                    </p>
                  ))}
              {job.status === "completed" && (
                <div className="completed-banner">
                  <span>
                    <CheckCheck size={28} />
                  </span>
                  <div>
                    <h2>Your collection is ready to go.</h2>
                    <p>
                      {bytes(job.output_bytes)} ·{" "}
                      {job.entries.filter((e) => e.selected).length} items ·
                      Contents & search included
                    </p>
                  </div>
                  <div className="archive-actions">
                    {native && (
                      <button
                        className="button primary"
                        disabled={busy}
                        onClick={() => openArchive(job.id)}
                      >
                        <ExternalLink size={17} /> Open ZIM
                      </button>
                    )}
                    <button
                      className={`button ${native ? "secondary" : "primary"}`}
                      disabled={busy}
                      onClick={() =>
                        void action(async () =>
                          setToast(await api.export(job.id)),
                        )
                      }
                    >
                      <ArrowDownToLine size={17} />
                      Save ZIM file
                    </button>
                    {native && (
                      <p>Choose Downloads or another folder when saving.</p>
                    )}
                  </div>
                </div>
              )}
              <CaptureReport job={job} />
              {native &&
                job.config.kind === "media" &&
                /^https?:\/\/(?:www\.|m\.|music\.)?(?:youtube\.com|youtu\.be)(?:\/|$)/i.test(
                  job.config.url,
                ) &&
                ["completed", "failed", "paused"].includes(job.status) &&
                job.phase !== "scan" &&
                job.entries.some((e) => e.selected) && (
                  <section
                    className="panel rebuild-panel"
                    aria-labelledby="repair-audio-title"
                  >
                    <div>
                      <h2 id="repair-audio-title">Wrong audio language?</h2>
                      <p>
                        Reselect YouTube’s original audio and rebuild this
                        collection. Matching saved video files are reused; audio
                        may download again. Exported ZIM copies stay unchanged.
                      </p>
                    </div>
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() =>
                        void action(async () => {
                          await repairYouTubeAudio(job.id);
                          setToast(
                            "Repairing original audio and rebuilding the ZIM",
                          );
                        })
                      }
                    >
                      <RefreshCw size={17} /> Repair original audio
                    </button>
                  </section>
                )}
              {job.status === "completed" && templates.length > 0 && (
                <section
                  className="panel rebuild-panel"
                  aria-labelledby="rebuild-title"
                >
                  <div>
                    <h2 id="rebuild-title">
                      Rebuild with a different template
                    </h2>
                    <p>
                      {browserOnly
                        ? "Reuse the downloaded files and update the archive's appearance. Copies you already saved stay unchanged."
                        : "Reuse the downloaded files. Screening Room may download missing thumbnails. Copies you already saved stay unchanged."}
                    </p>
                    {!browserOnly && (
                      <p>
                        Rebuild uses Zstandard level {settings?.zstd_level ?? 3}
                        . Change it under Settings → Archive compression.
                      </p>
                    )}
                  </div>
                  <div className="rebuild-controls">
                    <label htmlFor="rebuild-template">Archive template</label>
                    <select
                      id="rebuild-template"
                      value={
                        rebuildTemplate ??
                        settings?.archive_template ??
                        job.archive_template
                      }
                      disabled={busy}
                      onChange={(event) =>
                        setRebuildTemplate(
                          event.target.value as ArchiveTemplate,
                        )
                      }
                    >
                      {templates.map((template) => (
                        <option key={template.id} value={template.id}>
                          {template.name}
                        </option>
                      ))}
                    </select>
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() =>
                        void action(async () => {
                          await api.rebuild(
                            job.id,
                            rebuildTemplate ??
                              settings?.archive_template ??
                              job.archive_template,
                          );
                          setToast("Rebuilding from downloaded files");
                        })
                      }
                    >
                      <RefreshCw size={17} /> Rebuild ZIM
                    </button>
                  </div>
                </section>
              )}
              {job.status === "completed" &&
                !job.import &&
                (!browserOnly || job.config.kind === "website") && (
                  <section
                    className="panel rebuild-panel"
                    aria-labelledby="refresh-title"
                  >
                    <div>
                      <h2 id="refresh-title">Refresh this collection</h2>
                      <p>
                        Rescan the source and review what’s new, changed, or no
                        longer found. Pages are compared now; assets are
                        revalidated during download. Saved YouTube tracks are
                        reused. The existing ZIM stays intact until its
                        replacement is ready.
                      </p>
                    </div>
                    <button
                      className="button secondary"
                      disabled={busy}
                      onClick={() =>
                        void action(async () => {
                          await api.refresh(job.id);
                          setToast(
                            "Rescanning the collection. Review the changes before downloading.",
                          );
                        })
                      }
                    >
                      <RefreshCw size={17} /> Check for changes
                    </button>
                    {native &&
                      job.config.kind === "website" &&
                      !job.config.capture?.render_javascript && (
                        <button
                          className="button secondary"
                          disabled={
                            busy || !!job.config.capture?.max_download_mb
                          }
                          onClick={() =>
                            void action(async () => {
                              await refreshJavascript(job.id);
                              setToast(
                                "Rescanning with JavaScript. Review the rendered pages before building; your existing ZIM is preserved.",
                              );
                            })
                          }
                        >
                          <Globe2 size={17} /> Rescan with JavaScript
                        </button>
                      )}
                    {native &&
                      job.config.kind === "website" &&
                      !job.config.capture?.render_javascript &&
                      !!job.config.capture?.max_download_mb && (
                        <p className="settings-note">
                          To render JavaScript, start a new crawl with the
                          download budget cleared. Browser resource traffic
                          cannot be limited exactly.
                        </p>
                      )}
                  </section>
                )}
              {job.refresh?.compared && (
                <section
                  className="panel"
                  aria-labelledby="refresh-changes-title"
                >
                  <h2 id="refresh-changes-title">
                    Changes since the previous capture
                  </h2>
                  <p>
                    {["new", "changed", "unchanged", "unavailable", "not_seen"]
                      .map(
                        (kind) =>
                          `${job.refresh!.changes.filter((change) => change.kind === kind).length} ${kind === "not_seen" ? "not seen" : kind}`,
                      )
                      .join(" · ")}
                  </p>
                  {!job.refresh.complete && (
                    <p className="alert warning">
                      This scan was incomplete. Items not seen may still exist;
                      don’t treat them as confirmed removals.
                    </p>
                  )}
                  <details>
                    <summary>View changes</summary>
                    <ul>
                      {job.refresh.changes
                        .filter((change) => change.kind !== "unchanged")
                        .map((change) => (
                          <li key={change.url}>
                            <strong>
                              {change.kind === "not_seen"
                                ? "Not seen in this scan"
                                : change.kind}
                            </strong>
                            : {change.title}
                            <p className="source-address">{change.url}</p>
                          </li>
                        ))}
                    </ul>
                  </details>
                </section>
              )}
              <div className="stat-grid">
                <Stat
                  title="Discovered"
                  value={`${job.entries.length} items`}
                  icon={<Search size={20} />}
                />
                <Stat
                  title={
                    job.status === "review"
                      ? "Selected HTML / known media"
                      : "Saved so far"
                  }
                  value={bytes(job.status === "review" ? knownSize : job.bytes)}
                  icon={<ArrowDown size={20} />}
                />
                <Stat
                  title="Page request delay"
                  value={`${job.config.delay_ms / 1000}s minimum`}
                  icon={<ShieldCheck size={20} />}
                />
              </div>
              {job.javascript_needed &&
                ["review", "failed", "paused"].includes(job.status) && (
                  <section className="alert warning" role="status">
                    <strong>
                      This site needs JavaScript to discover more content.
                    </strong>
                    <p>
                      The static scan is incomplete.{" "}
                      {native
                        ? "Enable JavaScript and rescan before choosing what to download."
                        : "Use an installed app or import saved HTML to capture dynamic content."}
                    </p>
                    {native && !job.import && (
                      <button
                        className="button secondary"
                        disabled={busy || !!job.config.capture?.max_download_mb}
                        onClick={() =>
                          void action(async () => {
                            await api.editRetry(job.id, {
                              ...job.config,
                              capture: {
                                ...captureOptions(job.config),
                                render_javascript: true,
                                auto_render_javascript: true,
                              },
                            });
                            setToast(
                              "JavaScript enabled. Rescanning on this device; review the results before downloading.",
                            );
                          })
                        }
                      >
                        Enable JavaScript and rescan
                      </button>
                    )}
                    {!!job.config.capture?.max_download_mb && (
                      <p>
                        Clear the download budget in Edit crawl to allow browser
                        rendering.
                      </p>
                    )}
                  </section>
                )}
              <section className="card preview-card">
                <div className="section-header">
                  <div>
                    <h2>
                      {job.status === "review"
                        ? "Make it your collection"
                        : "Collection contents"}
                    </h2>
                    <p>
                      {job.status === "scanning"
                        ? job.config.capture?.render_javascript
                          ? "Rendering pages on this device. Scripts and page resources load during the scan; review is always required. You can pause at any time."
                          : native &&
                              (job.config.capture?.auto_render_javascript ??
                                true)
                            ? "Scanning pages. Dynamic content is rendered automatically on this device; rendered captures require review."
                            : job.auto_start_item_limit !== null
                              ? `Discovering items. Complete, error-free scans of up to ${job.auto_start_item_limit} items will start automatically.`
                              : "Discovering items. Media and page assets wait for your confirmation."
                        : job.status === "review"
                          ? "Choose what to keep. Asset sizes and some media sizes are not known yet."
                          : "Completed items are checkpointed and reused when you resume."}
                    </p>
                  </div>
                  <div className="job-actions">
                    {["failed", "paused", "review"].includes(job.status) &&
                      !job.import && (
                        <button
                          className="button secondary"
                          disabled={busy}
                          onClick={() => editCrawl(job)}
                        >
                          <Pencil size={16} /> Edit crawl
                        </button>
                      )}
                    {activeStates.includes(job.status) ? (
                      <button
                        className="button secondary"
                        disabled={busy || job.status === "pausing"}
                        onClick={() => void action(() => api.pause(job.id))}
                      >
                        <Pause size={16} />
                        Pause
                      </button>
                    ) : null}
                    {["review", "failed"].includes(job.status) &&
                      job.entries.some((e) => e.state !== "scan_failed") && (
                        <button
                          className="button primary"
                          disabled={busy || !selection.size}
                          onClick={() => setConfirm(true)}
                        >
                          Review & build
                          <ArrowRight size={16} />
                        </button>
                      )}
                  </div>
                </div>
                <div className="table-toolbar">
                  <label className="check-label">
                    <input
                      aria-label="Select all items"
                      type="checkbox"
                      disabled={!["review", "failed"].includes(job.status)}
                      checked={
                        job.entries.filter((e) => e.state !== "scan_failed")
                          .length > 0 &&
                        job.entries
                          .filter((e) => e.state !== "scan_failed")
                          .every((e) => selection.has(e.id))
                      }
                      onChange={(e) =>
                        setSelection(
                          new Set(
                            e.target.checked
                              ? job.entries
                                  .filter((e) => e.state !== "scan_failed")
                                  .map((e) => e.id)
                              : [],
                          ),
                        )
                      }
                    />
                    {selection.size} selected
                  </label>
                  <div className="filter-input">
                    <Search size={15} />
                    <input
                      aria-label="Filter collection"
                      placeholder="Find an item…"
                      value={filter}
                      onChange={(e) => {
                        setFilter(e.target.value);
                        setCollectionPage(0);
                      }}
                    />
                  </div>
                </div>
                <div className="collection-items-container">
                  <div className="collection-columns" aria-hidden="true">
                    <span />
                    <span>ITEM</span>
                    <span>DEPTH / DURATION</span>
                    <span>KNOWN SIZE</span>
                    <span>STATUS</span>
                  </div>
                  <ul
                    className="collection-items"
                    aria-label="Collection items"
                  >
                    {visibleEntries.map((entry) => (
                      <EntryRow
                        key={entry.id}
                        entry={entry}
                        media={job.config.kind === "media"}
                        selected={selection.has(entry.id)}
                        editable={["review", "failed"].includes(job.status)}
                        onToggle={() =>
                          setSelection((previous) => {
                            const next = new Set(previous);
                            next.has(entry.id)
                              ? next.delete(entry.id)
                              : next.add(entry.id);
                            return next;
                          })
                        }
                      />
                    ))}
                  </ul>
                  {pageCount > 1 && (
                    <div
                      className="table-toolbar"
                      aria-label="Collection pages"
                    >
                      <button
                        className="button secondary"
                        disabled={currentPage === 0}
                        onClick={() => setCollectionPage(currentPage - 1)}
                      >
                        Previous
                      </button>
                      <span aria-live="polite">
                        Page {currentPage + 1} of {pageCount}
                      </span>
                      <button
                        className="button secondary"
                        disabled={currentPage + 1 >= pageCount}
                        onClick={() => setCollectionPage(currentPage + 1)}
                      >
                        Next
                      </button>
                    </div>
                  )}
                  {job.entries.length === 0 && (
                    <div className="empty-large">
                      <LoaderCircle className="spin" size={28} />
                      <p>Looking for your first items…</p>
                    </div>
                  )}
                </div>
              </section>
              <section className="card activity-card">
                <div className="section-header">
                  <h2>
                    <Radio size={17} />
                    Activity log
                  </h2>
                  <span className="muted">Saved with this job</span>
                </div>
                <div className="log-lines" role="log" aria-label="Job activity">
                  {[...job.logs]
                    .reverse()
                    .slice(0, 80)
                    .map((log, index) => (
                      <div
                        key={`${log.time}-${index}`}
                        className={`log-line ${log.level}`}
                      >
                        <time>
                          {new Date(log.time * 1000).toLocaleTimeString()}
                        </time>
                        <span className="log-level">{log.level}</span>
                        <span>{log.message}</span>
                      </div>
                    ))}
                </div>
              </section>
            </>
          )}

          {tab === "settings" && (
            <>
              <div className="page-heading compact">
                <div className="eyebrow">SET UP YOUR WORKSPACE</div>
                <h1>Built to run right here.</h1>
                <p>
                  {browserOnly
                    ? "Crawling, checkpoints, search, and ZIM building run in this browser profile. No build server or native helper is used."
                    : "Website crawling and ZIM creation run locally, on every platform."}
                </p>
              </div>
              <section className="card settings-card appearance-preferences">
                <form
                  className="preferences-form"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void saveSettings("appearance");
                  }}
                >
                  <ArchiveTemplates
                    value={settingsDraft.archive_template}
                    disabled={!settings || busy}
                    onChange={(archive_template) =>
                      setSettingsDraft((previous) => ({
                        ...previous,
                        archive_template,
                      }))
                    }
                  />
                  <div className="preferences-actions">
                    <button
                      type="submit"
                      className="button primary"
                      disabled={!settings || busy}
                    >
                      {busy ? (
                        <LoaderCircle className="spin" size={16} />
                      ) : (
                        <Check size={16} />
                      )}
                      Save archive style
                    </button>
                  </div>
                </form>
              </section>
              <section className="card settings-card compression-preferences">
                <h2>Archive compression</h2>
                {browserOnly ? (
                  <p>
                    This browser build writes uncompressed ZIMs. Zstandard
                    compression is available in the installed apps.
                  </p>
                ) : (
                  <form
                    className="preferences-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void saveSettings("compression");
                    }}
                  >
                    <div className="setting-row">
                      <div>
                        <label htmlFor="zstd-level">
                          Zstandard compression level
                        </label>
                        <p>
                          1 is fastest; 22 prioritizes smaller files. Higher
                          levels use more time and memory. The default is 3.
                        </p>
                      </div>
                      <NumberInput
                        id="zstd-level"
                        className="threshold-input"
                        min={1}
                        max={22}
                        step={1}
                        required
                        disabled={!settings || busy}
                        value={settingsDraft.zstd_level}
                        onValueChange={(value) =>
                          setSettingsDraft((previous) => ({
                            ...previous,
                            zstd_level: value,
                          }))
                        }
                      />
                    </div>
                    <p className="settings-note">
                      Applies to new jobs and explicit rebuilds. Existing jobs
                      keep their saved level when resumed. Video, audio, and
                      images keep their existing compression.
                    </p>
                    <div className="preferences-actions">
                      <button
                        type="submit"
                        className="button primary"
                        disabled={!settings || busy}
                      >
                        <Check size={16} /> Save compression preference
                      </button>
                    </div>
                  </form>
                )}
              </section>
              {!browserOnly && (
                <section className="card settings-card">
                  <h2>CDN downloads</h2>
                  <form
                    className="preferences-form"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void saveSettings("cdn");
                    }}
                  >
                    <div className="setting-row">
                      <div>
                        <label htmlFor="cdn-parallel-requests">
                          Parallel CDN requests
                        </label>
                        <p>
                          Download 1–6 linked images, fonts, or stylesheets at a
                          time from a recognized CDN. Default: 3. Page delays,
                          download budgets, and server cooldowns still apply.
                        </p>
                      </div>
                      <NumberInput
                        id="cdn-parallel-requests"
                        className="threshold-input"
                        min={1}
                        max={6}
                        step={1}
                        required
                        disabled={!settings || busy}
                        value={settingsDraft.cdn_parallel_requests}
                        onValueChange={(value) =>
                          setSettingsDraft((previous) => ({
                            ...previous,
                            cdn_parallel_requests: value,
                          }))
                        }
                      />
                    </div>
                    <p className="settings-note">
                      Applies when downloads start or resume. Choose 1 to fetch
                      one file at a time.
                    </p>
                    <div className="preferences-actions">
                      <button
                        type="submit"
                        className="button primary"
                        disabled={!settings || busy}
                      >
                        <Check size={16} /> Save CDN preference
                      </button>
                    </div>
                  </form>
                </section>
              )}
              <section className="card settings-card download-preferences">
                <h2>Download confirmation</h2>
                <form
                  className="preferences-form"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void saveSettings();
                  }}
                >
                  <div className="setting-row">
                    <div>
                      <label htmlFor="auto-start">
                        Automatically start small jobs
                      </label>
                      <p>
                        After a complete scan, download every discovered item
                        and build the ZIM without a confirmation step.
                      </p>
                    </div>
                    <input
                      id="auto-start"
                      type="checkbox"
                      checked={settingsDraft.auto_start_small_jobs}
                      disabled={!settings || busy}
                      onChange={(e) =>
                        setSettingsDraft((previous) => ({
                          ...previous,
                          auto_start_small_jobs: e.target.checked,
                        }))
                      }
                    />
                  </div>
                  <div className="setting-row">
                    <div>
                      <label htmlFor="auto-start-limit">
                        Maximum items for automatic download
                      </label>
                      <p>
                        Includes the threshold itself. Pages and media entries
                        count as items; linked assets are additional.
                      </p>
                    </div>
                    <NumberInput
                      id="auto-start-limit"
                      className="threshold-input"
                      min={1}
                      max={5000}
                      step={1}
                      required
                      value={settingsDraft.auto_start_item_limit}
                      disabled={
                        !settings ||
                        busy ||
                        !settingsDraft.auto_start_small_jobs
                      }
                      onValueChange={(value) =>
                        setSettingsDraft((previous) => ({
                          ...previous,
                          auto_start_item_limit: value,
                        }))
                      }
                    />
                  </div>
                  <p className="settings-note">
                    This is an item-count threshold, not a download-size or
                    memory limit. Media sizes may be unknown, and a small
                    collection can still be large. Incomplete scans and scans
                    with failed items always stop for review. Changes apply to
                    new jobs.
                  </p>
                  <div className="preferences-actions">
                    <button
                      type="submit"
                      className="button primary"
                      disabled={busy || !settings}
                    >
                      {busy ? (
                        <LoaderCircle className="spin" size={16} />
                      ) : (
                        <Check size={16} />
                      )}
                      Save download preferences
                    </button>
                  </div>
                </form>
              </section>
              <section className="card settings-card">
                <h2>On-device engine</h2>
                <div className="setting-row">
                  <div>
                    <strong>Website crawler</strong>
                    <p>
                      {browserOnly
                        ? "Local browser crawler · robots.txt · retries · persistent checkpoints"
                        : "Rust · robots.txt · retries · persistent checkpoints"}
                    </p>
                  </div>
                  <span className="capability-badge">Included</span>
                </div>
                <div className="setting-row">
                  <div>
                    <strong>ZIM writer</strong>
                    <p>
                      {browserOnly
                        ? "Streaming ZIM 6.3 writer · uncompressed clusters · bounded file buffers"
                        : "Native Rust writer · no external process required"}
                    </p>
                  </div>
                  <span className="capability-badge">Included</span>
                </div>
                <div className="setting-row">
                  <div>
                    <strong>YouTube extractor</strong>
                    <p>
                      {browserOnly
                        ? "Not yet ported. Use the native app for YouTube collections."
                        : "Native extraction for channels, playlists, searches, and videos"}
                    </p>
                  </div>
                  <span className="capability-badge">
                    {browserOnly ? "Native app" : "Included"}
                  </span>
                </div>
                <div className="setting-row">
                  <div>
                    <strong>Additional media sites · yt-dlp</strong>
                    <p>
                      {capabilities?.yt_dlp.available
                        ? capabilities.yt_dlp.detail
                        : browserOnly
                          ? "Not available inside the browser. Direct video/audio URLs are supported."
                          : "Optional on desktop. Not required for website crawling or ZIM creation."}
                    </p>
                  </div>
                  <span
                    className={`capability-badge ${!capabilities?.yt_dlp.available ? "optional-badge" : ""}`}
                  >
                    {capabilities?.yt_dlp.available
                      ? "Available"
                      : browserOnly
                        ? "Native app"
                        : "Optional"}
                  </span>
                </div>
                <div className="setting-row">
                  <div>
                    <strong>Archive search</strong>
                    <p>
                      Portable offline search is always included. Native Kiwix
                      {browserOnly
                        ? "indexes are available in native builds; browser builds use the archive's own search."
                        : "indexes can be enabled at build time."}
                    </p>
                  </div>
                  <Search size={19} />
                </div>
                <div className="setting-row">
                  <div>
                    <strong>Local storage</strong>
                    <code>{capabilities?.data_dir || "Connecting…"}</code>
                  </div>
                  <Archive size={19} />
                </div>
                <p className="settings-note">
                  {browserOnly
                    ? "Keep the workspace tab open while jobs run. Resume after reopening to recover saved progress. Browser storage has a quota and can be removed when you clear site data; export finished archives."
                    : "On phones, keep the app open while crawling. If the app is suspended or closed, saved checkpoints let you resume after reopening."}
                </p>
              </section>
            </>
          )}
        </main>
      </div>

      {confirmDelete && job && (
        <div
          className="modal-backdrop"
          onClick={() => {
            if (!busy) setConfirmDelete(false);
          }}
        >
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="delete-title"
            onClick={(e) => e.stopPropagation()}
          >
            <h2 id="delete-title">Delete failed attempt?</h2>
            <p>
              Remove “{job.config.title}” and all its internal downloads and
              cached files from this device. Exported ZIM copies stay unchanged.
              This cannot be undone.
            </p>
            <div className="modal-actions">
              <button
                className="button secondary"
                disabled={busy}
                onClick={() => setConfirmDelete(false)}
              >
                Keep attempt
              </button>
              <button
                className="button primary"
                disabled={busy}
                onClick={() => void deleteAttempt()}
              >
                <Trash2 size={17} /> Delete attempt
              </button>
            </div>
          </section>
        </div>
      )}
      {confirm && job && (
        <div className="modal-backdrop" onClick={() => setConfirm(false)}>
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="confirm-title"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              className="modal-close"
              aria-label="Close confirmation"
              onClick={() => setConfirm(false)}
            >
              <X size={20} />
            </button>
            <span className="modal-icon">
              <Archive size={28} />
            </span>
            <div className="eyebrow">YOUR COLLECTION, YOUR CALL</div>
            <h2 id="confirm-title">Ready to take it offline?</h2>
            <p>
              {job.import
                ? "We’ll build the search index and ZIM from your saved files on this device."
                : "We’ll download your selection, build the search index and contents, and package everything into a ZIM file on this device."}
            </p>
            <dl>
              <div>
                <dt>Selected items</dt>
                <dd>{selection.size}</dd>
              </div>
              <div>
                <dt>Known source size</dt>
                <dd>{bytes(knownSize)}</dd>
              </div>
              {!job.import && (
                <>
                  <div>
                    <dt>Maximum per file</dt>
                    <dd>{job.config.max_resource_mb} MB</dd>
                  </div>
                  <div>
                    <dt>Page request delay</dt>
                    <dd>{job.config.delay_ms / 1000} seconds</dd>
                  </div>
                </>
              )}
            </dl>
            {!job.import && (
              <div className="alert info">
                This is not a total download estimate. Assets and media with
                unknown sizes add to it. You can pause and resume at any time.
              </div>
            )}
            <div className="modal-actions">
              <button
                className="button secondary"
                onClick={() => setConfirm(false)}
              >
                Keep reviewing
              </button>
              <button
                className="button primary"
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await api.build(job.id, [...selection]);
                    setConfirm(false);
                  })
                }
              >
                {busy ? (
                  <LoaderCircle className="spin" size={16} />
                ) : (
                  <ArrowDownToLine size={17} />
                )}
                {job.import ? "Build ZIM" : "Download & build ZIM"}
              </button>
            </div>
          </section>
        </div>
      )}
      {showHelp && (
        <div className="modal-backdrop" onClick={() => setShowHelp(false)}>
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="help-title"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              className="modal-close"
              aria-label="Close guide"
              onClick={() => setShowHelp(false)}
            >
              <X size={20} />
            </button>
            <span className="modal-icon">
              <BookOpen size={28} />
            </span>
            <h2 id="help-title">Your web, a little more portable.</h2>
            <p>
              1. Paste a website, YouTube channel, playlist, search, or media
              link. Set a small scan limit to start.
            </p>
            <p>
              2. Scan website HTML or media metadata. Small, complete scans can
              start automatically using your download preferences; larger or
              incomplete scans stop for review.
            </p>
            <p>
              3. Confirm downloads that require review. Pause whenever you need
              to; resume from Activity.
            </p>
            <p>
              4. Save the finished ZIM and open it in Kiwix. Search is available
              from the collection’s home page.
            </p>
            <div className="alert info">
              JavaScript-only websites, logins, live streams, and protected
              media may not be capturable. Failed items remain visible with a
              reason.
            </div>
            <button
              className="button primary"
              onClick={() => setShowHelp(false)}
            >
              Let’s explore
              <ArrowRight size={16} />
            </button>
          </section>
        </div>
      )}
      {toast && (
        <div className="toast" role="status">
          <Check size={18} />
          <span>{toast}</span>
          <button
            aria-label="Dismiss notification"
            onClick={() => setToast("")}
          >
            <X size={16} />
          </button>
        </div>
      )}
    </div>
  );
}

function titleFromUrl(value: string) {
  try {
    return new URL(normalizeSourceUrl(value)).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
}
function Status({ status }: { status: string }) {
  return (
    <span className={`status-pill ${status}`}>
      <span />
      {labels[status] || status}
    </span>
  );
}
function Stat({
  title,
  value,
  icon,
}: {
  title: string;
  value: string;
  icon: React.ReactNode;
}) {
  return (
    <div className="stat-card">
      <div>
        <p>{title}</p>
        <strong>{value}</strong>
      </div>
      <span>{icon}</span>
    </div>
  );
}
function JobRow({
  item,
  onClick,
  onOpen,
}: {
  item: Job;
  onClick: () => void;
  onOpen?: () => void;
}) {
  return (
    <div className="job-row-container">
      <button
        className="job-row"
        onClick={onOpen || onClick}
        aria-label={onOpen ? `Open ${item.config.title} ZIM` : undefined}
      >
        <span className={`job-icon ${item.config.kind}`}>
          {item.config.kind === "website" ? (
            <Globe2 size={23} />
          ) : (
            <Video size={23} />
          )}
        </span>
        <div className="job-row-title">
          <strong>{item.config.title}</strong>
          <small>
            {item.entry_count ?? item.entries.length} items ·{" "}
            {new Date(item.created_at * 1000).toLocaleDateString(undefined, {
              month: "short",
              day: "numeric",
            })}
          </small>
          {[
            "scanning",
            "downloading",
            "packaging",
            "pausing",
            "paused",
          ].includes(item.status) && <JobProgress job={item} compact />}
        </div>
        <Status status={item.status} />
        {onOpen ? <ExternalLink size={18} /> : <ChevronRight size={18} />}
      </button>
      {onOpen && (
        <button
          className="job-details"
          onClick={onClick}
          aria-label={`Details for ${item.config.title}`}
        >
          Details
        </button>
      )}
    </div>
  );
}
function EntryRow({
  entry,
  selected,
  onToggle,
  editable,
  media,
}: {
  entry: Entry;
  selected: boolean;
  onToggle: () => void;
  editable: boolean;
  media: boolean;
}) {
  media ||= !!entry.embedded_in?.length;
  return (
    <li className="collection-item">
      <label className="entry-selector">
        <input
          type="checkbox"
          aria-label={`Select ${entry.title}`}
          disabled={!editable || entry.state === "scan_failed"}
          checked={selected}
          onChange={onToggle}
        />
      </label>
      <div className="entry-title">
        {media ? <Video size={17} /> : <FileText size={17} />}
        <span>
          <strong>{entry.title}</strong>
          <small className="entry-url" title={entry.url}>
            {entry.url}
          </small>
          {!!entry.embedded_in?.length && (
            <small>
              Embedded video · referenced by {entry.embedded_in.length} page(s)
            </small>
          )}
          {entry.rendered && (
            <small>
              {entry.rendered.settled
                ? "Browser snapshot"
                : "Browser snapshot may be incomplete"}
            </small>
          )}
          {entry.rendered?.warnings.slice(0, 2).map((warning, index) => (
            <small className="entry-error" key={index}>
              {warning}
            </small>
          ))}
          {entry.error && <small className="entry-error">{entry.error}</small>}
        </span>
      </div>
      <div className="entry-metadata">
        <span className="entry-metric">
          <span>{media ? "Duration" : "Depth"}</span>{" "}
          {media ? duration(entry.duration) : entry.depth}
        </span>
        <span className="entry-metric">
          <span>Size</span> {bytes(entry.size)}
        </span>
        <span className={`entry-state ${entry.state}`}>
          {entry.state === "done" ? (
            <Check size={13} />
          ) : entry.state.includes("failed") ? (
            <X size={13} />
          ) : (
            <Clock3 size={13} />
          )}
          {entry.state === "pending"
            ? "Discovered"
            : entry.state === "done"
              ? "Saved"
              : "Failed"}
        </span>
      </div>
    </li>
  );
}
