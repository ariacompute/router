import { useCallback, useEffect, useState } from 'react';
import type { Conversation } from './types';
import { newId } from './types';

const STORAGE_KEY = 'aria-router:playground:conversations';
const MAX_CONVERSATIONS = 20;

function readStore(): Conversation[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as Conversation[];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function writeStore(items: Conversation[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(items.slice(0, MAX_CONVERSATIONS)));
}

export function createEmptyConversation(model: string): Conversation {
  return {
    id: newId('conv'),
    title: 'New chat',
    model,
    messages: [],
    updatedAt: Date.now(),
  };
}

export function useConversationStore(defaultModel: string) {
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [hydrated, setHydrated] = useState(false);

  useEffect(() => {
    const items = readStore();
    if (items.length === 0) {
      const fresh = createEmptyConversation(defaultModel);
      setConversations([fresh]);
      setActiveId(fresh.id);
    } else {
      setConversations(items);
      setActiveId(items[0]!.id);
    }
    setHydrated(true);
    // Hydrate once on mount; defaultModel is only a seed for an empty store.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!hydrated) return;
    writeStore(conversations);
  }, [conversations, hydrated]);

  const active = conversations.find((c) => c.id === activeId) ?? null;

  const updateActive = useCallback(
    (fn: (c: Conversation) => Conversation) => {
      setConversations((prev) =>
        prev.map((c) => (c.id === activeId ? { ...fn(c), updatedAt: Date.now() } : c)),
      );
    },
    [activeId],
  );

  const newConversation = useCallback(
    (model: string) => {
      const fresh = createEmptyConversation(model);
      setConversations((prev) => [fresh, ...prev].slice(0, MAX_CONVERSATIONS));
      setActiveId(fresh.id);
      return fresh;
    },
    [],
  );

  const deleteConversation = useCallback(
    (id: string) => {
      setConversations((prev) => {
        const next = prev.filter((c) => c.id !== id);
        if (next.length === 0) {
          const fresh = createEmptyConversation(defaultModel);
          setActiveId(fresh.id);
          return [fresh];
        }
        if (activeId === id) {
          setActiveId(next[0]!.id);
        }
        return next;
      });
    },
    [activeId, defaultModel],
  );

  return {
    conversations,
    active,
    activeId,
    setActiveId,
    updateActive,
    newConversation,
    deleteConversation,
    hydrated,
  };
}
