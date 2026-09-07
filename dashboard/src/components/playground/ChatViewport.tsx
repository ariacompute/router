import { useEffect, useRef } from 'react';
import MessageBubble from './MessageBubble';
import type { PlaygroundMessage } from './types';
import styles from './playground.module.css';

export default function ChatViewport({
  messages,
  busy,
}: {
  messages: PlaygroundMessage[];
  busy: boolean;
}) {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [messages, busy]);

  if (messages.length === 0) {
    return (
      <div className={styles.empty}>
        <h2 className={styles.emptyTitle}>Playground</h2>
        <p className={styles.emptyHint}>
          Test routing with a multi-turn chat. Try “please explain rust” on{' '}
          <code>ariacompute/semantic-auto</code>.
        </p>
      </div>
    );
  }

  return (
    <div className={styles.viewport}>
      {messages.map((m) => (
        <MessageBubble key={m.id} message={m} />
      ))}
      <div ref={bottomRef} />
    </div>
  );
}
