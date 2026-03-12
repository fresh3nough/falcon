<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';

  interface Props {
    visible: boolean;
    onclose: () => void;
    onopenagent?: () => void;
  }

  let { visible = $bindable(), onclose, onopenagent }: Props = $props();
  let query = $state('');
  let selectedIndex = $state(0);

  interface Command {
    id: string;
    label: string;
    description: string;
    action: () => void;
  }

  const commands: Command[] = [
    {
      id: 'warpify',
      label: 'Warpify (Agent Mode)',
      description: 'Run multi-step tasks with AI — Ctrl+G',
      action: () => {
        close();
        onopenagent?.();
      },
    },
    {
      id: 'generate',
      label: 'Generate Command',
      description: 'Describe what you want in natural language',
      action: () => {
        // The query text becomes the Grok prompt.
        if (query.trim()) {
          invoke('grok_generate_command', { description: query.trim() });
        }
        close();
      },
    },
    {
      id: 'explain',
      label: 'Explain Last Output',
      description: 'Ask Grok to explain the most recent command output',
      action: async () => {
        const blocks: any[] = await invoke('get_blocks');
        if (blocks.length > 0) {
          invoke('grok_explain', { blockId: blocks[blocks.length - 1].id });
        }
        close();
      },
    },
    {
      id: 'fix',
      label: 'Fix Last Command',
      description: 'Ask Grok to fix the most recent failed command',
      action: async () => {
        const blocks: any[] = await invoke('get_blocks');
        if (blocks.length > 0) {
          invoke('grok_fix', { blockId: blocks[blocks.length - 1].id });
        }
        close();
      },
    },
    {
      id: 'context',
      label: 'Show Session Context',
      description: 'Display current working directory, git status, etc.',
      action: async () => {
        const ctx = await invoke('get_context');
        console.log('Session context:', ctx);
        close();
      },
    },
  ];

  $effect(() => {
    if (visible) {
      query = '';
      selectedIndex = 0;
    }
  });

  function close() {
    visible = false;
    onclose();
  }

  function filteredCommands(): Command[] {
    if (!query.trim()) return commands;
    const q = query.toLowerCase();
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(q) ||
        c.description.toLowerCase().includes(q)
    );
  }

  function handleKeydown(e: KeyboardEvent) {
    const filtered = filteredCommands();
    if (e.key === 'Escape') {
      close();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, filtered.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (filtered[selectedIndex]) {
        filtered[selectedIndex].action();
      }
    }
  }
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="overlay" onclick={close} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="palette" onclick={(e) => e.stopPropagation()}>
      <input
        type="text"
        bind:value={query}
        placeholder="Type a command or describe what you need..."
        autofocus
        onkeydown={handleKeydown}
      />
      <div class="results">
        {#each filteredCommands() as cmd, i}
          <button
            class="result"
            class:selected={i === selectedIndex}
            onclick={() => cmd.action()}
          >
            <span class="label">{cmd.label}</span>
            <span class="desc">{cmd.description}</span>
          </button>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(10, 0, 25, 0.8);
    display: flex;
    justify-content: center;
    padding-top: 20vh;
    z-index: 1000;
  }

  .palette {
    background: #0a000f;
    border: 1px solid #ff007f66;
    border-radius: 12px;
    width: 500px;
    max-height: 400px;
    overflow: hidden;
    box-shadow: 0 20px 60px rgba(255, 0, 127, 0.25);
  }

  input {
    width: 100%;
    padding: 16px;
    background: transparent;
    border: none;
    border-bottom: 1px solid #ff007f44;
    color: #ff9ef7;
    font-size: 15px;
    font-family: 'JetBrains Mono', monospace;
    outline: none;
    box-sizing: border-box;
  }

  .results {
    max-height: 300px;
    overflow-y: auto;
    padding: 4px;
  }

  .result {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: 10px 14px;
    background: transparent;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    text-align: left;
    color: #ff9ef7;
  }
  .result:hover,
  .result.selected {
    background: #200038;
  }

  .label {
    font-size: 14px;
    font-weight: 500;
  }
  .desc {
    font-size: 12px;
    color: #cc44ff;
    margin-top: 2px;
  }
</style>
