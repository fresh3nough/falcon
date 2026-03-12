<script lang="ts">
  import Terminal from './lib/Terminal.svelte';
  import GrokSidebar from './lib/GrokSidebar.svelte';
  import CommandPalette from './lib/CommandPalette.svelte';
  import AgentPanel from './lib/AgentPanel.svelte';
  import AgentOutput from './lib/AgentOutput.svelte';
  import BlockContextMenu from './lib/BlockContextMenu.svelte';
  import OrchestratorPanel from './lib/OrchestratorPanel.svelte';

  let showSidebar = $state(true);
  let showPalette = $state(false);
  let showAgent = $state(false);
  let showOrchestrator = $state(false);

  // Block context menu state.
  let ctxMenu = $state({ visible: false, x: 0, y: 0, blockId: '' });

  // Multi-terminal tab state.
  interface TerminalTab { id: string; label: string; }
  let terminals: TerminalTab[] = $state([{ id: 'main', label: 'Terminal 1' }]);
  let activeTerminalId = $state('main');
  let tabCounter = $state(2);

  /** Add a new terminal tab with its own PTY. */
  function addTerminalTab() {
    const id = `tab-${tabCounter}`;
    const label = `Terminal ${tabCounter}`;
    tabCounter += 1;
    terminals = [...terminals, { id, label }];
    activeTerminalId = id;
  }

  function openContextMenu(x: number, y: number, blockId: string) {
    ctxMenu = { visible: true, x, y, blockId };
  }

  // Global keyboard shortcuts:
  //   Ctrl+K  — command palette
  //   Ctrl+B  — toggle Grok sidebar
  //   Ctrl+G  — toggle Warpify agent panel
  //   Ctrl+O  — toggle multi-agent orchestrator
  //   Ctrl+T  — new terminal tab
  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault();
      showPalette = !showPalette;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
      e.preventDefault();
      showSidebar = !showSidebar;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'g') {
      e.preventDefault();
      showAgent = !showAgent;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'o') {
      e.preventDefault();
      showOrchestrator = !showOrchestrator;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 't') {
      e.preventDefault();
      addTerminalTab();
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
      <button
        class="action-btn agent-btn"
        title="Warpify Agent (Ctrl+G)"
        onclick={() => (showAgent = !showAgent)}
      >
        # Warpify
      </button>
      <button
        class="action-btn orch-btn"
        title="Multi-Agent Orchestrator (Ctrl+O)"
        onclick={() => (showOrchestrator = !showOrchestrator)}
      >
        # Orchestrate
      </button>
    </div>
  </header>

  <div class="main">
    <div class="terminal-pane">
      <AgentPanel bind:visible={showAgent} onclose={() => (showAgent = false)} />
      <OrchestratorPanel bind:visible={showOrchestrator} onclose={() => (showOrchestrator = false)} />
      <AgentOutput />

      <!-- Terminal tab bar -->
      <div class="terminal-tabbar">
        {#each terminals as tab}
          <button
            class="terminal-tab"
            class:active={activeTerminalId === tab.id}
            onclick={() => (activeTerminalId = tab.id)}
          >
            {tab.label}
          </button>
        {/each}
        <button class="terminal-tab-add" onclick={addTerminalTab} title="New terminal (Ctrl+T)">
          +
        </button>
      </div>

      <!-- One Terminal per tab; inactive tabs are hidden but remain mounted -->
      {#each terminals as tab}
        <div class="terminal-wrapper" class:hidden={activeTerminalId !== tab.id}>
          <Terminal
            ptyId={tab.id}
            active={activeTerminalId === tab.id}
            oncontextmenu={openContextMenu}
          />
        </div>
      {/each}
    </div>
    {#if showSidebar}
      <div class="sidebar-pane">
        <GrokSidebar />
      </div>
    {/if}
  </div>
</div>

<CommandPalette
  bind:visible={showPalette}
  onclose={() => (showPalette = false)}
  onopenagent={() => { showPalette = false; showAgent = true; }}
/>

<BlockContextMenu
  bind:visible={ctxMenu.visible}
  x={ctxMenu.x}
  y={ctxMenu.y}
  blockId={ctxMenu.blockId}
  onclose={() => (ctxMenu.visible = false)}
/>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    background: #0a000f;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    background: #0a000f;
    color: #ff9ef7;
  }

  .titlebar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    height: 36px;
    padding: 0 16px;
    background: #0f0018;
    border-bottom: 1px solid #ff007f44;
    -webkit-app-region: drag;
    user-select: none;
  }

  .title {
    font-size: 13px;
    font-weight: 600;
    color: #ff9ef7;
  }

  .actions {
    display: flex;
    gap: 8px;
    -webkit-app-region: no-drag;
  }

  .action-btn {
    background: #200038;
    color: #e080ff;
    border: none;
    border-radius: 4px;
    padding: 4px 10px;
    font-size: 11px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
  }
  .action-btn:hover {
    background: #330055;
    color: #ff9ef7;
  }
  .agent-btn {
    background: #ff007f33;
    color: #ff007f;
    font-weight: 600;
  }
  .agent-btn:hover {
    background: #7aa2f755;
  }
  .orch-btn {
    background: #bb9af733;
    color: #bb9af7;
    font-weight: 600;
  }
  .orch-btn:hover {
    background: #bb9af755;
  }

  .main {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .terminal-pane {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  /* Terminal tab bar */
  .terminal-tabbar {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
    background: #0d0020;
    border-bottom: 1px solid #ff007f33;
    flex-shrink: 0;
    height: 28px;
  }

  .terminal-tab {
    background: #180028;
    color: #cc44ff;
    border: 1px solid #330044;
    border-bottom: none;
    border-radius: 4px 4px 0 0;
    padding: 3px 12px;
    font-size: 11px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
    white-space: nowrap;
    transition: background 0.1s;
  }
  .terminal-tab.active {
    background: #0a000f;
    color: #ff9ef7;
    border-color: #ff007f55;
  }
  .terminal-tab:hover:not(.active) {
    background: #220035;
    color: #ff9ef7;
  }

  .terminal-tab-add {
    background: transparent;
    color: #ff007f;
    border: 1px solid #ff007f44;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 14px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
    margin-left: 4px;
    line-height: 1;
  }
  .terminal-tab-add:hover {
    background: #ff007f22;
    color: #ff9ef7;
  }

  /* Terminal instance wrapper */
  .terminal-wrapper {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .terminal-wrapper.hidden {
    display: none;
  }

  .sidebar-pane {
    width: 350px;
    min-width: 280px;
    max-width: 450px;
    overflow: hidden;
  }
</style>
