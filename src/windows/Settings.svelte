<script lang="ts">
  import { onMount } from 'svelte';
  import { updateHotkey, saveSettings, applyPluginConfig, gdriveHasToken } from '../lib/api';
  import { settings, type AppSettings } from '../lib/stores/settings';
  import { applyTheme } from '../lib/stores/theme';
  import type { DiscoveredPlugin, PluginConfig } from '../lib/plugin-types';
  import { helpTexts } from '../lib/help-texts';
  import '../lib/settings-shared.css';
  import SettingsGeneral from './SettingsGeneral.svelte';
  import SettingsStorage from './SettingsStorage.svelte';
  import SettingsPlugins from './SettingsPlugins.svelte';

  let { onClose }: { onClose: () => void } = $props();

  let localSettings: AppSettings = $state({ ...$settings });
  let originalTheme = $state($settings.theme);
  let helpTopic = $state<string | null>(null);
  let guideTab = $state<'general' | 'plugins'>('general');
  let activeTab = $state<'general' | 'storage' | 'plugins'>('general');

  // ── Plugin state owned here so it survives the Plugins tab unmounting ──
  // (SettingsPlugins binds these; handleSave reads them for validation + save.)
  let discoveredPlugins = $state<DiscoveredPlugin[]>([]);
  let pluginConfigs = $state<PluginConfig[]>([]);
  let pluginsScanned = $state(false);

  // ── Google Drive connection state, owned here so it persists for the window's
  // lifetime across Storage-tab remounts (SettingsStorage binds these).
  let gdriveConnected = $state(false);
  let gdriveEmail = $state('');

  onMount(async () => {
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
      if (pluginsScanned) {
        for (const cfg of pluginConfigs) {
          if (!cfg.enabled) continue;
          const plugin = discoveredPlugins.find(p => p.path === cfg.path);
          if (!plugin) continue;
          if (plugin.settings_format && !cfg.settings?.trim()) {
            activeTab = 'plugins';
            alert(`Cannot enable "${plugin.name}" — plugin settings are required. Click "Settings" to configure.`);
            return;
          }
        }
      }

      const hotkeyChanged = localSettings.captureHotkey !== $settings.captureHotkey;
      if (hotkeyChanged) {
        await updateHotkey(localSettings.captureHotkey);
      }
      await saveSettings(localSettings);
      if (pluginsScanned) {
        await applyPluginConfig(pluginConfigs);
      }
      settings.set(localSettings);
      applyTheme(localSettings.theme);
      onClose();
    } catch (e) {
      alert('Failed to save: ' + e);
    }
  }

  function handleCancel() {
    applyTheme(originalTheme);
    onClose();
  }
</script>

<div class="settings-page">
  <!-- Tab bar -->
  <div class="tab-bar">
    <button class="tab-btn" class:active={activeTab === 'general'} onclick={() => activeTab = 'general'}>General</button>
    <button class="tab-btn" class:active={activeTab === 'storage'} onclick={() => activeTab = 'storage'}>Storage</button>
    <button class="tab-btn" class:active={activeTab === 'plugins'} onclick={() => activeTab = 'plugins'}>Plugins</button>
  </div>

  <div class="tab-content">
  {#if activeTab === 'general'}
    <SettingsGeneral settings={localSettings} {showHelp} />
  {/if}

  {#if activeTab === 'storage'}
    <SettingsStorage settings={localSettings} {showHelp} bind:gdriveConnected bind:gdriveEmail />
  {/if}

  {#if activeTab === 'plugins'}
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
  {@const currentTopic = isGuide ? (guideTab === 'plugins' ? 'guidePlugins' : 'guide') : helpTopic}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="help-overlay" role="button" tabindex="-1" onclick={() => helpTopic = null} onkeydown={(e) => { if (e.key === 'Escape') helpTopic = null; }}>
    <div class="help-popup" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => { if (e.key !== 'Escape') e.stopPropagation(); }}>
      {#if isGuide}
        <div class="guide-tab-bar">
          <button class="guide-tab" class:active={guideTab === 'general'} onclick={() => guideTab = 'general'}>User Guide</button>
          <button class="guide-tab" class:active={guideTab === 'plugins'} onclick={() => guideTab = 'plugins'}>Plugins</button>
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

