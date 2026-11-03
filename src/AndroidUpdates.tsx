import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Release = {
  version: string;
  version_code: number;
  url: string;
  size_bytes: number;
  notes: string;
};
type UpdateStatus = {
  enabled: boolean;
  automatic: boolean;
  feed_url: string | null;
  current_version: string;
  last_checked: number | null;
  retry_at: number | null;
  error: string | null;
  release: Release | null;
  dismissed_version: number | null;
};

export function AndroidUpdates({
  settingsVisible,
  active,
}: {
  settingsVisible: boolean;
  active: boolean;
}) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [feed, setFeed] = useState("");
  const [automatic, setAutomatic] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  useEffect(() => {
    let disposed = false;
    let enabled = false;
    let checking = false;
    const check = async () => {
      if (!enabled || checking || document.hidden) return;
      checking = true;
      try {
        const next = await invoke<UpdateStatus>("check_app_updates", {
          manual: false,
        });
        if (!disposed) setStatus(next);
      } catch (e) {
        if (!disposed) setError(String(e));
      } finally {
        checking = false;
      }
    };
    void invoke<UpdateStatus>("get_update_status")
      .then((next) => {
        if (disposed) return;
        setStatus(next);
        setFeed(next.feed_url ?? "");
        setAutomatic(next.automatic);
        enabled = next.enabled;
        void check();
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      });
    const timer = setInterval(() => void check(), 60 * 60 * 1000);
    const visible = () => void check();
    document.addEventListener("visibilitychange", visible);
    return () => {
      disposed = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", visible);
    };
  }, []);

  async function action(command: string, args?: Record<string, unknown>) {
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const next = await invoke<UpdateStatus | undefined>(command, args);
      if (next) setStatus(next);
      if (command === "download_app_update")
        setMessage(
          "Opened your browser. Download the APK and approve Android’s update prompt.",
        );
      if (command === "configure_app_updates")
        setMessage("Update preferences saved.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (!status?.enabled)
    return error && settingsVisible ? (
      <div className="alert warning">
        Could not load update settings: {error}
      </div>
    ) : null;
  const release = status.release;
  const offered =
    release &&
    (settingsVisible || status.dismissed_version !== release.version_code);
  return (
    <>
      {offered && (
        <section className="card app-update" aria-label="Available app update">
          <h2>Zimsmith {release.version} is available</h2>
          <p>
            {(release.size_bytes / 1024 / 1024).toFixed(1)} MB ·{" "}
            {new URL(release.url).hostname}
          </p>
          {release.notes && <p className="update-notes">{release.notes}</p>}
          <div className="update-actions">
            <button
              className="button primary"
              disabled={busy || active}
              onClick={() => void action("download_app_update")}
            >
              Download update
            </button>
            {!settingsVisible && (
              <button
                className="button secondary"
                disabled={busy}
                onClick={() => void action("dismiss_app_update")}
              >
                Later
              </button>
            )}
          </div>
          {active && (
            <p>Pause running jobs before downloading an app update.</p>
          )}
          <p className="settings-note">
            The download opens in your browser. Android will ask you to confirm
            installation. Your saved jobs stay on this device.
          </p>
        </section>
      )}
      {(error || message) && (
        <div
          className={`alert ${error ? "warning" : "success"}`}
          role={error ? "alert" : "status"}
        >
          {error || message}
        </div>
      )}
      {settingsVisible && (
        <section
          className="card settings-card app-update"
          aria-label="App updates"
        >
          <h2>App updates</h2>
          <p>
            Direct Android build · installed version {status.current_version}
          </p>
          <p>
            Checks run at most once a day. Only release information is
            requested; your crawl URLs and archives are not sent.
          </p>
          <label className="update-toggle">
            <input
              type="checkbox"
              checked={automatic}
              onChange={(e) => setAutomatic(e.target.checked)}
            />{" "}
            Check automatically
          </label>
          <details>
            <summary>Advanced: update source</summary>
            <label>
              HTTPS release feed
              <input
                type="url"
                value={feed}
                onChange={(e) => setFeed(e.target.value)}
                placeholder="https://your-domain/updates/android.json"
              />
            </label>
            <p className="settings-note">
              Use a release feed from the app publisher. Leave empty to use this
              build’s default. Changing the source clears the cached update.
            </p>
          </details>
          <div className="update-actions">
            <button
              className="button secondary"
              disabled={busy}
              onClick={() =>
                void action("configure_app_updates", {
                  automatic,
                  feedUrl: feed,
                })
              }
            >
              Save update settings
            </button>
            <button
              className="button secondary"
              disabled={
                busy ||
                !status.feed_url ||
                !!(status.retry_at && status.retry_at * 1000 > Date.now())
              }
              onClick={() => void action("check_app_updates", { manual: true })}
            >
              {busy ? "Working…" : "Check for updates"}
            </button>
          </div>
          {!status.feed_url && (
            <p>
              No update server is configured yet. Set the release feed above
              once hosting is ready.
            </p>
          )}
          {status.last_checked && (
            <p>
              Last checked:{" "}
              {new Date(status.last_checked * 1000).toLocaleString()}
              {!status.error && !release ? " · You’re up to date." : ""}
            </p>
          )}
          {status.error && <p role="status">{status.error}</p>}
          {status.retry_at && status.retry_at * 1000 > Date.now() && (
            <p>
              Next check available after{" "}
              {new Date(status.retry_at * 1000).toLocaleString()}.
            </p>
          )}
        </section>
      )}
    </>
  );
}
