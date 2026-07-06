<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { getVersion } from '@tauri-apps/api/app';
  import { onMount } from 'svelte';
  import { settings } from '../lib/stores/settings';

  let { onClose }: { onClose: () => void } = $props();
  let version = $state('');
  onMount(async () => {
    try { version = await getVersion(); } catch { version = ''; }
  });
</script>

<div class="about-page">
  <div class="about-content">
    <p class="line">Created by ultrathinker.</p>
    <p class="line">
      <button class="link-btn" onclick={() => openUrl('https://cliptoall.appshub.net')}>
        https://cliptoall.appshub.net
      </button>
    </p>
    <p class="line email">universeissilent42@gmail.com</p>
    <p class="line version">Version  {version}</p>
    <div class="spacer"></div>
    <p class="line hint">Press {$settings.captureHotkey} to capture  &middot;  double-press to toggle mode</p>
  </div>
</div>

<style>
  .about-page {
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-base);
    padding: 20px;
    box-sizing: border-box;
  }

  .about-content {
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }

  .line {
    font-size: 9.5pt;
    color: var(--text-main);
    margin: 0;
    line-height: 1.8;
  }

  .email {
    color: var(--text-dim);
  }

  .version {
    color: var(--text-dim);
    font-size: 9pt;
  }

  .hint {
    color: var(--text-dim);
    font-size: 9pt;
    margin-top: 12px;
  }

  .spacer {
    height: 8px;
  }

  .link-btn {
    background: none;
    border: none;
    padding: 0;
    font-size: 9.5pt;
    color: var(--text-main);
    cursor: pointer;
    font-family: inherit;
    text-decoration: none;
  }

  .link-btn:hover {
    text-decoration: underline;
  }
</style>
