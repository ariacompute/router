/** Static style guard for the impeccable audit (primary rules). */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (rel: string) => readFileSync(resolve(process.cwd(), rel), 'utf-8');

const stylesheets = [
  'src/index.css',
  'src/App.module.css',
  'src/pages/page.module.css',
  'src/components/playground/playground.module.css',
].map((p) => [p, read(p)] as const);
const allCss = stylesheets.map(([, text]) => text).join('\n');
const indexHtml = read('index.html');
const config = JSON.parse(read('.impeccable/config.json')) as {
  engineMap: Record<string, string>;
  dom: { routes: string[]; themes: string[] };
};

function tokenBlock(css: string, selector: string): string {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`missing token block ${selector}`);
  const open = css.indexOf('{', start);
  const end = css.indexOf('}', open);
  return css.slice(open + 1, end);
}

function token(block: string, name: string): string {
  const m = new RegExp(`${name}\\s*:\\s*([^;]+);`).exec(block);
  if (!m) throw new Error(`missing token ${name}`);
  return m[1].trim();
}

function luminance(hex: string): number {
  const h = hex.replace('#', '');
  const full = h.length === 3 ? h.split('').map((c) => c + c).join('') : h;
  const channels = [0, 2, 4].map((i) => {
    const v = parseInt(full.slice(i, i + 2), 16) / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(a: string, b: string): number {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

describe('static design tokens', () => {
  it('uses zero radii and no elevation', () => {
    const root = tokenBlock(allCss, ':root');
    for (const match of root.matchAll(/(--radius[\w-]*)\s*:\s*([^;]+);/g)) {
      expect(match[2].trim(), match[1]).toBe('0');
    }
    expect(token(root, '--shadow')).toBe('none');
    expect(token(root, '--shadow-sm')).toBe('none');
    expect(token(root, '--font-size-min')).toBe('12px');
  });

  it('is monochrome: no gradients, uppercase or letter spacing', () => {
    expect(allCss).not.toMatch(/linear-gradient|radial-gradient|conic-gradient/i);
    expect(allCss).not.toMatch(/text-transform\s*:\s*uppercase/i);
    expect(allCss).not.toMatch(/letter-spacing\s*:/i);
  });

  it('animates only transform and opacity', () => {
    for (const m of allCss.matchAll(/transition\s*:\s*([^;]+);/g)) {
      for (const part of m[1].split(',')) {
        const property = part.trim().split(/\s+/)[0];
        expect(['transform', 'opacity']).toContain(property);
      }
    }
    for (const m of allCss.matchAll(/animation\s*:\s*([^;]+);/g)) {
      expect(m[1]).not.toMatch(/background|color|border/i);
    }
  });

  it('never renders text below 12px', () => {
    for (const m of allCss.matchAll(/font-size\s*:\s*([\d.]+)(rem|px)/g)) {
      const px = m[2] === 'rem' ? Number(m[1]) * 16 : Number(m[1]);
      expect(px, `font-size ${m[0]}`).toBeGreaterThanOrEqual(12);
    }
  });

  it('keeps AA contrast for every text token in both themes', () => {
    for (const selector of [':root', 'html[data-theme="dark"]']) {
      const block = tokenBlock(allCss, selector);
      const bg = token(block, '--page-bg');
      const panel = token(block, '--panel-bg');
      for (const name of ['--text-main', '--text-muted', '--text-soft']) {
        const fg = token(block, name);
        expect(contrast(fg, bg), `${selector} ${name} on page`).toBeGreaterThanOrEqual(4.5);
        expect(contrast(fg, panel), `${selector} ${name} on panel`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it('uses a local font stack without a web-font CDN', () => {
    const root = tokenBlock(allCss, ':root');
    for (const name of ['--font-sans', '--font-display', '--font-label']) {
      expect(token(root, name)).not.toMatch(/Inter|Chakra|Newsreader|fonts\.googleapis/i);
    }
    expect(indexHtml).not.toMatch(/fonts\.googleapis\.com|fonts\.gstatic\.com|@import\s+url\(https?:/i);
    expect(indexHtml).toMatch(/<html lang="[a-z-]+"/);
    expect(indexHtml).toMatch(/color-scheme/);
  });
});

describe('impeccable config', () => {
  it('maps TSX to DOM and covers all routes in both themes', () => {
    expect(config.engineMap['.tsx']).toBe('dom');
    expect(config.engineMap['.css']).toBe('css');
    expect(config.dom.themes).toEqual(['light', 'dark']);
    expect(config.dom.routes.length).toBe(15);
    expect(new Set(config.dom.routes).size).toBe(15);
    for (const route of ['/', '/model-performance', '/trust', '/playground']) {
      expect(config.dom.routes).toContain(route);
    }
  });
});
