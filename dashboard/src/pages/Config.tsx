import { useEffect, useState } from 'react';
import { getJson, putText, type ConfigPayload } from '../api';
import styles from './page.module.css';

export default function Config() {
  const [yaml, setYaml] = useState('');
  const [doc, setDoc] = useState<unknown>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  function load() {
    setErr(null);
    getJson<ConfigPayload>('/v1/router/config')
      .then((p) => {
        setYaml(p.yaml);
        setDoc(p.document);
      })
      .catch((e: Error) => setErr(e.message));
  }

  useEffect(load, []);

  async function save() {
    setBusy(true);
    setErr(null);
    setMsg(null);
    try {
      await putText('/v1/router/config', yaml, 'application/yaml');
      setMsg('Saved and reloaded.');
      load();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h1 className={styles.h1}>Config</h1>
      <p>Edit the in-memory YAML v0.3 document. Save validates, then writes the serve `--config` path.</p>
      <div className={styles.row}>
        <button type="button" className="btn-primary" onClick={save} disabled={busy}>
          Save
        </button>
        <button type="button" className="btn-ghost" onClick={load} disabled={busy}>
          Reload
        </button>
        {msg ? (
          <span className="alert alert-ok" style={{ padding: '0.4rem 0.8rem' }}>
            {msg}
          </span>
        ) : null}
        {err ? (
          <span className="alert alert-err" style={{ padding: '0.4rem 0.8rem' }}>
            {err}
          </span>
        ) : null}
      </div>
      <textarea rows={22} value={yaml} onChange={(e) => setYaml(e.target.value)} spellCheck={false} />
      <h2 className={styles.h2}>Parsed</h2>
      <pre className={styles.mono}>{doc ? JSON.stringify(doc, null, 2) : ''}</pre>
    </>
  );
}
