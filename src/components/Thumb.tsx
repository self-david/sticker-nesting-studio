import { useEffect, useState } from "react";
import { useI18n } from "../lib/i18n";

interface Props {
  src: string;
  alt: string;
  ext?: string;
  className?: string;
  /** Hides the explanatory line, for tiny previews. */
  compact?: boolean;
}

/**
 * Image with a visible failure state. If the Tauri asset protocol cannot
 * serve the file, the user sees which sticker is missing its preview instead
 * of a silent broken image.
 */
export function Thumb({ src, alt, ext, className, compact = false }: Props) {
  const { t } = useI18n();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [src]);

  if (!src || failed) {
    return (
      <div
        className={`thumb-missing${compact ? " is-compact" : ""}`}
        role="img"
        aria-label={`${alt}: ${t("thumb.failed")}`}
      >
        <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
          <rect x="3" y="4.5" width="18" height="15" rx="2.5" fill="none" stroke="currentColor" strokeWidth="1.6" />
          <path d="M3.5 16l5-5 4 4 3-3 5.5 5.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round" />
        </svg>
        <span className="thumb-missing-ext">{ext ? ext.toUpperCase() : "IMG"}</span>
        {!compact && <span className="thumb-missing-text">{t("thumb.failed")}</span>}
      </div>
    );
  }

  return (
    <img
      className={className}
      src={src}
      alt={alt}
      loading="lazy"
      decoding="async"
      draggable={false}
      onError={() => setFailed(true)}
    />
  );
}
