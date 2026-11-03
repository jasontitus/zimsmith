import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Link2, X } from "lucide-react";

export interface SharedLink {
  id: string;
  url: string | null;
  title: string;
  error: string | null;
}

export async function dismissSharedLink(id: string) {
  await invoke<void>("dismiss_shared_link", { id });
  window.dispatchEvent(new Event("zimsmith-shared-links-changed"));
}

export function SharedLinks({
  selected,
  onReview,
  onDismiss,
  onScan,
  busy,
}: {
  selected: string | null;
  onReview: (link: SharedLink, automatic: boolean) => void;
  onDismiss: (id: string) => void;
  onScan: () => void;
  busy: boolean;
}) {
  const [links, setLinks] = useState<SharedLink[]>([]);
  const [error, setError] = useState("");
  const seen = useRef(new Set<string>());
  const reading = useRef(false);
  const poll = useCallback(async () => {
    if (document.hidden || reading.current) return;
    reading.current = true;
    try {
      const items = await invoke<SharedLink[]>("get_shared_links");
      setLinks(items);
      setError("");
      for (const item of items) {
        if (!seen.current.has(item.id)) {
          seen.current.add(item.id);
          if (item.url) onReview(item, true);
          break;
        }
      }
    } catch (e) {
      setError(
        `Could not read shared links: ${e instanceof Error ? e.message : String(e)}`,
      );
    } finally {
      reading.current = false;
    }
  }, [onReview]);
  useEffect(() => {
    void poll();
    const timer = setInterval(() => void poll(), 3000);
    document.addEventListener("visibilitychange", poll);
    window.addEventListener("zimsmith-shared-links-changed", poll);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", poll);
      window.removeEventListener("zimsmith-shared-links-changed", poll);
    };
  }, [poll]);
  if (!links.length && !error) return null;
  return (
    <section className="panel shared-links" aria-label="Shared links">
      <div>
        <h2>
          <Link2 size={19} /> Shared links
        </h2>
        <p>
          Your link is ready to scan. Tap Scan shared link to begin, or adjust
          the crawl settings below first.
        </p>
      </div>
      {error && <p role="alert">{error}</p>}
      {links.map((link) => (
        <div className="shared-link" key={link.id}>
          <div>
            <strong>{link.title || "Shared website"}</strong>
            <p>{link.url || link.error}</p>
          </div>
          {link.url && (
            <button
              className={
                selected === link.id ? "button primary" : "button secondary"
              }
              disabled={busy}
              onClick={() =>
                selected === link.id ? onScan() : onReview(link, false)
              }
            >
              {selected === link.id ? "Scan shared link" : "Set up crawl"}
            </button>
          )}
          <button
            className="icon-button"
            aria-label={`Dismiss ${link.title || "shared website"}`}
            onClick={() => {
              void dismissSharedLink(link.id)
                .then(() => {
                  setLinks((items) =>
                    items.filter((item) => item.id !== link.id),
                  );
                  onDismiss(link.id);
                })
                .catch((e) => setError(String(e)));
            }}
          >
            <X size={19} />
          </button>
        </div>
      ))}
    </section>
  );
}
