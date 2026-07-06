/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  theme: {
    extend: {
      colors: {
        'bg-base': 'var(--bg-base)',
        'bg-surface': 'var(--bg-surface)',
        'bg-input': 'var(--bg-input)',
        'accent': 'var(--accent)',
        'accent-hover': 'var(--accent-hover)',
        'accent-dim': 'var(--accent-dim)',
        'text-main': 'var(--text-main)',
        'text-dim': 'var(--text-dim)',
        'border': 'var(--border)',
        'bg-toolbar': 'var(--bg-toolbar)'
      },
      fontFamily: {
        'segoe': ['"Segoe UI"', 'system-ui', 'sans-serif']
      }
    }
  },
  plugins: []
};
