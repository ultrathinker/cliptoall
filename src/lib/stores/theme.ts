import { writable } from 'svelte/store';
import { themes, type Theme } from '../themes';

export const currentTheme = writable<Theme>(themes.classic);

export function applyTheme(themeKey: string) {
  const theme = themes[themeKey] || themes.classic;
  currentTheme.set(theme);
  
  const root = document.documentElement;
  root.style.setProperty('--bg-base', theme.bgBase);
  root.style.setProperty('--bg-surface', theme.bgSurface);
  root.style.setProperty('--bg-input', theme.bgInput);
  root.style.setProperty('--accent', theme.accent);
  root.style.setProperty('--accent-hover', theme.accentHover);
  root.style.setProperty('--accent-dim', theme.accentDim);
  root.style.setProperty('--text-main', theme.textMain);
  root.style.setProperty('--text-dim', theme.textDim);
  root.style.setProperty('--border', theme.border);
  root.style.setProperty('--bg-toolbar', theme.bgToolbar);
}
