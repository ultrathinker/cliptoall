<script lang="ts">
  import { gdriveAuthorize, gdriveDisconnect } from '../lib/api';
  import type { AppSettings } from '../lib/stores/settings';
  import '../lib/settings-storage.css';

  // gdriveConnected/gdriveEmail are owned by the parent so they persist for the
  // window's lifetime — otherwise re-entering this tab remounts the child and
  // the "Connected as <email>" label would drop back to just "Connected".
  let {
    settings,
    showHelp,
    gdriveConnected = $bindable(false),
    gdriveEmail = $bindable(''),
  }: {
    settings: AppSettings;
    showHelp: (key: string) => void;
    gdriveConnected: boolean;
    gdriveEmail: string;
  } = $props();

  let showSecretKey = $state(false);

  async function handleGDriveConnect() {
    try {
      gdriveEmail = await gdriveAuthorize();
      gdriveConnected = true;
    } catch (e) {
      alert('Failed to connect: ' + e);
    }
  }

  async function handleGDriveDisconnect() {
    try {
      await gdriveDisconnect();
      gdriveConnected = false;
      gdriveEmail = '';
    } catch (e) {
      alert('Failed to disconnect: ' + e);
    }
  }
</script>

<fieldset class="group-box">
  <legend>Image Storage <button class="help-btn help-btn-legend" onclick={() => showHelp('storage')}>?</button></legend>

  <div class="radio-row">
    <label class="radio-label">
      <input type="radio" bind:group={settings.storageType} value="gdrive" />
      <span>Google Drive</span>
    </label>
    <label class="radio-label">
      <input type="radio" bind:group={settings.storageType} value="s3" />
      <span>Amazon S3</span>
    </label>
  </div>

  {#if settings.storageType === 's3'}
    <div class="storage-fields">
      <div class="field-row">
        <label class="field-label" for="s3-access-key">Access Key ID:</label>
        <input id="s3-access-key" type="text" bind:value={settings.amazonAccessKeyId} class="field-input full" />
      </div>
      <div class="field-row">
        <label class="field-label" for="s3-secret-key">Secret Access Key:</label>
        <div class="field-with-btn">
          <input id="s3-secret-key" type={showSecretKey ? 'text' : 'password'} bind:value={settings.amazonSecretAccessKey} class="field-input flex1" />
          <button class="btn-small" onclick={() => showSecretKey = !showSecretKey}>{showSecretKey ? 'Hide' : 'Show'}</button>
        </div>
      </div>
      <div class="field-row two-col">
        <div class="field-pair">
          <label class="field-label" for="s3-bucket">S3 Bucket:</label>
          <input id="s3-bucket" type="text" bind:value={settings.amazonBucket} class="field-input" />
        </div>
        <div class="field-pair">
          <label class="field-label-short" for="s3-region">S3 Region:</label>
          <input id="s3-region" type="text" bind:value={settings.amazonRegion} class="field-input" />
        </div>
      </div>
      <div class="field-row">
        <label class="field-label" for="s3-folder">S3 Folder:</label>
        <input id="s3-folder" type="text" bind:value={settings.amazonS3folder} class="field-input half" />
      </div>
    </div>
  {:else}
    <div class="storage-fields">
      <div class="field-row">
        <label class="field-label" for="gdrive-folder">Drive Folder:</label>
        <input id="gdrive-folder" type="text" bind:value={settings.googleDriveFolder} class="field-input half" />
      </div>
      <div class="field-row gdrive-connect-row">
        <span class="field-label"></span>
        <div class="gdrive-actions">
          {#if !gdriveConnected}
            <button class="btn-accent" onclick={handleGDriveConnect}>Connect to Google Drive</button>
          {:else}
            <button class="btn-small" onclick={handleGDriveDisconnect}>Disconnect</button>
          {/if}
          <span class="gdrive-status" class:connected={gdriveConnected}>
            {gdriveConnected ? `Connected${gdriveEmail ? ` as ${gdriveEmail}` : ''}` : 'Not connected'}
          </span>
        </div>
      </div>
    </div>
  {/if}
</fieldset>
