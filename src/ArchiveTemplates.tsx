import { useEffect, useState } from "react";
import { Check, LoaderCircle, MonitorSmartphone, Palette } from "lucide-react";
import { api } from "./api";
import type { ArchiveTemplate, ArchiveTemplateOption } from "./types";

export function ArchiveTemplates({
  value,
  onChange,
  disabled,
}: {
  value: ArchiveTemplate;
  onChange: (template: ArchiveTemplate) => void;
  disabled: boolean;
}) {
  const [templates, setTemplates] = useState<ArchiveTemplateOption[]>([]);
  const [html, setHtml] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let active = true;
    api
      .archiveTemplates()
      .then((items) => {
        if (active) setTemplates(items);
      })
      .catch((error) => {
        if (active) setError(String(error));
      });
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError("");
    setHtml("");
    api
      .previewArchiveTemplate(value)
      .then((html) => {
        if (active) setHtml(html);
      })
      .catch((error) => {
        if (active) setError(String(error));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [value]);
  const selected = templates.find((template) => template.id === value);
  return (
    <>
      <div className="appearance-heading">
        <div>
          <h2>Archive appearance</h2>
          <p>Give your offline collection a home worth opening.</p>
        </div>
        <Palette size={21} />
      </div>
      <div
        className="template-options"
        role="radiogroup"
        aria-label="Archive index template"
      >
        {templates.map((template) => (
          <label
            className={`template-option ${template.id === value ? "selected" : ""}`}
            key={template.id}
          >
            <input
              type="radio"
              name="archive-template"
              aria-label={template.name}
              value={template.id}
              checked={template.id === value}
              disabled={disabled}
              onChange={() => onChange(template.id)}
            />
            <span
              className={`template-swatch ${template.id}`}
              aria-hidden="true"
            >
              <span className="swatch-top">
                <i />
                <i />
              </span>
              <span className="swatch-hero">
                <b>
                  {template.id === "screening_room"
                    ? "Your front row.\nAnywhere."
                    : template.id === "field_notes"
                      ? "Collected for\nthe curious."
                      : template.id === "midnight"
                        ? "A little more\ndiscovery."
                        : "Your world,\nwithin reach."}
                </b>
                <i />
              </span>
              <span className="swatch-search" />
              <span className="swatch-cards">
                <i />
                <i />
                <i />
                <i />
              </span>
            </span>
            <span className="template-option-name">
              {template.name}
              {value === template.id && <Check size={15} />}
            </span>
            <span className="template-option-description">
              {template.description}
            </span>
          </label>
        ))}
      </div>
      <div className="template-preview-heading">
        <div>
          <strong>{selected?.name || "Archive"} preview</strong>
          <span>Static style preview · sample content</span>
        </div>
        <MonitorSmartphone size={18} />
      </div>
      {error ? (
        <div className="alert error" role="alert">
          Could not load archive preview: {error}
        </div>
      ) : (
        <div className="template-preview" aria-busy={loading}>
          {loading ? (
            <div className="template-loading">
              <LoaderCircle className="spin" size={22} />
              <span>Preparing preview…</span>
            </div>
          ) : (
            <iframe
              key={value}
              title={`${selected?.name || "Archive"} index page preview`}
              sandbox=""
              srcDoc={html}
            />
          )}
        </div>
      )}
      <p className="settings-note">
        The template styles the contents, search page, and media player pages.
        Captured website pages keep their original layout, with a matching
        collection navigation bar. Fonts and styles are included in the ZIM.
        Applies to new jobs. To change a downloaded collection, open its Details
        and choose a template under Rebuild.
      </p>
    </>
  );
}
