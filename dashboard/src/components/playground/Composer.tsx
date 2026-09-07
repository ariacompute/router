import {
  useLayoutEffect,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
} from 'react';
import type { ModelListItem } from '../../api';
import styles from './playground.module.css';

export default function Composer({
  model,
  models,
  value,
  busy,
  onModelChange,
  onChange,
  onSend,
  onStop,
}: {
  model: string;
  models: ModelListItem[];
  value: string;
  busy: boolean;
  onModelChange: (m: string) => void;
  onChange: (v: string) => void;
  onSend: () => void;
  onStop: () => void;
}) {
  const taRef = useRef<HTMLTextAreaElement>(null);

  useLayoutEffect(() => {
    const el = taRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 180)}px`;
  }, [value]);

  function onKeyDown(e: ReactKeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (!busy && value.trim()) onSend();
    }
  }

  return (
    <div className={styles.composer}>
      <div className={styles.composerTop}>
        <label className={styles.modelLabel} htmlFor="pg-model">
          Model
        </label>
        <select
          id="pg-model"
          className={styles.modelSelect}
          value={model}
          disabled={busy}
          onChange={(e) => onModelChange(e.target.value)}
        >
          {models.length === 0 ? (
            <option value={model}>{model || 'Loading…'}</option>
          ) : (
            models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.id}
                {m.owned_by ? ` (${m.owned_by})` : ''}
              </option>
            ))
          )}
        </select>
      </div>
      <div className={styles.composerRow}>
        <textarea
          ref={taRef}
          className={styles.composerInput}
          rows={1}
          placeholder="Ask me anything… (Enter to send, Shift+Enter for newline)"
          value={value}
          disabled={busy}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
        />
        {busy ? (
          <button type="button" className="btn-ghost" onClick={onStop}>
            Stop
          </button>
        ) : (
          <button
            type="button"
            className="btn-primary"
            onClick={onSend}
            disabled={!value.trim()}
          >
            Send
          </button>
        )}
      </div>
    </div>
  );
}
