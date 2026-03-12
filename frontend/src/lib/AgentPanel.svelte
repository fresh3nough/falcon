<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';

  interface ImageRef {
    dataUrl: string;
    name: string;
  }

  interface Props {
    visible: boolean;
    onclose: () => void;
  }

  let { visible = $bindable(), onclose }: Props = $props();

  let input = $state('');
  let isRunning = $state(false);
  let isConfigured = $state(false);
  // Default to Full Auto (index 4) — no approval dialogs.
  let autonomyLevel = $state(4);
  let inputEl: HTMLInputElement = $state(null!);
  let fileInputEl: HTMLInputElement = $state(null!);
  let pendingImages: ImageRef[] = $state([]);

  const autonomyLabels = [
    'Suggest Only',
    'Ask Everything',
    'Auto Read-Only',
    'Auto Non-Destructive',
    'Full Auto',
  ];

  onMount(async () => {
    isConfigured = await invoke<boolean>('grok_status');
    // Always apply Full Auto on mount.
    await invoke('set_autonomy_level', { level: '4' });
  });

  // Auto-focus when the bar appears.
  $effect(() => {
    if (visible && inputEl) {
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  // -- Image handling --

  function handleDrop(e: DragEvent) {
    e.preventDefault();
    if (!e.dataTransfer?.files) return;
    addImageFiles(e.dataTransfer.files);
  }

  function handleDragOver(e: DragEvent) {
    e.preventDefault();
  }

  function handlePaste(e: ClipboardEvent) {
    if (!e.clipboardData?.files?.length) return;
    addImageFiles(e.clipboardData.files);
  }

  async function addImageFiles(files: FileList) {
    for (const file of Array.from(files)) {
      if (!file.type.startsWith('image/')) continue;
      const reader = new FileReader();
      reader.onload = () => {
        pendingImages = [...pendingImages, {
          dataUrl: reader.result as string,
          name: file.name,
        }];
      };
      reader.readAsDataURL(file);
    }
  }

  function removePendingImage(idx: number) {
    pendingImages = pendingImages.filter((_, i) => i !== idx);
  }

  async function submit() {
    const text = input.trim();
    if (!text || isRunning) return;

    isRunning = true;
    input = '';
    const images = [...pendingImages];
    pendingImages = [];

    try {
      // Ensure Full Auto is set before launching.
      await invoke('set_autonomy_level', { level: '4' });
      await invoke<string>('agent_run', {
        prompt: text,
        imageDataUrls: images.length > 0 ? images.map(i => i.dataUrl) : undefined,
      });
    } catch (err) {
      console.error('Agent start failed:', err);
    }

    // Close the input bar immediately -- output renders in the terminal.
    isRunning = false;
    visible = false;
    onclose();
  }

  async function undoLast() {
    try {
      const msg = await invoke<string>('agent_undo');
      console.log(msg);
    } catch (err) {
      console.error('Undo failed:', err);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      submit();
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      visible = false;
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
{#if visible}
  <div
    class="agent-bar-wrap"
    ondrop={handleDrop}
    ondragover={handleDragOver}
    onpaste={handlePaste}
  >
    <!-- Pending image thumbnails -->
    {#if pendingImages.length > 0}
      <div class="pending-images">
        {#each pendingImages as img, idx}
          <div class="pending-thumb-wrap">
            <img src={img.dataUrl} alt={img.name} class="pending-thumb" title={img.name} />
            <button class="remove-img" onclick={() => removePendingImage(idx)}>x</button>
          </div>
        {/each}
      </div>
    {/if}

    <div class="agent-bar">
      <span class="hash">#</span>
      <input
        type="text"
        bind:this={inputEl}
        bind:value={input}
        onkeydown={handleKeydown}
        placeholder={isConfigured ? 'Describe what you need...' : 'Set XAI_API_KEY to enable'}
        disabled={!isConfigured || isRunning}
      />
      <!-- Hidden file input for image uploads -->
      <input
        type="file"
        accept="image/*"
        multiple
        bind:this={fileInputEl}
        onchange={(e) => {
          const files = (e.target as HTMLInputElement).files;
          if (files) addImageFiles(files);
          (e.target as HTMLInputElement).value = '';
        }}
        style="display:none"
      />
      <!-- Image attach button -->
      <button
        class="img-btn"
        onclick={() => fileInputEl?.click()}
        disabled={!isConfigured || isRunning}
        title="Attach image(s) for vision analysis"
      >
        img
      </button>
      <button onclick={submit} disabled={!isConfigured || isRunning || !input.trim()}>
        {isRunning ? '...' : 'Go'}
      </button>
      <div class="autonomy-control" title="Agent autonomy level">
        <input
          type="range"
          min="0"
          max="4"
          step="1"
          bind:value={autonomyLevel}
          class="autonomy-slider"
        />
        <span class="autonomy-label">{autonomyLabels[autonomyLevel]}</span>
      </div>
      <button class="undo-btn" onclick={undoLast} title="Undo last agent file change">Undo</button>
      <button class="esc" onclick={() => { visible = false; onclose(); }}>Esc</button>
    </div>
  </div>
{/if}

<style>
  .agent-bar-wrap {
    background: #0f0018;
    border-bottom: 1px solid #ff007f44;
  }
  .agent-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
  }
  .hash {
    font-family: 'JetBrains Mono', monospace;
    font-size: 15px;
    font-weight: 700;
    color: #ff007f;
  }
  input {
    flex: 1;
    background: #0a000f;
    border: 1px solid #330044;
    border-radius: 6px;
    color: #ff9ef7;
    padding: 6px 10px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    outline: none;
  }
  input:focus {
    border-color: #ff007f;
  }
  button {
    background: #ff007f;
    color: #0a000f;
    border: none;
    border-radius: 6px;
    padding: 6px 14px;
    font-weight: 700;
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
  }
  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  button:hover:not(:disabled) {
    background: #ff3399;
  }
  .autonomy-control {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .autonomy-slider {
    width: 80px;
    accent-color: #ff007f;
    cursor: pointer;
    height: 4px;
  }
  .autonomy-label {
    font-size: 10px;
    color: #00ffff;
    min-width: 90px;
  }
  .undo-btn {
    background: #200038;
    color: #ffe600;
    font-weight: 600;
    font-size: 11px;
  }
  .undo-btn:hover {
    background: #330055;
    color: #ffe600;
  }
  .esc {
    background: #200038;
    color: #e080ff;
    font-weight: 600;
  }
  .esc:hover {
    background: #330055;
    color: #ff9ef7;
  }

  /* Image attach button */
  .img-btn {
    background: #200038;
    color: #cc44ff;
    font-weight: 600;
    font-size: 10px;
    padding: 6px 8px;
    flex-shrink: 0;
  }
  .img-btn:hover:not(:disabled) {
    background: #330055;
    color: #ff9ef7;
  }

  /* Pending images strip above the input bar */
  .pending-images {
    display: flex;
    gap: 6px;
    padding: 6px 12px 0;
    flex-wrap: wrap;
  }
  .pending-thumb-wrap {
    position: relative;
  }
  .pending-thumb {
    width: 48px;
    height: 48px;
    object-fit: cover;
    border-radius: 4px;
    border: 1px solid #ff007f44;
  }
  .remove-img {
    position: absolute;
    top: -4px;
    right: -4px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #ff1744;
    color: #0a000f;
    border: none;
    font-size: 10px;
    line-height: 16px;
    text-align: center;
    cursor: pointer;
    padding: 0;
    font-weight: 700;
  }
</style>
