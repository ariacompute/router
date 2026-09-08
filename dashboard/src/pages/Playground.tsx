import { useCallback, useEffect, useRef, useState } from 'react';
import {
  listRouterModels,
  streamChat,
  type ChatMessage,
  type ModelListItem,
} from '../api';
import ChatViewport from '../components/playground/ChatViewport';
import Composer from '../components/playground/Composer';
import ConversationSidebar from '../components/playground/ConversationSidebar';
import styles from '../components/playground/playground.module.css';
import {
  newId,
  titleFromPrompt,
  type PlaygroundMessage,
} from '../components/playground/types';
import { useConversationStore } from '../components/playground/useConversationStore';

const PREFERRED_MODEL = 'ariacompute/semantic-auto';

export default function Playground() {
  const [models, setModels] = useState<ModelListItem[]>([]);
  const [defaultModel, setDefaultModel] = useState(PREFERRED_MODEL);
  const [input, setInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  const {
    conversations,
    active,
    activeId,
    setActiveId,
    updateActive,
    newConversation,
    deleteConversation,
    hydrated,
  } = useConversationStore(defaultModel);

  useEffect(() => {
    let cancelled = false;
    listRouterModels()
      .then((list) => {
        if (cancelled) return;
        setModels(list);
        const preferred =
          list.find((m) => m.id === PREFERRED_MODEL)?.id ?? list[0]?.id ?? PREFERRED_MODEL;
        setDefaultModel(preferred);
      })
      .catch(() => {
        /* keep preferred default */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const model = active?.model ?? defaultModel;

  const setModel = useCallback(
    (m: string) => {
      updateActive((c) => ({ ...c, model: m }));
    },
    [updateActive],
  );

  const stop = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setBusy(false);
  }, []);

  const send = useCallback(async () => {
    const prompt = input.trim();
    if (!prompt || !active || busy) return;

    setErr(null);
    setInput('');
    setBusy(true);

    const userMsg: PlaygroundMessage = {
      id: newId('msg'),
      role: 'user',
      content: prompt,
    };
    const assistantId = newId('msg');
    const assistantMsg: PlaygroundMessage = {
      id: assistantId,
      role: 'assistant',
      content: '',
      streaming: true,
    };

    updateActive((c) => ({
      ...c,
      title: c.messages.length === 0 ? titleFromPrompt(prompt) : c.title,
      messages: [...c.messages, userMsg, assistantMsg],
    }));

    const history: ChatMessage[] = [
      ...active.messages.map((m) => ({ role: m.role, content: m.content })),
      { role: 'user', content: prompt },
    ];

    const ac = new AbortController();
    abortRef.current = ac;

    try {
      await streamChat(
        // Omit max_tokens so upstream uses its default (matches curl / OpenAI clients).
        { model, messages: history },
        {
          signal: ac.signal,
          onHeaders: (headers) => {
            updateActive((c) => ({
              ...c,
              messages: c.messages.map((m) =>
                m.id === assistantId ? { ...m, headers: { ...m.headers, ...headers } } : m,
              ),
            }));
          },
          onDelta: (delta) => {
            updateActive((c) => ({
              ...c,
              messages: c.messages.map((m) =>
                m.id === assistantId ? { ...m, content: m.content + delta } : m,
              ),
            }));
          },
          onUsage: (usage) => {
            const total =
              usage.total_tokens ??
              (usage.prompt_tokens ?? 0) + (usage.completion_tokens ?? 0);
            if (!total) return;
            const label =
              usage.prompt_tokens != null || usage.completion_tokens != null
                ? `${usage.prompt_tokens ?? 0}+${usage.completion_tokens ?? 0}`
                : String(total);
            updateActive((c) => ({
              ...c,
              messages: c.messages.map((m) =>
                m.id === assistantId
                  ? {
                      ...m,
                      headers: { ...m.headers, 'x-aria-usage-tokens': label },
                    }
                  : m,
              ),
            }));
          },
        },
      );
      updateActive((c) => ({
        ...c,
        messages: c.messages.map((m) =>
          m.id === assistantId ? { ...m, streaming: false } : m,
        ),
      }));
    } catch (e) {
      if ((e as Error).name === 'AbortError') {
        updateActive((c) => ({
          ...c,
          messages: c.messages.map((m) =>
            m.id === assistantId
              ? { ...m, streaming: false, content: m.content || '(stopped)' }
              : m,
          ),
        }));
      } else {
        const message = (e as Error).message || 'request failed';
        setErr(message);
        updateActive((c) => ({
          ...c,
          messages: c.messages.map((m) =>
            m.id === assistantId
              ? { ...m, streaming: false, error: message, content: m.content }
              : m,
          ),
        }));
      }
    } finally {
      abortRef.current = null;
      setBusy(false);
    }
  }, [active, busy, input, model, updateActive]);

  if (!hydrated || !active) {
    return <div className={styles.root}>Loading…</div>;
  }

  return (
    <div className={styles.root}>
      <ConversationSidebar
        items={conversations}
        activeId={activeId}
        onSelect={setActiveId}
        onNew={() => newConversation(defaultModel)}
        onDelete={deleteConversation}
      />
      <div className={styles.main}>
        <ChatViewport messages={active.messages} busy={busy} />
        {err ? <div className={styles.errBanner}>{err}</div> : null}
        <Composer
          model={model}
          models={models.length ? models : [{ id: model }]}
          value={input}
          busy={busy}
          onModelChange={setModel}
          onChange={setInput}
          onSend={send}
          onStop={stop}
        />
      </div>
    </div>
  );
}
