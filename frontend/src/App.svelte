<script lang="ts">
  import Terminal from './lib/Terminal.svelte';
  import GrokSidebar from './lib/GrokSidebar.svelte';
  import CommandPalette from './lib/CommandPalette.svelte';

  let showSidebar = $state(true);
  let showPalette = $state(false);

  // Global keyboard shortcut: Ctrl+K opens the command palette,
  // Ctrl+B toggles the Grok sidebar.
  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault();
      showPalette = !showPalette;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
      e.preventDefault();
      showSidebar = !showSidebar;
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="app">
  <header class="titlebar">
    <div class="title">Grok Terminal</div>
    <div class="actions">
      <button
        class="action-btn"
        title="Command Palette (Ctrl+K)"
        onclick={() => (showPalette = true)}
      >
        Ctrl+K
      </button>
      <button
        class="action-btn"
        title="Toggle Sidebar (Ctrl+B)"
        onclick={() => (showSidebar = !showSidebar)}
      >
        {showSidebar ? 'Hide AI' : 'Show AI'}
      </button>
    </div>
  </header>

  <div class="main">
    <div class="terminal-pane">
      <Terminal />
    </div>
    {#if showSidebar}
      <div class="sidebar-pane">
        <GrokSidebar />
      </div>
    {/if}
  </div>
</div>

<CommandPalette bind:visible={showPalette} onclose={() => (showPalette = false)} />

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    background: #1a1b26;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background: #1a1b26;
    color: #c0caf5;
  }

  .titlebar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    height: 36px;
    padding: 0 16px;
    background: #16161e;
    border-bottom: 1px solid #292e42;
    -webkit-app-region: drag;
    user-select: none;
  }

  .title {
    font-size: 13px;
    font-weight: 600;
    color: #c0caf5;
  }

  .actions {
    display: flex;
    gap: 8px;
    -webkit-app-region: no-drag;
  }

  .action-btn {
    background: #292e42;
    color: #a9b1d6;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
    font-size: 11px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
  }
  .action-btn:hover {
    background: #3b4261;
    color: #c0caf5;
  }

  .main {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .terminal-pane {
    flex: 1;
    overflow: hidden;
  }

  .sidebar-pane {
    width: 350px;
    min-width: 280px;
    max-width: 450px;
    overflow: hidden;
  }
</style>
