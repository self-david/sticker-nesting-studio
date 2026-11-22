import { useEffect, useMemo, useRef, useState } from "react";
import type { Sticker } from "../types";
import { assetUrl } from "../lib/api";
import { useI18n } from "../lib/i18n";
import { matchedFacets, matchedTags, type ParsedQuery } from "../lib/search";
import { fmtMm } from "../lib/format";
import { Thumb } from "./Thumb";

const GAP = 14;
const MIN_CARD = 168;
const INFO_H = 56;
const HEADER_H = 36;
const OVERSCAN = 1;

/** One block of the grid: a whole library, or one series / one type. */
export interface GridGroup {
  key: string;
  label: string;
  items: Sticker[];
}

interface Props {
  groups: GridGroup[];
  /** Off for the plain library: no headers, one block. */
  showHeaders: boolean;
  selected: Set<string>;
  /** The active search, so each card can show the tags that matched. */
  query?: ParsedQuery;
  onSelect: (id: string, event: React.MouseEvent) => void;
  /** Quick “add one copy to the sheet”, without leaving the library. */
  onAdd: (sticker: Sticker) => void;
}

export function StickerGrid({ groups, showHeaders, selected, query, onSelect, onAdd }: Props) {
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

    const headers: { key: string; label: string; count: number; y: number }[] = [];
    const rows: { y: number; items: Sticker[] }[] = [];
    let y = 0;
    for (const group of groups) {
      if (showHeaders) {
        headers.push({ key: group.key, label: group.label, count: group.items.length, y });
        y += HEADER_H;
      }
      for (let index = 0; index < group.items.length; index += cols) {
        rows.push({ y, items: group.items.slice(index, index + cols) });
        y += rowH;
      }
    }

    const top = Math.max(0, scrollTop - HEADER_H - rowH * OVERSCAN);
    const bottom = scrollTop + size.h + HEADER_H + rowH * OVERSCAN;
    return {
      cols,
      cardW,
      cardH,
      totalH: Math.max(0, y - GAP),
      headers: headers.filter((header) => header.y + HEADER_H >= top && header.y <= bottom),
      rows: rows.filter((row) => row.y + cardH >= top && row.y <= bottom),
    };
  }, [groups, showHeaders, size, scrollTop]);

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
        {layout.headers.map((header) => (
          <div
            key={`header-${header.key || "none"}`}
            className="grid-group"
            style={{ transform: `translate3d(0, ${header.y}px, 0)` }}
          >
            <span className="grid-group-label">{header.label}</span>
            <span className="grid-group-count">{header.count}</span>
          </div>
        ))}

        {layout.rows.map((row) =>
          row.items.map((sticker, col) => {
            const isSel = selected.has(sticker.id);
            const thumb = assetUrl(sticker.thumbPath);
            // Series and types come first: they explain a `serie:` / `tipo:` match.
            const chips = query
              ? [
                  ...matchedFacets(sticker, query).map((facet) => ({
                    key: `facet-${facet.kind}`,
                    text: facet.text,
                    title: `${facet.kind === "serie" ? t("detail.serie") : t("detail.tipo")}: ${facet.text}`,
                    facet: true,
                  })),
                  ...matchedTags(sticker, query).map((tag) => ({
                    key: `tag-${tag}`,
                    text: tag,
                    title: `#${tag}`,
                    facet: false,
                  })),
                ]
              : [];
            const x = 14 + col * (layout.cardW + GAP);
            return (
              <div
                key={sticker.id}
                className={`card${isSel ? " is-selected" : ""}`}
                style={{
                  width: layout.cardW,
                  height: layout.cardH,
                  transform: `translate3d(${x}px, ${row.y}px, 0)`,
                  left: 0,
                  top: 0,
                }}
                onClick={(e) => onSelect(sticker.id, e)}
                title={sticker.name}
              >
                <div className="card-thumb" style={{ height: layout.cardW }}>
                  <Thumb src={thumb} ext={sticker.ext} alt={sticker.name} />
                  {chips.length > 0 && (
                    <div className="card-matches">
                      {chips.slice(0, 3).map((chip) => (
                        <span
                          key={chip.key}
                          className={`match-chip${chip.facet ? " is-facet" : ""}`}
                          title={chip.title}
                        >
                          {chip.text}
                        </span>
                      ))}
                      {chips.length > 3 && (
                        <span className="match-chip">+{chips.length - 3}</span>
                      )}
                    </div>
                  )}
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
                    {fmtMm(sticker.widthMm, locale)} × {fmtMm(sticker.heightMm, locale)}{" "}
                    {t("unit.mm")}
                  </span>
                </div>
              </div>
            );
          }),
        )}
      </div>
    </div>
  );
}
