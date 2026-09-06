import { useState } from 'react';
import { sendJson } from '../api';
import styles from './page.module.css';

export default function Playground() {
  const [model, setModel] = useState('ariacompute/semantic-auto');
  const [prompt, setPrompt] = useState('please explain rust');
  const [out, setOut] = useState('');
  const [hdrs, setHdrs] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function send() {
    setBusy(true);
    setErr(null);
    try {
      const { data, headers } = await sendJson<Record<string, unknown>>(
        '/v1/router/chat',
        'POST',
        {
          model,
          messages: [{ role: 'user', content: prompt }],
          max_tokens: 64,
        },
      );
      setHdrs(
        [
          headers.get('x-aria-router-layer'),
          headers.get('x-aria-router-decision'),
          headers.get('x-aria-router-model'),
        ]
          .filter(Boolean)
          .join(' / '),
      );
      setOut(JSON.stringify(data, null, 2));
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h1 className={styles.h1}>Playground</h1>
      <div className="stack" style={{ maxWidth: '42rem' }}>
        <div className="stack" style={{ gap: '0.35rem' }}>
          <label className="stat-label" htmlFor="model">
            Model
          </label>
          <input
            id="model"
            className="input-field"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
        </div>
        <div className="stack" style={{ gap: '0.35rem' }}>
          <label className="stat-label" htmlFor="prompt">
            Message
          </label>
          <textarea
            id="prompt"
            className="input-field"
            rows={5}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
        </div>
        <div className={styles.row}>
          <button type="button" className="btn-primary" onClick={send} disabled={busy}>
            Send
          </button>
          {hdrs ? <span className="badge badge-accent">{hdrs}</span> : null}
          {err ? (
            <span className="alert alert-err" style={{ padding: '0.4rem 0.8rem' }}>
              {err}
            </span>
          ) : null}
        </div>
      </div>
      <pre className={styles.mono}>{out}</pre>
    </>
  );
}
