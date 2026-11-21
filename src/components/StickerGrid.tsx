import { useEffect, useMemo, useRef, useState } from "react";
import type { Sticker } from "../types";
import { assetUrl } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { fmtMm } from "../lib/format";
import { Thumb } from "./Thumb";

const GAP = 14;
const MIN_CARD = 168;
const INFO_H = 56;
const OVERSCAN = 2;

interface Props {
  items: Sticker[];
  selected: Set<string>;
  onSelect: (id: string, event: React.MouseEvent) => void;
  /** Quick “add one copy to the sheet”, without leaving the library. */
  onAdd: (sticker: Sticker) => void;
}

export function StickerGrid({ items, selected, onSelect, onAdd }: Props) {
  const { t, locale } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const [scrollTop, setScrollTop] = useState(0);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: el.clientHeight });
      setScrollTop(el.scrollTop);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(() => {
    const innerW = Math.max(0, size.w - 28); // padding
    const cols = innerW > 0 ? Math.max(1, Math.floor((innerW + GAP) / (MIN_CARD + GAP))) : 1;
    const cardW = innerW > 0 ? (innerW - GAP * (cols - 1)) / cols : MIN_CARD;
    const cardH = cardW + INFO_H;
    const rowH = cardH + GAP;
    const rows = Math.ceil(items.length / cols);
    const totalH = Math.max(0, rows * rowH - GAP);
    const firstRow = Math.max(0, Math.floor(scrollTop / rowH) - OVERSCAN);
    const lastRow = Math.min(rows - 1, Math.floor((scrollTop + size.h) / rowH) + OVERSCAN);
    return { cols, cardW, cardH, rowH, rows, totalH, firstRow, lastRow };
  }, [size, items.length, scrollTop]);

  const from = layout.firstRow * layout.cols;
  const to = Math.min(items.length, (layout.lastRow + 1) * layout.cols);
  const visible = items.slice(from, to);

  return (
    <div
      className="grid-scroll"
      ref={scrollRef}
      onScroll={(e) => {
        const el = e.currentTarget;
        if (el) setScrollTop(el.scrollTop);
      }}
    >
      <div className="grid-inner" style={{ height: layout.totalH }}>
        {visible.map((sticker, index) => {
          const absolute = from + index;
          const row = Math.floor(absolute / layout.cols);
          const col = absolute % layout.cols;
          const x = 14 + col * (layout.cardW + GAP);
          const y = row * layout.rowH;
          const isSel = selected.has(sticker.id);
          const thumb = assetUrl(sticker.thumbPath);
          return (
            <div
              key={sticker.id}
              className={`card${isSel ? " is-selected" : ""}`}
              style={{
                width: layout.cardW,
                height: layout.cardH,
                transform: `translate3d(${x}px, ${y}px, 0)`,
                left: 0,
                top: 0,
              }}
              onClick={(e) => onSelect(sticker.id, e)}
              title={sticker.name}
            >
              <div className="card-thumb" style={{ height: layout.cardW }}>
                <Thumb src={thumb} ext={sticker.ext} alt={sticker.name} />
                {sticker.kind === "vector" && <span className="card-badge">SVG</span>}
                <button
                  className="card-add"
                  title={t("card.addToSheet")}
                  aria-label={`${t("card.addToSheet")}: ${sticker.name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onAdd(sticker);
                  }}
                >
                  +
                </button>
              </div>
              <div className="card-info">
                <span className="card-name">{sticker.name}</span>
                <span className="card-size">
                  {fmtMm(sticker.widthMm, locale)} × {fmtMm(sticker.heightMm, locale)} {t("unit.mm")}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
