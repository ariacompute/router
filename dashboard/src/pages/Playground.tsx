import { useState } from 'react';
import { sendJson } from '../api';
import styles from './page.module.css';

export default function Playground() {
  const [model, setModel] = useState('aria/semantic-auto');
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
          headers.get('x-ariarouter-layer'),
          headers.get('x-ariarouter-decision'),
          headers.get('x-ariarouter-model'),
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
      <label className={styles.label} htmlFor="model">
        Model
      </label>
      <input id="model" value={model} onChange={(e) => setModel(e.target.value)} />
      <label className={styles.label} htmlFor="prompt">
        Message
      </label>
      <textarea id="prompt" rows={5} value={prompt} onChange={(e) => setPrompt(e.target.value)} />
      <div className={styles.row}>
        <button type="button" onClick={send} disabled={busy}>
          Send
        </button>
        {hdrs ? <span>{hdrs}</span> : null}
        {err ? <span className={styles.err}>{err}</span> : null}
      </div>
      <pre className={styles.mono}>{out}</pre>
    </>
  );
}
