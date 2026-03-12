<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';

  interface Props {
    visible: boolean;
    onclose: () => void;
  }

  let { visible = $bindable(), onclose }: Props = $props();

  let input = $state('');
  let isRunning = $state(false);
  let isConfigured = $state(false);
  let autonomyLevel = $state(2);  // 0-4 slider (default: AutoReadOnly)
  let dryRun = $state(false);
  let inputEl: HTMLInputElement = $state(null!);

  const autonomyLabels = [
    'Suggest Only',
    'Ask Everything',
    'Auto Read-Only',
    'Auto Non-Destructive',
    'Full Auto',
  ];

  onMount(async () => {
    isConfigured = await invoke<boolean>('grok_status');
    autonomyLevel = await invoke<number>('get_autonomy_level');
    dryRun = await invoke<boolean>('get_dry_run');
  });

  // Auto-focus when the bar appears.
  $effect(() => {
    if (visible && inputEl) {
      requestAnimationFrame(() => inputEl?.focus());
    }
  });

  async function submit() {
    const text = input.trim();
    if (!text || isRunning) return;

    isRunning = true;
    input = '';

    try {
      // Sync autonomy + dry-run settings before launching.
      await invoke('set_autonomy_level', { level: String(autonomyLevel) });
      await invoke('set_dry_run', { enabled: dryRun });
      await invoke<string>('agent_run', { prompt: text });
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

{#if visible}
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
    <label class="ac-toggle" title="Dry-run mode: simulate changes without executing">
      <input type="checkbox" bind:checked={dryRun} />
      <span>Dry Run</span>
    </label>
    <button class="undo-btn" onclick={undoLast} title="Undo last agent file change">Undo</button>
    <button class="esc" onclick={() => { visible = false; onclose(); }}>Esc</button>
  </div>
{/if}

<style>
  .agent-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: #0f0018;
    border-bottom: 1px solid #ff007f44;
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
  .ac-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: #00ffff;
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
  }
  .ac-toggle input[type='checkbox'] {
    accent-color: #00ffff;
    width: 13px;
    height: 13px;
    cursor: pointer;
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
</style>
