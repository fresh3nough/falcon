<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  interface Props {
    visible: boolean;
    x: number;
    y: number;
    blockId: string;
    onclose: () => void;
  }

  let { visible = $bindable(), x, y, blockId, onclose }: Props = $props();

  const actions = [
    { id: 'explain', label: 'Explain', icon: '?' },
    { id: 'fix', label: 'Fix', icon: '!' },
    { id: 'script', label: 'Turn into Script', icon: '#' },
    { id: 'tests', label: 'Add Tests', icon: 'T' },
  ];

  async function runAction(actionId: string) {
    visible = false;
    onclose();

    try {
      await invoke('block_action', { blockId, action: actionId });
    } catch (err) {
      console.error('Block action failed:', err);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      visible = false;
      onclose();
    }
  }
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ctx-overlay" onclick={() => { visible = false; onclose(); }} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="ctx-menu" style="left: {x}px; top: {y}px;" onclick={(e) => e.stopPropagation()}>
      {#each actions as action}
        <button class="ctx-item" onclick={() => runAction(action.id)}>
          <span class="ctx-icon">{action.icon}</span>
          <span>{action.label}</span>
        </button>
      {/each}
    </div>
  </div>
{/if}

<style>
  .ctx-overlay {
    position: fixed;
    inset: 0;
    z-index: 900;
  }

  .ctx-menu {
    position: absolute;
    background: #0a000f;
    border: 1px solid #ff007f66;
    border-radius: 8px;
    padding: 4px;
    min-width: 180px;
    box-shadow: 0 8px 24px rgba(128, 0, 64, 0.6);
  }

  .ctx-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 12px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: #ff9ef7;
    font-size: 13px;
    cursor: pointer;
    text-align: left;
    font-family: 'JetBrains Mono', monospace;
  }
  .ctx-item:hover {
    background: #200038;
  }

  .ctx-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 4px;
    background: #ff007f22;
    color: #ff007f;
    font-size: 11px;
    font-weight: 700;
  }
</style>
