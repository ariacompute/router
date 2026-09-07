import { useCallback, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import RouteHeaders from './RouteHeaders';
import type { PlaygroundMessage } from './types';
import styles from './playground.module.css';

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* ignore */
    }
  }, [text]);
  if (!text) return null;
  return (
    <button type="button" className={styles.copyBtn} onClick={copy}>
      {copied ? 'Copied' : 'Copy'}
    </button>
  );
}

export default function MessageBubble({ message }: { message: PlaygroundMessage }) {
  const isUser = message.role === 'user';
  return (
    <div className={`${styles.bubbleRow} ${isUser ? styles.bubbleUser : styles.bubbleAssistant}`}>
      <div className={styles.bubble}>
        {isUser ? (
          <div className={styles.userText}>{message.content}</div>
        ) : (
          <>
            {message.error ? (
              <div className={styles.errorText}>{message.error}</div>
            ) : (
              <div className={styles.md}>
                <ReactMarkdown remarkPlugins={[remarkGfm]}>
                  {message.content || (message.streaming ? '…' : '')}
                </ReactMarkdown>
                {message.streaming ? <span className={styles.cursor} aria-hidden /> : null}
              </div>
            )}
            {message.headers && !message.streaming ? (
              <RouteHeaders headers={message.headers} />
            ) : null}
            {!message.streaming && message.content ? <CopyButton text={message.content} /> : null}
          </>
        )}
      </div>
    </div>
  );
}
