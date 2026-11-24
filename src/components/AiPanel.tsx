import { useEffect, useRef, useState } from "react";
import type { AiFailure, AiKeyCheck, AiKeyInfo, AiStatus, Sticker } from "../types";
import {
  aiAddKey,
  aiDescribe,
  aiRemoveKey,
  aiReorderKeys,
  aiStatus,
  aiTagSuggestions,
  aiTestKey,
  aiUpdateKey,
  mergeTags,
} from "../lib/api";
import { useI18n } from "../lib/i18n";
import { MergeSuggestions } from "./MergeSuggestions";

interface Props {
  open: boolean;
  /** Every sticker that has never been processed by the AI. */
  pending: Sticker[];
  total: number;
  onClose: () => void;
  /** Reloads the library once the run is over. */
  onUpdated: () => Promise<void> | void;
  /** Stickers the text classifier has not looked at yet. */
  unclassified: number;
  /** Runs the text-only classification and reports back through a toast. */
  onClassify: () => Promise<void>;
}

interface KeyRowProps {
  info: AiKeyInfo;
  index: number;
  total: number;
  busy: boolean;
  check: AiKeyCheck | null;
  testing: boolean;
  onTest: (id: string) => void;
  onRename: (id: string, name: string) => void;
  onMove: (index: number, delta: number) => void;
  onRemove: (id: string) => void;
}

function KeyRow({
  info,
  index,
  total,
  busy,
  check,
  testing,
  onTest,
  onRename,
  onMove,
  onRemove,
}: KeyRowProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(info.name);

  useEffect(() => {
    setDraft(info.name);
  }, [info.name]);

  return (
    <li className="ai-key-row">
      <span className="ai-key-order">{index + 1}</span>
      <input
        className="text-input ai-key-name"
        value={draft}
        disabled={busy}
        aria-label={t("ai.keyNamePlaceholder")}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          const name = draft.trim();
          if (name && name !== info.name) onRename(info.id, name);
          else setDraft(info.name);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
      />
      <span className="ai-key-hint">{info.hint}</span>
      <button
        className={`ai-key-check${check ? ` is-${check.status}` : ""}`}
        onClick={() => onTest(info.id)}
        disabled={busy || testing}
        title={check?.message || t("ai.keyTest")}
      >
        {testing
          ? t("ai.keyTesting")
          : !check
            ? t("ai.keyTest")
            : check.status === "ok"
              ? t("ai.checkOk")
              : check.status === "limited"
                ? t("ai.checkLimited", { s: Math.ceil(check.wait) })
                : check.status === "invalid"
                  ? t("ai.checkInvalid")
                  : t("ai.checkError")}
      </button>
      <div className="ai-key-actions">
        <button
          className="icon-btn"
          disabled={busy || index === 0}
          title={t("ai.keyUp")}
          onClick={() => onMove(index, -1)}
        >
          ↑
        </button>
        <button
          className="icon-btn"
          disabled={busy || index === total - 1}
          title={t("ai.keyDown")}
          onClick={() => onMove(index, 1)}
        >
          ↓
        </button>
        <button
          className="icon-btn"
          disabled={busy}
          title={t("ai.keyRemove")}
          onClick={() => onRemove(info.id)}
        >
          ✕
        </button>
      </div>
    </li>
  );
}

export function AiPanel({
  open,
  pending,
  total,
  onClose,
  onUpdated,
  unclassified,
  onClassify,
}: Props) {
  const { t } = useI18n();
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [nameInput, setNameInput] = useState("");
  const [keyInput, setKeyInput] = useState("");
  const [adding, setAdding] = useState(false);
  const [busyKey, setBusyKey] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [checks, setChecks] = useState<Record<string, AiKeyCheck>>({});
  const [testingId, setTestingId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const [classifying, setClassifying] = useState(false);
  const [done, setDone] = useState(0);
  const [goal, setGoal] = useState(0);
  const [failures, setFailures] = useState<AiFailure[]>([]);
  const [summary, setSummary] = useState<{
    updated: number;
    failed: number;
    cancelled: boolean;
    cached: number;
  } | null>(null);
  const cancelRef = useRef(false);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setKeyError(null);
    setSummary(null);
    setFailures([]);
    setChecks({});
    setDone(0);
    aiStatus()
      .then((next) => {
        if (!cancelled) setStatus(next);
      })
      .catch((err) => {
        if (!cancelled) setKeyError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  // Esc closes the dialog, like the ✕: not while a batch is halfway through.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || running || classifying) return;
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, running, classifying, onClose]);

  if (!open) return null;

  const keys = status?.keys ?? [];
  const configured = keys.length > 0;
  const locked = running || classifying || adding || busyKey;

  const classify = async () => {
    setClassifying(true);
    try {
      await onClassify();
    } finally {
      setClassifying(false);
    }
  };

  const applyStatus = (next: AiStatus) => {
    setStatus(next);
    setKeyError(null);
  };

  const addKey = async () => {
    const key = keyInput.trim();
    if (!key) return;
    setAdding(true);
    setKeyError(null);
    try {
      applyStatus(await aiAddKey(nameInput, key));
      setKeyInput("");
      setNameInput("");
      setSummary(null);
    } catch (err) {
      setKeyError(String(err));
    } finally {
      setAdding(false);
    }
  };

  const testKey = async (id: string) => {
    setTestingId(id);
    try {
      const check = await aiTestKey(id);
      setChecks((prev) => ({ ...prev, [id]: check }));
    } catch (err) {
      setChecks((prev) => ({
        ...prev,
        [id]: { status: "error", message: String(err), wait: 0, ms: 0 },
      }));
    } finally {
      setTestingId(null);
    }
  };

  const renameKey = async (id: string, name: string) => {
    setBusyKey(true);
    try {
      applyStatus(await aiUpdateKey(id, { name }));
    } catch (err) {
      setKeyError(String(err));
    } finally {
      setBusyKey(false);
    }
  };

  const removeKey = async (id: string) => {
    setBusyKey(true);
    try {
      applyStatus(await aiRemoveKey(id));
    } catch (err) {
      setKeyError(String(err));
    } finally {
      setBusyKey(false);
    }
  };

  // The order of the list is the order the run uses.
  const moveKey = async (index: number, delta: number) => {
    const target = index + delta;
    if (target < 0 || target >= keys.length) return;
    const ids = keys.map((k) => k.id);
    [ids[index], ids[target]] = [ids[target], ids[index]];
    setBusyKey(true);
    try {
      applyStatus(await aiReorderKeys(ids));
    } catch (err) {
      setKeyError(String(err));
    } finally {
      setBusyKey(false);
    }
  };

  // One sticker at a time; each request takes the next key in the list.
  const run = async () => {
    if (!configured || !pending.length || running) return;
    cancelRef.current = false;
    setRunning(true);
    setDone(0);
    setGoal(pending.length);
    setFailures([]);
    setSummary(null);

    const queue = pending.map((s) => s.id);
    const failed: AiFailure[] = [];
    let processed = 0;
    let cached = 0;
    let stop = false;

    while (queue.length && !cancelRef.current && !stop) {
      const id = queue.shift();
      if (!id) break;
      try {
        const report = await aiDescribe([id]);
        cached += report.cached;
        for (const failure of report.failures) {
          failed.push(failure);
          if (/api key|clave/i.test(failure.message)) stop = true;
        }
      } catch (err) {
        const message = String(err);
        failed.push({ id, message });
        if (/api key|clave/i.test(message)) stop = true;
      }
      processed += 1;
      setDone(processed);
    }

    setRunning(false);
    setFailures(failed);
    setSummary({
      updated: processed - failed.length,
      failed: failed.length,
      cancelled: cancelRef.current || stop,
      cached,
    });
    await onUpdated();
  };

  const remaining = pending.length;

  return (
    <div
      className="ai-backdrop"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget && !running && !classifying) onClose();
      }}
    >
      <div className="ai-panel" role="dialog" aria-modal="true" aria-label={t("ai.title")}>
        <header className="ai-head">
          <div>
            <h2>
              <span className="ai-spark" aria-hidden="true">
                ✦
              </span>
              {t("ai.title")}
            </h2>
            <p className="muted small">{t("ai.subtitle")}</p>
          </div>
          <button className="icon-btn" onClick={onClose} title={t("ai.close")} disabled={running}>
            ✕
          </button>
        </header>

        <div className="ai-body">
          <section className="ai-field">
          <div className="ai-field-head">
            <span className="field-label">
              {t("ai.keysTitle")} · {keys.length}
            </span>
            {keys.length > 1 && <span className="muted small">{t("ai.keysOrder")}</span>}
          </div>

          {status === null && <span className="ai-status warn">{t("ai.loading")}</span>}
          {status !== null && keys.length === 0 && (
            <span className="ai-status warn">{t("ai.keysNone")}</span>
          )}

          {keys.length > 0 && (
            <ul className="ai-key-list">
              {keys.map((info, index) => (
                <KeyRow
                  key={info.id}
                  info={info}
                  index={index}
                  total={keys.length}
                  busy={running || busyKey}
                  check={checks[info.id] ?? null}
                  testing={testingId === info.id}
                  onTest={(id) => void testKey(id)}
                  onRename={(id, name) => void renameKey(id, name)}
                  onMove={(i, delta) => void moveKey(i, delta)}
                  onRemove={(id) => void removeKey(id)}
                />
              ))}
            </ul>
          )}

          <div className="ai-key-add">
            <input
              className="text-input ai-key-name"
              value={nameInput}
              placeholder={t("ai.keyNamePlaceholder")}
              autoComplete="off"
              disabled={locked}
              onChange={(e) => setNameInput(e.target.value)}
            />
            <input
              className="text-input"
              type="password"
              value={keyInput}
              placeholder={t("ai.keyPlaceholder")}
              autoComplete="off"
              spellCheck={false}
              disabled={locked}
              onChange={(e) => setKeyInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void addKey();
              }}
            />
            <button
              className="primary-btn"
              onClick={() => void addKey()}
              disabled={locked || !keyInput.trim()}
            >
              + {t("ai.keyAdd")}
            </button>
          </div>

            {keyError && <span className="err">{keyError}</span>}
          </section>

          {(running || summary) && (
            <section className="ai-progress-box">
              <div className="ai-progress" role="progressbar" aria-valuemin={0} aria-valuemax={goal} aria-valuenow={done}>
                <span style={{ width: `${goal ? Math.round((done / goal) * 100) : 0}%` }} />
              </div>
              <span className="muted small">
                {running
                  ? t("ai.running", { done, n: goal })
                  : summary?.cancelled
                    ? t("ai.cancelled", { n: done })
                    : summary && summary.cached > 0
                      ? t("ai.doneCached", {
                          n: summary.updated,
                          cached: summary.cached,
                        })
                      : t("ai.done", { n: summary?.updated ?? 0 })}
              </span>
              {!running && (summary?.failed ?? 0) > 0 && (
                <span className="err small">{t("ai.failed", { n: summary?.failed ?? 0 })}</span>
              )}
              {failures.length > 0 && (
                <ul className="ai-errors">
                  {failures.slice(0, 4).map((failure, index) => (
                    <li key={`${failure.id}-${index}`}>{failure.message}</li>
                  ))}
                </ul>
              )}
            </section>
          )}

          <section className="ai-field">
            <div className="ai-field-head">
              <span className="field-label">{t("ai.classifyTitle")}</span>
            </div>
            <span className="muted small">{t("ai.classifyNote")}</span>
            <span className="muted small">
              {t("ai.classifyModel", { model: status?.textModel ?? "openai/gpt-oss-20b" })}
            </span>
            <button
              className="ghost-btn"
              onClick={() => void classify()}
              disabled={locked || classifying || unclassified === 0}
              title={configured ? undefined : t("ai.needKey")}
            >
              {classifying
                ? t("ai.classifying")
                : unclassified === 0
                  ? t("ai.classifyNone")
                  : t("ai.classify", { n: unclassified })}
            </button>
          </section>

          <MergeSuggestions
            disabled={locked}
            scan={aiTagSuggestions}
            apply={async (group) => {
              const moved = await mergeTags(group.variantes, group.canonico);
              await onUpdated();
              return t("ai.tagsMerged", { n: moved, name: group.canonico });
            }}
            keys={{
              title: "ai.tagsTitle",
              note: "ai.tagsNote",
              scan: "ai.tagsScan",
              scanning: "ai.tagsScanning",
              none: "ai.tagsNone",
              apply: "ai.tagsApply",
              applying: "ai.tagsApplying",
              failed: "ai.tagsFailed",
            }}
          />
        </div>

        <section className="ai-run">
          <div>
            <strong>
              {remaining === 0
                ? t("ai.pendingNone")
                : t("ai.pending", { n: remaining, total })}
            </strong>
            <p className="muted small ai-note">{t("ai.note")}</p>
          </div>
          {!running ? (
            <button
              className="primary-btn"
              onClick={() => void run()}
              disabled={!configured || remaining === 0}
              title={configured ? undefined : t("ai.needKey")}
            >
              {t("ai.run", { n: remaining })}
            </button>
          ) : (
            <button className="ghost-btn" onClick={() => (cancelRef.current = true)}>
              {t("ai.cancel")}
            </button>
          )}
        </section>
      </div>
    </div>
  );
}
