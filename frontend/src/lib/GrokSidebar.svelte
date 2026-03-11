<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  // -- Props --
  interface Props {
    onwarpify?: () => void;
  }
  let { onwarpify }: Props = $props();

  // -- Message types --
  type MessageRole = 'user' | 'assistant' | 'agent-step';

  interface Message {
    role: MessageRole;
    content: string;
    // Agent step metadata (only when role === 'agent-step').
    stepType?: string;
    stepData?: any;
  }

  let messages: Message[] = $state([]);
  let input = $state('');
  let isStreaming = $state(false);
  let isConfigured = $state(false);
  let agentRunning = $state(false);
  let showLogSearch = $state(false);
  let logSearchQuery = $state('');
  let messagesEl: HTMLDivElement;

  let unlistenToken: UnlistenFn;
  let unlistenDone: UnlistenFn;
  let unlistenSidebarStep: UnlistenFn;
  let unlistenSidebarThinking: UnlistenFn;

  onMount(async () => {
    isConfigured = await invoke<boolean>('grok_status');

    // Listen for streaming tokens from the Grok backend (chat responses).
    unlistenToken = await listen<string>('grok-token', (event) => {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.content += event.payload;
        messages = [...messages];
      }
      scrollToBottom();
    });

    unlistenDone = await listen('grok-done', () => {
      isStreaming = false;
    });

    // Listen for sidebar-specific agent step events.
    unlistenSidebarStep = await listen<{
      session_id: string;
      step: string;
      data: any;
    }>('sidebar-agent-step', (event) => {
      renderAgentStepInSidebar(event.payload.step, event.payload.data);
    });

    // Listen for agent thinking tokens (shared channel).
    unlistenSidebarThinking = await listen<string>('agent-thinking-token', (event) => {
      if (agentRunning) {
        // Append to the last agent-step message if it exists.
        const last = messages[messages.length - 1];
        if (last && last.role === 'agent-step' && last.stepType === 'thinking') {
          last.content += event.payload;
          messages = [...messages];
        }
        scrollToBottom();
      }
    });
  });

  onDestroy(() => {
    if (unlistenToken) unlistenToken();
    if (unlistenDone) unlistenDone();
    if (unlistenSidebarStep) unlistenSidebarStep();
    if (unlistenSidebarThinking) unlistenSidebarThinking();
  });

  function scrollToBottom() {
    if (messagesEl) {
      requestAnimationFrame(() => {
        messagesEl.scrollTop = messagesEl.scrollHeight;
      });
    }
  }

  // -- Chat (now uses deep context) --
  async function sendMessage() {
    const text = input.trim();
    if (!text || isStreaming) return;

    messages = [...messages, { role: 'user', content: text }];
    messages = [...messages, { role: 'assistant', content: '' }];
    input = '';
    isStreaming = true;
    scrollToBottom();

    try {
      await invoke('grok_chat_deep', { userMessage: text });
    } catch (err) {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.content = `Error: ${err}`;
        messages = [...messages];
      }
      isStreaming = false;
    }
  }

  // -- Quick actions --

  /// Query terminal logs by keyword.
  async function queryLogs() {
    const q = logSearchQuery.trim();
    if (!q || isStreaming) return;

    messages = [...messages, { role: 'user', content: `[Query Logs] ${q}` }];
    messages = [...messages, { role: 'assistant', content: '' }];
    logSearchQuery = '';
    showLogSearch = false;
    isStreaming = true;
    scrollToBottom();

    try {
      await invoke('grok_query_logs', { query: q });
    } catch (err) {
      appendError(err);
    }
  }

  /// Review running processes.
  async function reviewProcesses() {
    if (isStreaming) return;

    messages = [...messages, { role: 'user', content: '[Review Processes]' }];
    messages = [...messages, { role: 'assistant', content: '' }];
    isStreaming = true;
    scrollToBottom();

    try {
      await invoke('grok_review_processes', { filter: null });
    } catch (err) {
      appendError(err);
    }
  }

  /// Suggest workflow improvements.
  async function suggestImprovements() {
    if (isStreaming) return;

    messages = [...messages, { role: 'user', content: '[Suggest Improvements]' }];
    messages = [...messages, { role: 'assistant', content: '' }];
    isStreaming = true;
    scrollToBottom();

    try {
      await invoke('grok_suggest_improvements');
    } catch (err) {
      appendError(err);
    }
  }

  /// Launch a Warpify agent session from the sidebar.
  async function launchWarpify() {
    const text = input.trim();
    if (!text || isStreaming || agentRunning) return;

    messages = [...messages, { role: 'user', content: `[Warpify] ${text}` }];
    input = '';
    agentRunning = true;
    scrollToBottom();

    // Notify parent to optionally show the agent panel as well.
    onwarpify?.();

    try {
      await invoke('sidebar_warpify', { prompt: text });
    } catch (err) {
      messages = [...messages, {
        role: 'agent-step',
        content: `Error: ${err}`,
        stepType: 'error',
      }];
      agentRunning = false;
    }
  }

  // -- Helpers --

  function appendError(err: any) {
    const last = messages[messages.length - 1];
    if (last && last.role === 'assistant') {
      last.content = `Error: ${err}`;
      messages = [...messages];
    }
    isStreaming = false;
  }

  /// Render an agent step as a styled message in the sidebar.
  function renderAgentStepInSidebar(step: string, data: any) {
    switch (step) {
      case 'thinking':
        messages = [...messages, {
          role: 'agent-step',
          content: '',
          stepType: 'thinking',
          stepData: data,
        }];
        break;

      case 'commands': {
        const cmds = data as { command: string; is_destructive: boolean }[];
        const cmdList = cmds.map(c => `  $ ${c.command}${c.is_destructive ? ' [DESTRUCTIVE]' : ''}`).join('\n');
        messages = [...messages, {
          role: 'agent-step',
          content: `Commands to run:\n${cmdList}`,
          stepType: 'commands',
          stepData: data,
        }];
        break;
      }

      case 'executing':
        messages = [...messages, {
          role: 'agent-step',
          content: `> $ ${data.command}`,
          stepType: 'executing',
        }];
        break;

      case 'output': {
        const out = String(data.output || '').slice(0, 2000);
        messages = [...messages, {
          role: 'agent-step',
          content: out,
          stepType: 'output',
        }];
        break;
      }

      case 'done':
        messages = [...messages, {
          role: 'agent-step',
          content: String(data.summary || data),
          stepType: 'done',
        }];
        agentRunning = false;
        break;

      case 'error':
        messages = [...messages, {
          role: 'agent-step',
          content: String(data.error || data),
          stepType: 'error',
        }];
        agentRunning = false;
        break;

      case 'cancelled':
        messages = [...messages, {
          role: 'agent-step',
          content: 'Agent cancelled.',
          stepType: 'cancelled',
        }];
        agentRunning = false;
        break;

      case 'auto-approved': {
        const cmds = data as { command: string }[];
        const cmdList = cmds.map(c => `  $ ${c.command}`).join('\n');
        messages = [...messages, {
          role: 'agent-step',
          content: `Auto-approved:\n${cmdList}`,
          stepType: 'auto-approved',
        }];
        break;
      }

      case 'auto-correcting':
        messages = [...messages, {
          role: 'agent-step',
          content: 'Autocorrecting errors...',
          stepType: 'auto-correcting',
        }];
        break;

      case 'verifying':
        messages = [...messages, {
          role: 'agent-step',
          content: 'Verifying completion...',
          stepType: 'verifying',
        }];
        break;
    }
    scrollToBottom();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }

  function handleLogKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      queryLogs();
    }
    if (e.key === 'Escape') {
      showLogSearch = false;
    }
  }
</script>

<div class="sidebar">
  <div class="sidebar-header">
    <h3>Grok AI</h3>
    <span class="status" class:connected={isConfigured}>
      {isConfigured ? 'Connected' : 'No API Key'}
    </span>
  </div>

  <!-- Quick action toolbar -->
  <div class="toolbar">
    <button
      class="tool-btn"
      title="Search terminal logs"
      onclick={() => (showLogSearch = !showLogSearch)}
      disabled={!isConfigured || isStreaming}
    >
      Query Logs
    </button>
    <button
      class="tool-btn"
      title="Review running processes"
      onclick={reviewProcesses}
      disabled={!isConfigured || isStreaming}
    >
      Processes
    </button>
    <button
      class="tool-btn"
      title="Suggest workflow improvements"
      onclick={suggestImprovements}
      disabled={!isConfigured || isStreaming}
    >
      Suggestions
    </button>
  </div>

  <!-- Log search input (toggle) -->
  {#if showLogSearch}
    <div class="log-search">
      <input
        type="text"
        bind:value={logSearchQuery}
        onkeydown={handleLogKeydown}
        placeholder="Search logs by keyword..."
        autofocus
      />
      <button onclick={queryLogs} disabled={!logSearchQuery.trim()}>Search</button>
    </div>
  {/if}

  <div class="messages" bind:this={messagesEl}>
    {#if messages.length === 0}
      <div class="empty-state">
        <p>Ask Grok anything about your terminal session.</p>
        <p class="hint">Try: "How do I find large files?" or "Explain the last error"</p>
      </div>
    {/if}
    {#each messages as msg}
      {#if msg.role === 'agent-step'}
        <div class="message agent-step step-{msg.stepType || 'default'}">
          <span class="role-tag">Agent</span>
          <pre class="content">{msg.content}</pre>
        </div>
      {:else}
        <div class="message {msg.role}">
          <span class="role-tag">{msg.role === 'user' ? 'You' : 'Grok'}</span>
          <pre class="content">{msg.content}{#if msg.role === 'assistant' && isStreaming && msg === messages[messages.length - 1]}<span class="cursor-blink">|</span>{/if}</pre>
        </div>
      {/if}
    {/each}
  </div>

  <div class="input-area">
    <textarea
      bind:value={input}
      onkeydown={handleKeydown}
      placeholder={isConfigured ? 'Ask Grok...' : 'Set XAI_API_KEY to enable'}
      disabled={!isConfigured || isStreaming}
      rows={2}
    ></textarea>
    <div class="input-buttons">
      <button onclick={sendMessage} disabled={!isConfigured || isStreaming || !input.trim()}>
        Send
      </button>
      <button
        class="warpify-btn"
        onclick={launchWarpify}
        disabled={!isConfigured || isStreaming || agentRunning || !input.trim()}
        title="Escalate to Warpify agent (full tool access)"
      >
        Warpify
      </button>
    </div>
  </div>
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #16161e;
    border-left: 1px solid #292e42;
    color: #c0caf5;
  }

  .sidebar-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid #292e42;
  }

  .sidebar-header h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .status {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 10px;
    background: #f7768e33;
    color: #f7768e;
  }
  .status.connected {
    background: #9ece6a33;
    color: #9ece6a;
  }

  /* -- Toolbar -- */
  .toolbar {
    display: flex;
    gap: 4px;
    padding: 8px 12px;
    border-bottom: 1px solid #292e42;
  }
  .tool-btn {
    background: #292e42;
    color: #a9b1d6;
    border: none;
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 11px;
    font-family: 'JetBrains Mono', monospace;
    cursor: pointer;
    white-space: nowrap;
  }
  .tool-btn:hover:not(:disabled) {
    background: #3b4261;
    color: #c0caf5;
  }
  .tool-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* -- Log search -- */
  .log-search {
    display: flex;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid #292e42;
    background: #1a1b2e;
  }
  .log-search input {
    flex: 1;
    background: #16161e;
    border: 1px solid #292e42;
    border-radius: 4px;
    color: #c0caf5;
    padding: 4px 8px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    outline: none;
  }
  .log-search input:focus {
    border-color: #7aa2f7;
  }
  .log-search button {
    padding: 4px 10px;
    font-size: 11px;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
  }

  .empty-state {
    text-align: center;
    padding: 24px 16px;
    color: #565f89;
  }
  .empty-state .hint {
    font-size: 12px;
    font-style: italic;
  }

  .message {
    margin-bottom: 12px;
    padding: 8px 12px;
    border-radius: 8px;
  }
  .message.user {
    background: #1a1b2e;
  }
  .message.assistant {
    background: #1e2030;
    border-left: 2px solid #7aa2f7;
  }

  /* -- Agent step styles -- */
  .message.agent-step {
    background: #1e2030;
    border-left: 2px solid #bb9af7;
    font-size: 12px;
  }
  .message.step-thinking {
    border-left-color: #bb9af7;
    color: #bb9af7;
    font-style: italic;
  }
  .message.step-commands {
    border-left-color: #e0af68;
  }
  .message.step-executing {
    border-left-color: #7dcfff;
  }
  .message.step-output {
    border-left-color: #565f89;
    color: #a9b1d6;
  }
  .message.step-done {
    border-left-color: #9ece6a;
    color: #9ece6a;
  }
  .message.step-error {
    border-left-color: #f7768e;
    color: #f7768e;
  }
  .message.step-cancelled {
    border-left-color: #e0af68;
    color: #e0af68;
  }
  .message.step-auto-approved {
    border-left-color: #7dcfff;
    color: #7dcfff;
  }
  .message.step-auto-correcting {
    border-left-color: #e0af68;
    font-style: italic;
  }
  .message.step-verifying {
    border-left-color: #7dcfff;
    font-style: italic;
  }

  .role-tag {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    color: #565f89;
    display: block;
    margin-bottom: 4px;
  }

  .content {
    margin: 0;
    white-space: pre-wrap;
    word-wrap: break-word;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 13px;
    line-height: 1.5;
  }

  .cursor-blink {
    animation: blink 1s step-end infinite;
    color: #7aa2f7;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }

  .input-area {
    display: flex;
    gap: 8px;
    padding: 12px;
    border-top: 1px solid #292e42;
  }

  .input-buttons {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  textarea {
    flex: 1;
    background: #1a1b26;
    border: 1px solid #292e42;
    border-radius: 6px;
    color: #c0caf5;
    padding: 8px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    resize: none;
    outline: none;
  }
  textarea:focus {
    border-color: #7aa2f7;
  }

  button {
    background: #7aa2f7;
    color: #1a1b26;
    border: none;
    border-radius: 6px;
    padding: 8px 16px;
    font-weight: 600;
    cursor: pointer;
    font-size: 13px;
  }
  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  button:hover:not(:disabled) {
    background: #89b4fa;
  }

  .warpify-btn {
    background: #7aa2f733;
    color: #7aa2f7;
    font-weight: 700;
    font-size: 11px;
    padding: 4px 12px;
  }
  .warpify-btn:hover:not(:disabled) {
    background: #7aa2f755;
  }
</style>
