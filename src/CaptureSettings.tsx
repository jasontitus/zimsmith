import type { CaptureOptions, Config } from "./types";
import { captureOptions } from "./capture";
import { browserOnly } from "./browser/client";
export function CaptureSettings({
  config,
  onChange,
}: {
  config: Config;
  onChange: (value: CaptureOptions) => void;
}) {
  const c = captureOptions(config);
  const direct = /\.(mp4|webm|mov|mp3|m4a|ogg|wav)(?:$|\?)/i.test(config.url);
  const update = <K extends keyof CaptureOptions>(
    key: K,
    value: CaptureOptions[K],
  ) => onChange({ ...c, [key]: value });
  return (
    <section
      className="capture-settings"
      aria-labelledby="capture-settings-title"
    >
      <h3 id="capture-settings-title">Scope & storage</h3>
      <div className="form-grid">
        <div>
          <label htmlFor="download-budget">Download budget (MB)</label>
          <input
            id="download-budget"
            type="number"
            min={1}
            max={1048576}
            placeholder="Unlimited"
            value={c.max_download_mb ?? ""}
            onChange={(e) =>
              update("max_download_mb", e.target.value ? +e.target.value : null)
            }
          />
          <p className="settings-note">
            Stops new downloads at this budget. Resumes keep the counter;
            refreshing starts a new budget. Leave disk space for the ZIM too.
          </p>
        </div>
        {!direct &&
          (config.kind === "media" || (!browserOnly && c.embedded_videos)) && (
            <div>
              <label htmlFor="video-quality">Video quality</label>
              <select
                id="video-quality"
                value={c.video_quality}
                onChange={(e) =>
                  update(
                    "video_quality",
                    e.target.value as CaptureOptions["video_quality"],
                  )
                }
              >
                <option value="small">Small · up to 360p</option>
                <option value="balanced">Balanced · up to 720p</option>
                <option value="high">High · up to 1080p</option>
              </select>
              <p className="settings-note">
                YouTube uses the best compatible stream within this limit.
                Direct file URLs keep their original quality.
              </p>
            </div>
          )}
      </div>
      {config.kind === "website" && (
        <>
          {!browserOnly && (
            <>
              {(c.render_javascript || c.auto_render_javascript) && (
                <>
                  <label htmlFor="render-timeout">
                    Rendering timeout per page (seconds)
                  </label>
                  <input
                    id="render-timeout"
                    type="number"
                    min={10}
                    max={180}
                    value={c.render_timeout_secs}
                    onChange={(e) =>
                      update("render_timeout_secs", +e.target.value)
                    }
                  />
                  <p className="settings-note">
                    Up to 8 MB of rendered HTML per page. Browser traffic cannot
                    be limited exactly, so clear the download budget to use this
                    mode. Forms and checkout remain online-only.
                  </p>
                </>
              )}
            </>
          )}
          <div className="form-grid">
            <div>
              <label htmlFor="include-urls">Include page URLs</label>
              <textarea
                id="include-urls"
                rows={3}
                placeholder="/articles/*"
                value={c.include_urls.join("\n")}
                onChange={(e) =>
                  update("include_urls", e.target.value.split("\n"))
                }
              />
            </div>
            <div>
              <label htmlFor="exclude-urls">Exclude URLs</label>
              <textarea
                id="exclude-urls"
                rows={3}
                placeholder="*/login*"
                value={c.exclude_urls.join("\n")}
                onChange={(e) =>
                  update("exclude_urls", e.target.value.split("\n"))
                }
              />
            </div>
          </div>
          <p className="settings-note">
            One rule per line. * matches any text; ? matches one character.
            Rules starting with / match paths, others match full URLs. Includes
            apply to pages; excludes apply to pages and assets. The starting
            page is scanned for discovery even if it isn’t included.
          </p>
          {!browserOnly && (
            <>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={c.embedded_videos}
                  onChange={(e) => update("embedded_videos", e.target.checked)}
                />
                Download embedded YouTube videos
              </label>
              <p className="settings-note">
                Videos appear separately in review. Review is always required
                when embeds are found; their sizes may be unknown. Uses the
                video quality, subtitle, file-size, and total-budget settings.
              </p>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={c.discover_sitemaps}
                  onChange={(e) =>
                    update("discover_sitemaps", e.target.checked)
                  }
                />
                Discover pages from sitemaps
              </label>
              <p className="settings-note">
                Checks up to 8 same-origin sitemaps, each at most 2 MB. Scope,
                depth, URL filters, robots rules, and download budgets still
                apply.
              </p>
            </>
          )}
          <div className="capture-switches">
            {(
              [
                ["images", "Images"],
                ["styles", "Stylesheets"],
                ["fonts", "Fonts"],
                ["media", "Linked video & audio"],
                ["documents", "Documents"],
              ] as const
            ).map(([key, label]) => (
              <label className="check-label" key={key}>
                <input
                  type="checkbox"
                  checked={c[key]}
                  onChange={(e) => update(key, e.target.checked)}
                />
                {label}
              </label>
            ))}
          </div>
        </>
      )}
      {!direct &&
        (config.kind === "media" || (!browserOnly && c.embedded_videos)) && (
          <>
            <h3>Subtitles & transcripts</h3>
            <label className="check-label">
              <input
                type="checkbox"
                checked={c.subtitles}
                onChange={(e) => update("subtitles", e.target.checked)}
              />
              Save available subtitles and searchable transcripts
            </label>
            <div className="form-grid">
              <div>
                <label htmlFor="subtitle-language">Subtitle language</label>
                <input
                  id="subtitle-language"
                  value={c.subtitle_language}
                  maxLength={35}
                  placeholder="original or en"
                  onChange={(e) => update("subtitle_language", e.target.value)}
                />
                <p className="settings-note">
                  Use original, or a language code such as en, es, or pt-BR.
                  This does not change the audio language.
                </p>
              </div>
              <label className="check-label">
                <input
                  type="checkbox"
                  checked={c.auto_captions}
                  onChange={(e) => update("auto_captions", e.target.checked)}
                />
                Allow auto-generated captions
              </label>
            </div>
          </>
        )}
    </section>
  );
}
