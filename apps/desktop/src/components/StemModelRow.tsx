/**
 * The stem model in Settings → Account (#385): `ModelRow` with the htdemucs
 * model's id and words (`lib/stemModel.ts`). Like the speech and speaker
 * models', it downloads only when the user presses Download; the rules are in
 * `ModelRow.tsx`, which holds the only call of `downloadMlModel`.
 */

import { useEffect, useState } from "react";

import {
  STEM_DISCARD_QUESTION,
  STEM_MODEL_ID,
  STEM_MODEL_LICENCES,
  STEM_MODEL_NOTE,
  STEM_MODEL_SOURCE,
  STEM_MODEL_TITLE,
  stemReadyText,
  stemRemoveQuestion,
} from "../lib/stemModel";
import { SPEECH_MODEL_PENDING } from "../lib/speechModel";
import { isAndroid } from "../lib/platform";
import { deviceInfo } from "../lib/tauri-bridge";
import { ModelRow, type ModelCopy } from "./ModelRow";

const COPY: ModelCopy = {
  id: STEM_MODEL_ID,
  prefix: "stem-model",
  heading: "Stem model",
  title: STEM_MODEL_TITLE,
  source: STEM_MODEL_SOURCE,
  note: STEM_MODEL_NOTE,
  licences: STEM_MODEL_LICENCES,
  pending: SPEECH_MODEL_PENDING,
  subject: "the stem model",
  progressLabel: "Stem model download",
  removeQuestion: stemRemoveQuestion,
  discardQuestion: STEM_DISCARD_QUESTION,
  readyText: stemReadyText,
};

/**
 * Shown in place of the row on an Android device with too little memory.
 * The model peaks near 3 GB, and Android closes an app that takes that much
 * on a smaller device, so the download is not offered there: 171 MB for
 * something that cannot run.
 */
export const STEMS_NEED_MEMORY =
  "Stem separation needs about 3 GB of free memory, so it runs on a phone or tablet with 8 GB or more. This device has less, and the stem model is not offered here.";

export function StemModelRow() {
  // Android only: a desktop is never refused, and is not asked.
  const [tooSmall, setTooSmall] = useState(false);
  useEffect(() => {
    if (!isAndroid()) return undefined;
    let cancelled = false;
    deviceInfo()
      .then((device) => {
        if (!cancelled) setTooSmall(!device.stems_supported);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  if (tooSmall) {
    return (
      <div data-testid="stem-model-unavailable" className="mb-4">
        <div className="mb-1.5 font-mono text-[10px] uppercase tracking-[0.18em] text-[var(--text-faint)]">
          Stem model
        </div>
        <p className="rounded-md border border-[var(--border)] bg-[var(--surface)] px-3 py-2 text-xs text-[var(--text-dim)]">
          {STEMS_NEED_MEMORY}
        </p>
      </div>
    );
  }
  return <ModelRow copy={COPY} />;
}
