import type { Conversation } from './types';
import styles from './playground.module.css';

export default function ConversationSidebar({
  items,
  activeId,
  onSelect,
  onNew,
  onDelete,
}: {
  items: Conversation[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => void;
}) {
  return (
    <aside className={styles.sidebar}>
      <button type="button" className={`btn-primary ${styles.newChat}`} onClick={onNew}>
        New chat
      </button>
      <ul className={styles.convList}>
        {items.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              className={`${styles.convItem} ${c.id === activeId ? styles.convActive : ''}`}
              onClick={() => onSelect(c.id)}
              title={c.title}
            >
              <span className={styles.convTitle}>{c.title}</span>
            </button>
            <button
              type="button"
              className={styles.convDelete}
              aria-label={`Delete ${c.title}`}
              onClick={() => onDelete(c.id)}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
