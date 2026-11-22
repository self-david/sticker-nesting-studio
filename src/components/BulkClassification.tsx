import { useState } from "react";
import type { FacetCount } from "../types";
import { useI18n } from "../lib/i18n";
import { TIPOS } from "../lib/tipos";

/** Sentinel: «leave the type as it is»; the empty value means «no type». */
const KEEP = "__keep";

interface Props {
  series: FacetCount[];
  onApply: (patch: { serie?: string; tipo?: string }) => void;
}

/**
 * Serie and type for the whole selection at once. Only what the user touched
 * travels, so fixing the type of 30 stickers never wipes their series.
 */
export function BulkClassification({ series, onApply }: Props) {
  const { t } = useI18n();
  const [serie, setSerie] = useState("");
  const [serieTouched, setSerieTouched] = useState(false);
  const [tipo, setTipo] = useState(KEEP);

  const ready = serieTouched || tipo !== KEEP;

  const apply = () => {
    if (!ready) return;
    onApply({
      serie: serieTouched ? serie.trim() : undefined,
      tipo: tipo === KEEP ? undefined : tipo,
    });
  };

  return (
    <div className="bulk">
      <input
        className="text-input bulk-serie"
        list="bulk-series"
        value={serie}
        placeholder={t("selection.seriePlaceholder")}
        aria-label={t("detail.serie")}
        onChange={(event) => {
          setSerie(event.target.value);
          setSerieTouched(true);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") apply();
        }}
      />
      <datalist id="bulk-series">
        {series
          .filter((facet) => facet.value !== "")
          .map((facet) => (
            <option key={facet.value} value={facet.value} />
          ))}
      </datalist>
      <select
        className="bulk-tipo"
        value={tipo}
        aria-label={t("detail.tipo")}
        onChange={(event) => setTipo(event.target.value)}
      >
        <option value={KEEP}>{t("selection.tipoKeep")}</option>
        <option value="">{t("facet.noTipo")}</option>
        {TIPOS.map((value) => (
          <option key={value} value={value}>
            {value}
          </option>
        ))}
      </select>
      <button className="ghost-btn bulk-apply" onClick={apply} disabled={!ready}>
        {t("selection.apply")}
      </button>
    </div>
  );
}
