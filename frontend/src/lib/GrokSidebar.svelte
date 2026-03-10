<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  interface Message {
    role: 'user' | 'assistant';
    content: string;
  }

  let messages: Message[] = $state([]);
  let input = $state('');
  let isStreaming = $state(false);
  let isConfigured = $state(false);
  let messagesEl: HTMLDivElement;

  let unlistenToken: UnlistenFn;
  let unlistenDone: UnlistenFn;

  onMount(async () => {
    // Check if Grok API key is configured.
    isConfigured = await invoke<boolean>('grok_status');

    // Listen for streaming tokens from the Grok backend.
    unlistenToken = await listen<string>('grok-token', (event) => {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.content += event.payload;
        messages = [...messages]; // trigger reactivity
      }
      scrollToBottom();
    });

    unlistenDone = await listen('grok-done', () => {
      isStreaming = false;
    });
  });

  onDestroy(() => {
    if (unlistenToken) unlistenToken();
    if (unlistenDone) unlistenDone();
  });

  function scrollToBottom() {
    if (messagesEl) {
      messagesEl.scrollTop = messagesEl.scrollHeight;
    }
  }

  async function sendMessage() {
    const text = input.trim();
    if (!text || isStreaming) return;

    messages = [...messages, { role: 'user', content: text }];
    messages = [...messages, { role: 'assistant', content: '' }];
    input = '';
    isStreaming = true;
    scrollToBottom();

    try {
      await invoke('grok_chat', { userMessage: text });
    } catch (err) {
      const last = messages[messages.length - 1];
      if (last && last.role === 'assistant') {
        last.content = `Error: ${err}`;
        messages = [...messages];
      }
      isStreaming = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
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

  <div class="messages" bind:this={messagesEl}>
    {#if messages.length === 0}
      <div class="empty-state">
        <p>Ask Grok anything about your terminal session.</p>
        <p class="hint">Try: "How do I find large files?" or "Explain the last error"</p>
      </div>
    {/if}
    {#each messages as msg}
      <div class="message {msg.role}">
        <span class="role-tag">{msg.role === 'user' ? 'You' : 'Grok'}</span>
        <pre class="content">{msg.content}{#if msg.role === 'assistant' && isStreaming && msg === messages[messages.length - 1]}<span class="cursor-blink">|</span>{/if}</pre>
      </div>
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
    <button onclick={sendMessage} disabled={!isConfigured || isStreaming || !input.trim()}>
      Send
    </button>
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
</style>
