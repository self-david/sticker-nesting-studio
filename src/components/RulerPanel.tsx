import { useState } from "react";
import { useI18n } from "../lib/i18n";
import { clamp } from "../lib/sheet";
import { DEFAULT_PX_PER_MM, MAX_PX_PER_MM, MIN_PX_PER_MM, RULER_CM } from "../lib/scale";

interface Props {
  value: number;
  onClose: () => void;
  onSave: (pxPerMm: number) => void;
}

/**
 * A ten centimetre ruler to hold a real one against: whatever the bar measures
 * on screen becomes the scale the sheet is drawn at.
 */
export function RulerPanel({ value, onClose, onSave }: Props) {
  const { t } = useI18n();
  const [perMm, setPerMm] = useState(value);
  const lengthPx = perMm * RULER_CM * 10;
  const ticks = Array.from({ length: RULER_CM * 10 + 1 }, (_, i) => i);

  return (
    <div className="picker-backdrop" onPointerDown={onClose}>
      <div
        className="picker ruler-panel"
        role="dialog"
        aria-modal="true"
        aria-label={t("sheet.rulerTitle")}
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header className="picker-head">
          <div className="series-headtext">
            <span className="field-label">{t("sheet.rulerTitle")}</span>
            <p className="muted small series-note">{t("sheet.rulerNote")}</p>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label={t("detail.close")}>
            ✕
          </button>
        </header>

        <div className="ruler-track">
          <div className="ruler-bar" style={{ width: lengthPx }}>
            {ticks.map((tick) => (
              <span
                key={tick}
                className={`ruler-tick${tick % 10 === 0 ? " is-cm" : ""}`}
                style={{ left: tick * perMm }}
              >
                {tick % 10 === 0 && <em>{tick / 10}</em>}
              </span>
            ))}
            <span className="ruler-unit">{t("unit.cm")}</span>
          </div>
        </div>

        <div className="ruler-controls">
          <label className="sheet-field">
            <span className="field-label">{t("sheet.rulerWidth", { cm: RULER_CM })}</span>
            <div className="input-unit">
              <input
                className="text-input num mini"
                type="number"
                min={MIN_PX_PER_MM * RULER_CM * 10}
                max={MAX_PX_PER_MM * RULER_CM * 10}
                step={1}
                value={Math.round(lengthPx)}
                onChange={(event) =>
                  setPerMm(
                    clamp(Number(event.target.value) / (RULER_CM * 10), MIN_PX_PER_MM, MAX_PX_PER_MM),
                  )
                }
              />
              <span className="unit">px</span>
            </div>
          </label>
          <input
            className="ruler-slider"
            type="range"
            min={MIN_PX_PER_MM}
            max={MAX_PX_PER_MM}
            step={0.01}
            value={perMm}
            aria-label={t("sheet.rulerWidth", { cm: RULER_CM })}
            onChange={(event) =>
              setPerMm(clamp(Number(event.target.value), MIN_PX_PER_MM, MAX_PX_PER_MM))
            }
          />
        </div>

        <footer className="picker-foot">
          <button className="link-btn" onClick={() => setPerMm(DEFAULT_PX_PER_MM)}>
            {t("sheet.rulerReset")}
          </button>
          <div className="archive-actions">
            <button className="ghost-btn" onClick={onClose}>
              {t("common.cancel")}
            </button>
            <button className="primary-btn" onClick={() => onSave(perMm)}>
              {t("sheet.rulerSave")}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
