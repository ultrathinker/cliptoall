import { themes } from '../themes';

export function applyTheme(themeKey: string) {
  // Fall back to the app-wide default theme (matches the Rust AppSettings
  // default) when an unknown key is passed, so first-run/invalid values render
  // the same theme the backend would persist.
  const theme = themes[themeKey] || themes.crimson;

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
