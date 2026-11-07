<script lang="ts">
  import { onMount } from 'svelte';
  import { updateHotkey, saveSettings, applyPluginConfig, gdriveHasToken } from '../lib/api';
  import { settings, type AppSettings } from '../lib/stores/settings';
  import { applyTheme } from '../lib/stores/theme';
  import type { DiscoveredPlugin, PluginConfig } from '../lib/plugin-types';
  import { buildHelpTexts } from '../lib/help-texts';
  import { showAlert } from '../lib/stores/alert.svelte';
  import { capabilities, loadCapabilities } from '../lib/features';
  import '../lib/settings-shared.css';
  // SettingsPlugins is mounted only when the binary has the plugin system
  // (`$capabilities.plugins`: Windows always; macOS only in the personal
  // build). The component ships in the bundle either way; without the
  // plugin system nothing ever mounts it or calls its commands.
  import SettingsGeneral from './SettingsGeneral.svelte';
  import SettingsStorage from './SettingsStorage.svelte';
  import SettingsPlugins from './SettingsPlugins.svelte';

  // `initialTab` lets a caller elsewhere in the app land the user on the right
  // pane — e.g. the "Connect Google Drive" link shown on an upload
  // authorization error opens Settings straight on Storage.
  // `tabRequest` lets a caller elsewhere in the app land the user on the right
  // pane — e.g. the "Connect Google Drive" link shown on an upload
  // authorization error opens Settings straight on Storage. The `seq` makes
  // repeat requests distinct so they still switch a window that is already open.
  let { onClose, tabRequest }: {
    onClose: () => void;
    tabRequest?: { tab: 'general' | 'storage'; seq: number };
  } = $props();

  let localSettings: AppSettings = $state({ ...$settings });
  let originalTheme = $state($settings.theme);
  let helpTopic = $state<string | null>(null);
  let guideTab = $state<'general' | 'plugins'>('general');
  // `plugins` is only a valid activeTab value when `$capabilities.plugins` is
  // true. Initial state stays 'general' either way; the tab button below is
  // the gate, so the value can never be 'plugins' in a build without the
  // plugin system (no user input reaches it).
  let activeTab = $state<'general' | 'storage' | 'plugins'>('general');

  // Follow every request, not just the first. Reruns only when `seq` changes,
  // so a tab the user picked by hand is never yanked out from under them.
  $effect(() => {
    if (tabRequest && tabRequest.seq > 0) activeTab = tabRequest.tab;
  });

  // ── Plugin state owned here so it survives the Plugins tab unmounting ──
  // (SettingsPlugins binds these; handleSave reads them for validation + save.)
  // All unused on the store build — declared but never read; the TS compiler
  // sees them via the unconditional import above and is happy either way.
  let discoveredPlugins = $state<DiscoveredPlugin[]>([]);
  let pluginConfigs = $state<PluginConfig[]>([]);
  let pluginsScanned = $state(false);

  // ── Google Drive connection state, owned here so it persists for the window's
  // lifetime across Storage-tab remounts (SettingsStorage binds these).
  let gdriveConnected = $state(false);
  let gdriveEmail = $state('');

  // Help texts depend on what this binary has (plugins, data dir name).
  let helpTexts = $derived(buildHelpTexts($capabilities.plugins, $capabilities.appDirName));
  let pluginsEnabled = $derived($capabilities.plugins);

  onMount(async () => {
    loadCapabilities();
    try {
      gdriveConnected = await gdriveHasToken();
    } catch (_) {}
  });

  function showHelp(key: string) {
    if (key === 'guide') guideTab = 'general';
    helpTopic = key;
  }

  async function handleSave() {
    try {
      // Validate plugin settings before saving
      if (pluginsEnabled && pluginsScanned) {
        for (const cfg of pluginConfigs) {
          if (!cfg.enabled) continue;
          const plugin = discoveredPlugins.find(p => p.path === cfg.path);
          if (!plugin) continue;
          if (plugin.settings_format && !cfg.settings?.trim()) {
            activeTab = 'plugins';
            showAlert(`Cannot enable "${plugin.name}" — plugin settings are required. Click "Settings" to configure.`);
            return;
          }
        }
      }

      const hotkeyChanged = localSettings.captureHotkey !== $settings.captureHotkey;
      if (hotkeyChanged) {
        await updateHotkey(localSettings.captureHotkey);
      }
      await saveSettings(localSettings);
      if (pluginsEnabled && pluginsScanned) {
        await applyPluginConfig(pluginConfigs);
      }
      settings.set(localSettings);
      applyTheme(localSettings.theme);
      onClose();
    } catch (e) {
      showAlert('Failed to save: ' + e);
    }
  }

  function handleCancel() {
    applyTheme(originalTheme);
    onClose();
  }
</script>

<div class="settings-page">
  <!-- Tab bar. The Plugins tab is not rendered at all when the binary has no
       plugin system (`$capabilities.plugins` false: the default macOS
       build). The Svelte tree below then never contains a `SettingsPlugins`
       mount, so none of its plugin commands is ever invoked. -->
  <div class="tab-bar">
    <button class="tab-btn" class:active={activeTab === 'general'} onclick={() => activeTab = 'general'}>General</button>
    <button class="tab-btn" class:active={activeTab === 'storage'} onclick={() => activeTab = 'storage'}>Storage</button>
    {#if pluginsEnabled}
      <button class="tab-btn" class:active={activeTab === 'plugins'} onclick={() => activeTab = 'plugins'}>Plugins</button>
    {/if}
  </div>

  <div class="tab-content">
  {#if activeTab === 'general'}
    <SettingsGeneral settings={localSettings} {showHelp} />
  {/if}

  {#if activeTab === 'storage'}
    <SettingsStorage settings={localSettings} {showHelp} bind:gdriveConnected bind:gdriveEmail />
  {/if}

  {#if pluginsEnabled && activeTab === 'plugins'}
    <SettingsPlugins
      bind:pluginConfigs
      bind:discoveredPlugins
      bind:pluginsScanned
    />
  {/if}
  </div>

  <!-- Bottom buttons -->
  <div class="bottom-buttons">
    <button class="guide-link" onclick={() => showHelp('guide')}>User Guide</button>
    <div class="bottom-buttons-right">
      <button class="btn-accent btn-save" onclick={handleSave}>Save</button>
      <button class="btn-default" onclick={handleCancel}>Cancel</button>
    </div>
  </div>
</div>

{#if helpTopic && helpTexts[helpTopic]}
  {@const isGuide = helpTopic === 'guide'}
  {@const currentTopic = isGuide ? (guideTab === 'plugins' && pluginsEnabled ? 'guidePlugins' : 'guide') : helpTopic}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1" onclick={() => helpTopic = null} onkeydown={(e) => { if (e.key === 'Escape') helpTopic = null; }}>
    <div class="help-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      {#if isGuide}
        <div class="guide-tab-bar">
          <button class="guide-tab" class:active={guideTab === 'general'} onclick={() => guideTab = 'general'}>User Guide</button>
          {#if pluginsEnabled}
            <button class="guide-tab" class:active={guideTab === 'plugins'} onclick={() => guideTab = 'plugins'}>Plugins</button>
          {/if}
        </div>
      {:else}
        <div class="help-title">{helpTexts[helpTopic].title}</div>
      {/if}
      <div class="help-body">
        {#each helpTexts[currentTopic].text.split('\n') as line}
          {#if line === ''}
            <br />
          {:else if /^[A-Z][A-Z\s\-—]+$/.test(line.trim())}
            <p class="help-heading">{line}</p>
          {:else if line.startsWith('• ')}
            <p class="help-bullet">{line}</p>
          {:else}
            <p>{line}</p>
          {/if}
        {/each}
      </div>
      <div class="help-footer">
        <button class="btn-default" onclick={() => helpTopic = null}>Close</button>
      </div>
    </div>
  </div>
{/if}

