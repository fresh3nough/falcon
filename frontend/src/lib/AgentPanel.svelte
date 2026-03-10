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
  let inputEl: HTMLInputElement = $state(null!);

  onMount(async () => {
    isConfigured = await invoke<boolean>('grok_status');
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
      await invoke<string>('agent_run', { prompt: text });
    } catch (err) {
      console.error('Agent start failed:', err);
    }

    // Close the input bar immediately -- output renders in the terminal.
    isRunning = false;
    visible = false;
    onclose();
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
    <button class="esc" onclick={() => { visible = false; onclose(); }}>Esc</button>
  </div>
{/if}

<style>
  .agent-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: #16161e;
    border-bottom: 1px solid #7aa2f744;
  }
  .hash {
    font-family: 'JetBrains Mono', monospace;
    font-size: 15px;
    font-weight: 700;
    color: #7aa2f7;
  }
  input {
    flex: 1;
    background: #1a1b26;
    border: 1px solid #292e42;
    border-radius: 6px;
    color: #c0caf5;
    padding: 6px 10px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    outline: none;
  }
  input:focus {
    border-color: #7aa2f7;
  }
  button {
    background: #7aa2f7;
    color: #1a1b26;
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
    background: #89b4fa;
  }
  .esc {
    background: #292e42;
    color: #a9b1d6;
    font-weight: 600;
  }
  .esc:hover {
    background: #3b4261;
    color: #c0caf5;
  }
</style>
