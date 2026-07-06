<script lang="ts">
  import { applyTheme } from '../lib/stores/theme';
  import { themes } from '../lib/themes';
  import type { AppSettings } from '../lib/stores/settings';
  import '../lib/settings-general.css';

  // `settings` is the parent's reactive localSettings proxy — mutating its
  // fields here propagates to the parent (which owns save).
  let { settings, showHelp }: { settings: AppSettings; showHelp: (key: string) => void } = $props();

  let hotkeyListening = $state(false);
  let hotkeyError = $state('');

  function onThemeChange(e: Event) {
    const key = (e.target as HTMLSelectElement).value;
    settings.theme = key;
    applyTheme(key);
  }

  function mapKeyCode(code: string): string | null {
    if (/^Key([A-Z])$/.test(code)) return code[3];
    if (/^Digit(\d)$/.test(code)) return code[5];
    if (/^F(\d{1,2})$/.test(code)) return code;
    if (/^Numpad(\d)$/.test(code)) return 'Num' + code[6];
    const map: Record<string, string> = {
      Space: 'Space', Enter: 'Enter', Backspace: 'Backspace', Tab: 'Tab',
      Escape: 'Escape', Delete: 'Delete', Insert: 'Insert', Home: 'Home',
      End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', PrintScreen: 'PrintScreen',
      ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
      Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
      Semicolon: ';', Quote: "'", Backquote: '`', Backslash: '\\',
      Comma: ',', Period: '.', Slash: '/',
      NumpadAdd: 'NumAdd', NumpadSubtract: 'NumSub', NumpadMultiply: 'NumMul',
      NumpadDivide: 'NumDiv', NumpadDecimal: 'NumDec', NumpadEnter: 'NumEnter',
    };
    return map[code] ?? null;
  }

  // Keys the Rust parse_hotkey actually accepts — validate here for immediate feedback (3.5).
  const HOTKEY_SUPPORTED_KEYS = new Set<string>([
    ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split(''),
    '0','1','2','3','4','5','6','7','8','9',
    'F1','F2','F3','F4','F5','F6','F7','F8','F9','F10','F11','F12',
    'SPACE','ENTER','TAB','PRINTSCREEN','INSERT','DELETE','HOME','END','PAGEUP','PAGEDOWN',
  ]);

  function handleHotkeyKeydown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;
    const key = mapKeyCode(e.code);
    if (!key) return;
    if (!HOTKEY_SUPPORTED_KEYS.has(key.toUpperCase())) {
      hotkeyError = `"${key}" can't be used as the capture key. Pick a letter, number, F1–F12, or Space/Enter/Tab/Ins/Del/Home/End/PgUp/PgDn/PrintScreen.`;
      return;
    }
    const parts: string[] = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    const isSpecial = /^F\d{1,2}$/.test(key) || key === 'PrintScreen';
    if (parts.length === 0 && !isSpecial) {
      hotkeyError = 'A modifier key (Ctrl, Alt, Shift) is required';
      return;
    }
    parts.push(key);
    settings.captureHotkey = parts.join('+');
    hotkeyError = '';
    hotkeyListening = false;
  }

  function toggleHotkeyCapture() {
    hotkeyListening = !hotkeyListening;
    hotkeyError = '';
  }

  function cancelHotkeyCapture() {
    hotkeyListening = false;
    hotkeyError = '';
  }
</script>

<fieldset class="group-box">
  <legend>General</legend>
  <div class="general-layout">
    <div class="general-left">
      <div class="cb-row">
        <label class="cb-label">
          <input type="checkbox" bind:checked={settings.autoclose} />
          <span>Autoclose in 30 seconds</span>
        </label>
        <button class="help-btn" onclick={() => showHelp('autoclose')}>?</button>
      </div>
      <div class="cb-row">
        <label class="cb-label">
          <input type="checkbox" bind:checked={settings.escapeHidesResults} />
          <span>Escape hides Results window</span>
        </label>
        <button class="help-btn" onclick={() => showHelp('escapeHides')}>?</button>
      </div>
      <div class="cb-row">
        <label class="cb-label">
          <input type="checkbox" bind:checked={settings.autorun} />
          <span>Add to Autorun</span>
        </label>
        <button class="help-btn" onclick={() => showHelp('autorun')}>?</button>
      </div>
      <div class="cb-row">
        <label class="cb-label">
          <input type="checkbox" bind:checked={settings.loggingOn} />
          <span>Write to Log File</span>
        </label>
        <button class="help-btn" onclick={() => showHelp('logging')}>?</button>
      </div>
      <div class="cb-row">
        <label class="cb-label">
          <input type="checkbox" bind:checked={settings.skipUploadInCopyMode} />
          <span>Skip upload in Copy Image mode</span>
        </label>
        <button class="help-btn" onclick={() => showHelp('skipUpload')}>?</button>
      </div>
    </div>
    <div class="general-right">
      <div class="control-stack">
        <span class="control-label">Theme <button class="help-btn" onclick={() => showHelp('theme')}>?</button></span>
        <select value={settings.theme} onchange={onThemeChange} class="dropdown-select">
          {#each Object.entries(themes) as [key, theme]}
            <option value={key}>{theme.name}</option>
          {/each}
        </select>
      </div>
      <div class="control-stack">
        <span class="control-label">Default Mode <button class="help-btn" onclick={() => showHelp('defaultMode')}>?</button></span>
        <select bind:value={settings.defaultMode} class="dropdown-select">
          <option value="image">Green: Copy Image</option>
          <option value="link">Pink: Copy Link</option>
        </select>
      </div>
      <div class="control-stack">
        <span class="control-label">Capture Hotkey <button class="help-btn" onclick={() => showHelp('hotkey')}>?</button></span>
        {#if hotkeyListening}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="hotkey-input listening"
            readonly
            autofocus
            placeholder="Press keys... (click to cancel)"
            onkeydown={handleHotkeyKeydown}
            onclick={cancelHotkeyCapture}
          />
        {:else}
          <button class="hotkey-input" onclick={toggleHotkeyCapture}>
            {settings.captureHotkey}
          </button>
        {/if}
        {#if hotkeyError}
          <span class="hotkey-error">{hotkeyError}</span>
        {/if}
      </div>
      <div class="control-stack">
        <span class="control-label">File name prefix <button class="help-btn" onclick={() => showHelp('imagePrefix')}>?</button></span>
        <input class="dropdown-select" type="text" bind:value={settings.imagePrefix} placeholder="cta_" />
      </div>
      <div class="control-stack">
        <span class="control-label">Shared image size (DPI) <button class="help-btn" onclick={() => showHelp('outputMode')}>?</button></span>
        <select bind:value={settings.outputMode} class="dropdown-select">
          <option value="off">Full resolution (largest, looks big in a browser)</option>
          <option value="resize">Resize to logical size (smaller, slightly soft)</option>
          <option value="exif">Full-res + EXIF density (crisp in browsers)</option>
        </select>
      </div>
      <div class="control-stack">
        <span class="control-label">JPEG quality: {settings.jpegQuality}%</span>
        <input
          type="range"
          min="50"
          max="100"
          step="1"
          bind:value={settings.jpegQuality}
          style="width: 100%; accent-color: var(--accent);"
          aria-label="JPEG quality"
        />
        <span style="font-size: 8pt; opacity: 0.6; color: var(--text-dim);">Only affects the uploaded/shared file. Capture &amp; editing stay lossless.</span>
      </div>
    </div>
  </div>
</fieldset>
