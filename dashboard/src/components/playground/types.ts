export type ChatRole = 'user' | 'assistant';

export type PlaygroundMessage = {
  id: string;
  role: ChatRole;
  content: string;
  headers?: Record<string, string>;
  streaming?: boolean;
  error?: string;
};

export type Conversation = {
  id: string;
  title: string;
  model: string;
  messages: PlaygroundMessage[];
  updatedAt: number;
};

export function newId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 10)}${Date.now().toString(36)}`;
}

export function titleFromPrompt(prompt: string): string {
  const t = prompt.trim().replace(/\s+/g, ' ');
  if (!t) return 'New chat';
  return t.length > 42 ? `${t.slice(0, 42)}…` : t;
}
