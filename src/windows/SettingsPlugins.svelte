<script lang="ts">
  import { onMount } from 'svelte';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import {
    discoverPlugins, loadPluginConfigs, readScript, saveScript,
    precompileScript, runScript, runScriptInTerminal, deleteScript, checkRuntime,
  } from '../lib/api';
  import type { DiscoveredPlugin, PluginConfig } from '../lib/plugin-types';
  import { pythonTemplate, csharpTemplate, powershellTemplate, getAiInstructions } from '../lib/plugin-templates';
  import '../lib/settings-plugins.css';

  // The three fields the parent's handleSave needs are $bindable so they survive
  // this component's unmount when the user switches away from the Plugins tab,
  // and stay readable by the parent for validation + persistence.
  let {
    pluginConfigs = $bindable([]),
    discoveredPlugins = $bindable([]),
    pluginsScanned = $bindable(false),
  }: {
    pluginConfigs: PluginConfig[];
    discoveredPlugins: DiscoveredPlugin[];
    pluginsScanned: boolean;
  } = $props();

  let pluginsLoading = $state(false);
  let bindingTarget = $state<{ path: string; funcId: string } | null>(null);
  // Transient hint shown while listening when the user presses a key the
  // overlay can't map (only A–Z / 0–9 are valid shortcuts). Cleared on start.
  let bindingHint = $state('');
  let instructionTarget = $state<DiscoveredPlugin | null>(null);
  let settingsTarget = $state<{ plugin: DiscoveredPlugin; config: PluginConfig } | null>(null);
  let settingsEditValue = $state('');
  let settingsError = $state('');
  let settingsRevealed = $state(false);

  // ── Plugin toolbar state ─────────────────────────────────────
  let pluginSort = $state<'added' | 'name'>('added');
  let pluginSearch = $state('');

  // ── Script plugin state ───────────────────────────────────────
  let scriptEditorOpen = $state(false);
  let scriptEditorName = $state('');
  let scriptEditorLanguage = $state<'python' | 'csharp' | 'powershell'>('python');
  let scriptEditorContent = $state('');
  let scriptEditorError = $state('');
  let scriptEditorPath = $state<string | null>(null); // null = new, string = editing existing
  let consoleOutput = $state<Record<string, string>>({});
  let consoleVisible = $state<Record<string, boolean>>({});
  let scriptRunning = $state<Record<string, boolean>>({});
  let runtimeStatus = $state<Record<string, { available: boolean; version: string }>>({});
  let aiInstructionsOpen = $state(false);

  let filteredPlugins = $derived.by(() => {
    let list = discoveredPlugins;

    // Search filter (case-insensitive, matches name, description, filename)
    const q = pluginSearch.trim().toLowerCase();
    if (q) {
      list = list.filter(p => {
        const filename = p.path.split(/[/\\]/).pop()?.toLowerCase() || '';
        return p.name?.toLowerCase().includes(q)
          || p.description?.toLowerCase().includes(q)
          || filename.includes(q);
      });
    }

    // Sort
    if (pluginSort === 'name') {
      list = [...list].sort((a, b) => (a.name || '').localeCompare(b.name || ''));
    }
    // 'added' = original discovery order (default)

    return list;
  });

  // Scan on first activation. Because pluginsScanned is bindable and lives in the
  // parent, it persists across tab switches — re-entering the tab won't rescan.
  onMount(() => {
    if (!pluginsScanned) scanPlugins();
  });

  // ── Plugin functions ────────────────────────────────────────────

  async function scanPlugins() {
    pluginsLoading = true;
    try {
      const [discovered, configs] = await Promise.all([
        discoverPlugins(),
        loadPluginConfigs(),
      ]);
      discoveredPlugins = discovered;
      pluginConfigs = configs;
      pluginsScanned = true;

      // Check runtimes if any script plugins discovered
      if (discovered.some(p => p.plugin_type === 'python' || p.plugin_type === 'csharp' || p.plugin_type === 'powershell')) {
        checkRuntimes();
      }
    } catch (e) {
      console.error('Plugin scan failed:', e);
    } finally {
      pluginsLoading = false;
    }
  }

  function getPluginConfig(path: string): PluginConfig | undefined {
    return pluginConfigs.find(c => c.path === path);
  }

  function isPluginEnabled(path: string): boolean {
    return getPluginConfig(path)?.enabled ?? false;
  }

  function getKeyBinding(path: string, funcId: string, defaultKey: string): string {
    const cfg = getPluginConfig(path);
    const myKey = (cfg?.keyBindings[funcId] ?? defaultKey).toUpperCase();

    // Only check conflicts for enabled plugins
    if (!cfg?.enabled) return myKey;

    // First-come-first-served: build key set in discoveredPlugins order,
    // stop when we reach our own function (matches Rust backend behavior)
    const taken = new Set<string>();
    for (const plugin of discoveredPlugins) {
      const pcfg = getPluginConfig(plugin.path);
      if (!pcfg?.enabled) continue;
      for (const fn2 of plugin.functions) {
        if (plugin.path === path && fn2.id === funcId) {
          return taken.has(myKey) ? 'SETUP' : myKey;
        }
        const key = (pcfg.keyBindings[fn2.id] ?? fn2.default_key).toUpperCase();
        taken.add(key);
      }
    }
    return myKey;
  }

  function togglePlugin(path: string, plugin: DiscoveredPlugin) {
    const existing = pluginConfigs.findIndex(c => c.path === path);
    if (existing >= 0) {
      pluginConfigs[existing] = { ...pluginConfigs[existing], enabled: !pluginConfigs[existing].enabled };
    } else {
      const bindings: Record<string, string> = {};
      for (const fn of plugin.functions) {
        bindings[fn.id] = fn.default_key;
      }
      pluginConfigs = [...pluginConfigs, { path, enabled: true, keyBindings: bindings, settings: '' }];
    }
  }

  function startKeyBinding(path: string, funcId: string) {
    bindingTarget = { path, funcId };
    bindingHint = '';
  }

  function handleBindingKeydown(e: KeyboardEvent) {
    if (!bindingTarget) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === 'Escape') {
      bindingTarget = null;
      bindingHint = '';
      return;
    }

    // Ignore pure modifier presses
    if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;

    const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;

    // The overlay maps plugin shortcuts by single virtual-key code and only
    // knows A–Z / 0–9 (see key_string_to_vk in overlay.rs). Binding anything
    // else (F-keys, arrows, punctuation) would be saved but silently never
    // fire. Reject those here and keep listening + surface a hint, instead of
    // storing a dead binding.
    if (!/^[A-Z0-9]$/.test(key)) {
      bindingHint = 'Only A–Z or 0–9 can be used as a shortcut';
      return;
    }
    bindingHint = '';

    const { path, funcId } = bindingTarget;

    // Reject if key is already in use by another enabled plugin function
    for (const plugin of discoveredPlugins) {
      const pcfg = getPluginConfig(plugin.path);
      if (!pcfg?.enabled) continue;
      for (const fn2 of plugin.functions) {
        if (plugin.path === path && fn2.id === funcId) continue;
        const otherKey = (pcfg.keyBindings[fn2.id] ?? fn2.default_key).toUpperCase();
        if (otherKey === key) return; // key taken — keep listening
      }
    }

    const cfgIdx = pluginConfigs.findIndex(c => c.path === path);
    if (cfgIdx >= 0) {
      pluginConfigs[cfgIdx] = {
        ...pluginConfigs[cfgIdx],
        keyBindings: { ...pluginConfigs[cfgIdx].keyBindings, [funcId]: key },
      };
    }
    bindingTarget = null;
  }

  function showInstruction(plugin: DiscoveredPlugin) {
    instructionTarget = plugin;
  }

  function openSettingsEditor(plugin: DiscoveredPlugin) {
    const cfg = getPluginConfig(plugin.path);
    const config = cfg ?? { path: plugin.path, enabled: false, keyBindings: {}, settings: '' };
    settingsEditValue = config.settings || plugin.settings_format;
    settingsError = '';
    settingsRevealed = !config.settings;
    settingsTarget = { plugin, config };
  }

  function savePluginSettings() {
    if (!settingsTarget) return;
    if (settingsEditValue.trim()) {
      try {
        JSON.parse(settingsEditValue);
      } catch (e) {
        settingsError = 'Invalid JSON: ' + (e as Error).message;
        return;
      }
    }
    const cfgIdx = pluginConfigs.findIndex(c => c.path === settingsTarget!.plugin.path);
    if (cfgIdx >= 0) {
      pluginConfigs[cfgIdx] = { ...pluginConfigs[cfgIdx], settings: settingsEditValue.trim() };
    } else {
      pluginConfigs = [...pluginConfigs, {
        path: settingsTarget.plugin.path,
        enabled: false,
        keyBindings: {},
        settings: settingsEditValue.trim(),
      }];
    }
    settingsTarget = null;
  }

  async function handleCopyAiInstructions() {
    try {
      await writeText(getAiInstructions(scriptEditorLanguage));
    } catch {
      await navigator.clipboard.writeText(getAiInstructions(scriptEditorLanguage));
    }
  }

  function handleAddScript() {
    scriptEditorName = '';
    scriptEditorLanguage = 'python';
    scriptEditorContent = pythonTemplate('MyScript');
    scriptEditorError = '';
    scriptEditorPath = null;
    scriptEditorOpen = true;
    // Check runtimes if not already checked
    if (!runtimeStatus['python'] && !runtimeStatus['csharp'] && !runtimeStatus['powershell']) {
      checkRuntimes();
    }
  }

  async function handleEditScript(plugin: DiscoveredPlugin) {
    try {
      const content = await readScript(plugin.path);
      const filename = plugin.path.split(/[/\\]/).pop() || '';
      const ext = filename.split('.').pop();
      scriptEditorName = filename.replace(/\.(py|cs|ps1)$/, '');
      scriptEditorLanguage = ext === 'cs' ? 'csharp' : ext === 'ps1' ? 'powershell' : 'python';
      scriptEditorContent = content;
      scriptEditorError = '';
      scriptEditorPath = plugin.path;
      scriptEditorOpen = true;
    } catch (e) {
      alert('Failed to read script: ' + e);
    }
  }

  async function handleSaveScript() {
    if (!scriptEditorName.trim()) {
      scriptEditorError = 'Name is required';
      return;
    }
    try {
      const savedPath = await saveScript(
        scriptEditorName,
        scriptEditorLanguage,
        scriptEditorContent,
        scriptEditorPath !== null,
      );

      const isCSharp = scriptEditorLanguage === 'csharp';
      scriptEditorOpen = false;
      await scanPlugins();

      // Pre-compile C# in background (non-blocking — modal already closed)
      if (isCSharp) {
        consoleOutput = { ...consoleOutput, [savedPath]: 'Compiling...' };
        consoleVisible = { ...consoleVisible, [savedPath]: true };
        try {
          await precompileScript(savedPath);
          consoleOutput = { ...consoleOutput, [savedPath]: 'Compiled successfully' };
        } catch (e) {
          consoleOutput = { ...consoleOutput, [savedPath]: 'Compilation error: ' + e };
        }
      }
    } catch (e) {
      scriptEditorError = 'Failed to save: ' + String(e);
    }
  }

  async function handleRunScript(plugin: DiscoveredPlugin) {
    scriptRunning = { ...scriptRunning, [plugin.path]: true };
    consoleVisible = { ...consoleVisible, [plugin.path]: true };
    try {
      const output = await runScript(plugin.path);
      consoleOutput = { ...consoleOutput, [plugin.path]: output };
    } catch (e) {
      consoleOutput = { ...consoleOutput, [plugin.path]: 'ERROR: ' + String(e) };
    } finally {
      scriptRunning = { ...scriptRunning, [plugin.path]: false };
    }
  }

  async function handleRunInTerminal(plugin: DiscoveredPlugin) {
    try {
      await runScriptInTerminal(plugin.path);
    } catch (e) {
      alert('Failed to open PowerShell: ' + e);
    }
  }

  async function handleDeleteScript(plugin: DiscoveredPlugin) {
    if (!confirm(`Delete script "${plugin.name}"?\nFile: ${plugin.path.split(/[/\\]/).pop()}`)) return;
    try {
      await deleteScript(plugin.path);
      // Remove from configs if present
      pluginConfigs = pluginConfigs.filter(c => c.path !== plugin.path);
      await scanPlugins();
    } catch (e) {
      alert('Failed to delete: ' + e);
    }
  }

  function toggleConsole(path: string) {
    consoleVisible = { ...consoleVisible, [path]: !consoleVisible[path] };
  }

  async function checkRuntimes() {
    for (const lang of ['python', 'csharp', 'powershell'] as const) {
      try {
        const version = await checkRuntime(lang);
        runtimeStatus = { ...runtimeStatus, [lang]: { available: true, version } };
      } catch (_) {
        runtimeStatus = { ...runtimeStatus, [lang]: { available: false, version: '' } };
      }
    }
  }

  function onScriptLanguageChange() {
    // Only auto-fill template if creating new (not editing existing)
    if (!scriptEditorPath) {
      const name = scriptEditorName || 'MyScript';
      scriptEditorContent = scriptEditorLanguage === 'csharp' ? csharpTemplate(name) :
        scriptEditorLanguage === 'powershell' ? powershellTemplate(name) : pythonTemplate(name);
    }
  }

  function isScriptPlugin(plugin: DiscoveredPlugin): boolean {
    return plugin.plugin_type === 'python' || plugin.plugin_type === 'csharp' || plugin.plugin_type === 'powershell';
  }

  function pluginTypeBadge(plugin: DiscoveredPlugin): { label: string; cls: string } {
    switch (plugin.plugin_type) {
      case 'python': return { label: 'PY', cls: 'badge-py' };
      case 'csharp': return { label: 'C#', cls: 'badge-cs' };
      case 'powershell': return { label: 'PS', cls: 'badge-ps' };
      default: return { label: 'EXE', cls: 'badge-exe' };
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="plugins-panel" onkeydown={bindingTarget ? handleBindingKeydown : undefined}>

  <!-- Toolbar: Add Script, Sort, Search -->
  <div class="plugins-toolbar">
    <div class="plugins-toolbar-row">
      <button class="btn-small btn-add-script" onclick={handleAddScript}>+ Add Script</button>
      <select class="plugins-sort-select" bind:value={pluginSort}>
        <option value="added">By date added</option>
        <option value="name">By name</option>
      </select>
      <input class="plugins-search" type="text" placeholder="Search plugins..." bind:value={pluginSearch} />
    </div>
    <p class="plugins-hint">Place plugins in the <code>plugins/</code> folder next to ClipToAll.exe</p>
  </div>

  {#if pluginsLoading}
    <div class="plugins-status">Scanning for plugins...</div>
  {:else if discoveredPlugins.length === 0}
    <div class="plugins-status">
      <p>No plugin files found.</p>
    </div>
  {:else if filteredPlugins.length === 0}
    <div class="plugins-status">
      <p>No plugins match "{pluginSearch}"</p>
    </div>
  {:else}
    {#each filteredPlugins as plugin}
      {@const badge = pluginTypeBadge(plugin)}
      {@const isScript = isScriptPlugin(plugin)}
      {@const runtimeKey = plugin.plugin_type === 'csharp' ? 'csharp' : plugin.plugin_type === 'powershell' ? 'powershell' : 'python'}
      {@const runtimeOk = !isScript || runtimeStatus[runtimeKey]?.available !== false}
      {@const enabled = isPluginEnabled(plugin.path)}
      <div class="plugin-card" class:invalid={!plugin.valid}>
        <div class="plugin-header">
          <div class="plugin-info">
            <span class="plugin-exe">
              <span class="plugin-badge {badge.cls}">{badge.label}</span>
              {plugin.path.split(/[/\\]/).pop()}
              {#if isScript && plugin.mode}
                <span class="plugin-mode">{plugin.mode}</span>
              {/if}
            </span>
            {#if plugin.valid}
              <span class="plugin-name">{plugin.name} <span class="plugin-ver">v{plugin.version}</span></span>
            {/if}
            {#if isScript && runtimeStatus[runtimeKey]?.available === false}
              <span class="runtime-warn">{runtimeKey === 'csharp' ? '.NET SDK' : runtimeKey === 'powershell' ? 'PowerShell' : 'Python'} not found in PATH</span>
            {/if}
          </div>
          {#if plugin.valid}
            <div class="plugin-header-right">
              {#if isScript}
                <button class="plugin-link" onclick={() => handleRunScript(plugin)} disabled={scriptRunning[plugin.path] || !runtimeOk}>
                  {scriptRunning[plugin.path] ? 'Running...' : 'Run'}
                </button>
                <button class="plugin-link" onclick={() => handleRunInTerminal(plugin)} disabled={!runtimeOk} title="Open in PowerShell (interactive, stays open)">PS</button>
                <button class="plugin-link" onclick={() => handleEditScript(plugin)}>Edit</button>
                <button class="plugin-link plugin-link-danger" onclick={() => handleDeleteScript(plugin)}>Del</button>
              {/if}
              {#if plugin.settings_format}
                <button class="plugin-link" onclick={() => openSettingsEditor(plugin)}>
                  Settings
                  {#if enabled && !(getPluginConfig(plugin.path)?.settings?.trim())}
                    <span class="settings-warn">!</span>
                  {/if}
                </button>
              {/if}
              <button
                class="toggle-btn"
                class:on={enabled}
                onclick={() => togglePlugin(plugin.path, plugin)}
              >
                {enabled ? 'ON' : 'OFF'}
              </button>
            </div>
          {/if}
        </div>

        {#if !plugin.valid}
          <div class="plugin-error">Not a valid plugin: {plugin.error || 'no hello response'}</div>
        {:else}
          <div class="plugin-desc-row">
            {#if plugin.instruction}
              <span class="plugin-desc-preview">{plugin.instruction.length > 350 ? plugin.instruction.slice(0, 350) + '...' : plugin.instruction}</span>
              <button class="plugin-link" onclick={() => showInstruction(plugin)}>show more</button>
            {:else if plugin.description}
              <span class="plugin-desc-preview">{plugin.description}</span>
            {:else}
              <span class="plugin-desc-preview dim">There is no description</span>
            {/if}
          </div>
        {/if}

        {#if plugin.valid && plugin.functions.length > 0}
          <div class="plugin-functions" class:disabled={!enabled}>
            <div class="func-header">
              <span class="func-header-label">Skill</span>
              <span class="func-header-label">Shortcut</span>
            </div>
            {#each plugin.functions as fn}
              {@const keyLabel = getKeyBinding(plugin.path, fn.id, fn.default_key)}
              <div class="func-row">
                <span class="func-label">{fn.label}</span>
                {#if enabled}
                  {@const listening = bindingTarget?.path === plugin.path && bindingTarget?.funcId === fn.id}
                  <span class="key-cell">
                    {#if listening}
                      <span class="binding-hint" class:binding-hint-warn={bindingHint}>
                        {bindingHint || 'Press A–Z or 0–9'}
                      </span>
                    {/if}
                    <button
                      class="key-btn"
                      class:listening
                      class:setup={keyLabel === 'SETUP'}
                      onclick={() => startKeyBinding(plugin.path, fn.id)}
                    >
                      {#if listening}
                        ...
                      {:else}
                        {keyLabel}
                      {/if}
                    </button>
                  </span>
                {:else}
                  <span class="key-btn disabled">{keyLabel}</span>
                {/if}
              </div>
            {/each}
          </div>
        {/if}

        {#if isScript && consoleOutput[plugin.path] != null}
          <div class="console-panel">
            <div class="console-header">
              <button class="plugin-link" onclick={() => toggleConsole(plugin.path)}>
                {consoleVisible[plugin.path] ? 'Hide output' : 'Show output'}
              </button>
              <button class="plugin-link" onclick={() => { const { [plugin.path]: _1, ...restOut } = consoleOutput; consoleOutput = restOut; const { [plugin.path]: _2, ...restVis } = consoleVisible; consoleVisible = restVis; }}>Clear</button>
            </div>
            {#if consoleVisible[plugin.path]}
              <pre class="console-output">{consoleOutput[plugin.path]}</pre>
            {/if}
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</div>

{#if instructionTarget}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1" onclick={() => instructionTarget = null} onkeydown={(e) => { if (e.key === 'Escape') instructionTarget = null; }}>
    <div class="help-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      <div class="help-title">{instructionTarget.name}</div>
      <div class="help-body">
        {#each instructionTarget.instruction.split('\n') as line}
          {#if line === ''}<br />{:else}<p>{line}</p>{/if}
        {/each}
      </div>
      <div class="help-footer">
        <button class="btn-default" onclick={() => instructionTarget = null}>Close</button>
      </div>
    </div>
  </div>
{/if}

{#if settingsTarget}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1" onclick={() => settingsTarget = null} onkeydown={(e) => { if (e.key === 'Escape') settingsTarget = null; }}>
    <div class="help-popup settings-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      <div class="help-title">{settingsTarget.plugin.name} — Settings</div>
      <div class="help-body">
        {#if settingsTarget.plugin.settings_description}
          <p class="settings-desc">{settingsTarget.plugin.settings_description}</p>
        {/if}
        <p class="settings-format-label">Expected format:</p>
        <pre class="settings-format">{settingsTarget.plugin.settings_format}</pre>
        {#if settingsRevealed}
          <textarea
            class="settings-textarea"
            bind:value={settingsEditValue}
            rows="4"
            placeholder="Paste your settings JSON here..."
          ></textarea>
          <button class="btn-link settings-reveal-btn" onclick={() => settingsRevealed = false}>Hide</button>
        {:else}
          <div class="settings-masked">
            <span class="settings-masked-dots">{'●'.repeat(Math.min(settingsEditValue.length || 8, 24))}</span>
            <button class="btn-link settings-reveal-btn" onclick={() => settingsRevealed = true}>Show</button>
          </div>
        {/if}
        {#if settingsError}
          <span class="settings-error">{settingsError}</span>
        {/if}
      </div>
      <div class="help-footer">
        <button class="btn-accent" onclick={savePluginSettings}>Apply</button>
        <button class="btn-default" onclick={() => settingsTarget = null}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if scriptEditorOpen}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1" onclick={() => scriptEditorOpen = false} onkeydown={(e) => { if (e.key === 'Escape') scriptEditorOpen = false; }}>
    <div class="help-popup script-editor-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      <div class="help-title">{scriptEditorPath ? 'Edit Script' : 'New Script'}</div>
      <div class="help-body">
        <div class="script-editor-fields">
          <div class="script-field-row">
            <label class="script-field-label" for="script-name">Name:</label>
            <input
              id="script-name"
              class="field-input flex1"
              bind:value={scriptEditorName}
              placeholder="my-script"
              disabled={!!scriptEditorPath}
            />
          </div>
          <div class="script-field-row">
            <label class="script-field-label" for="script-language">Language:</label>
            <div class="script-lang-row">
              <select
                id="script-language"
                class="dropdown-select script-lang-select"
                bind:value={scriptEditorLanguage}
                onchange={onScriptLanguageChange}
                disabled={!!scriptEditorPath}
              >
                <option value="python">Python</option>
                <option value="csharp">C# (.NET)</option>
                <option value="powershell">PowerShell</option>
              </select>
              {#if runtimeStatus[scriptEditorLanguage]}
                {#if runtimeStatus[scriptEditorLanguage].available}
                  <span class="runtime-ok">{runtimeStatus[scriptEditorLanguage].version}</span>
                {:else}
                  <span class="runtime-warn">{scriptEditorLanguage === 'csharp' ? '.NET SDK' : scriptEditorLanguage === 'powershell' ? 'PowerShell' : 'Python'} not found</span>
                {/if}
              {/if}
              <button class="ai-instructions-link" onclick={() => aiInstructionsOpen = true}>AI Instructions</button>
            </div>
          </div>
        </div>
        <textarea
          class="script-textarea"
          bind:value={scriptEditorContent}
          rows="16"
          spellcheck="false"
        ></textarea>
        {#if scriptEditorError}
          <span class="settings-error">{scriptEditorError}</span>
        {/if}
      </div>
      <div class="help-footer">
        <button class="btn-accent" onclick={handleSaveScript}>Save</button>
        <button class="btn-default" onclick={() => scriptEditorOpen = false}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if aiInstructionsOpen}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1"
       onclick={() => aiInstructionsOpen = false}
       onkeydown={(e) => { if (e.key === 'Escape') aiInstructionsOpen = false; }}>
    <div class="help-popup ai-instructions-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      <div class="help-title">
        AI Instructions &mdash; {scriptEditorLanguage === 'python' ? 'Python' : scriptEditorLanguage === 'csharp' ? 'C# (.NET)' : 'PowerShell'}
      </div>
      <div class="help-body">
        <pre class="ai-instructions-pre">{getAiInstructions(scriptEditorLanguage)}</pre>
      </div>
      <div class="help-footer">
        <button class="btn-accent" onclick={handleCopyAiInstructions}>Copy</button>
        <button class="btn-default" onclick={() => aiInstructionsOpen = false}>Close</button>
      </div>
    </div>
  </div>
{/if}
