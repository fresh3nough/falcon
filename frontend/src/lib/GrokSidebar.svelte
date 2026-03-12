<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  interface ImageRef {
    dataUrl: string;
    name: string;
  }

  interface Message {
    role: 'user' | 'assistant';
    content: string;
    images?: ImageRef[];
  }

  let messages: Message[] = $state([]);
  let input = $state('');
  let isStreaming = $state(false);
  let isConfigured = $state(false);
  let messagesEl: HTMLDivElement;
  let pendingImages: ImageRef[] = $state([]);
  let sidebarEl: HTMLDivElement;

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

  // -- Send message --

  async function sendMessage() {
    const text = input.trim();
    if (!text || isStreaming) return;

    const images = [...pendingImages];
    messages = [...messages, { role: 'user', content: text, images: images.length ? images : undefined }];
    messages = [...messages, { role: 'assistant', content: '' }];
    input = '';
    pendingImages = [];
    isStreaming = true;
    scrollToBottom();

    try {
      if (images.length > 0) {
        // Use vision endpoint when images are attached.
        await invoke('grok_vision_chat', {
          userMessage: text,
          imageDataUrls: images.map(img => img.dataUrl),
        });
      } else {
        await invoke('grok_chat', { userMessage: text });
      }
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

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="sidebar"
  bind:this={sidebarEl}
  ondrop={handleDrop}
  ondragover={handleDragOver}
  onpaste={handlePaste}
>
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
        {#if msg.images && msg.images.length > 0}
          <div class="msg-images">
            {#each msg.images as img}
              <img src={img.dataUrl} alt={img.name} class="msg-thumb" title={img.name} />
            {/each}
          </div>
        {/if}
        <pre class="content">{msg.content}{#if msg.role === 'assistant' && isStreaming && msg === messages[messages.length - 1]}<span class="cursor-blink">|</span>{/if}</pre>
      </div>
    {/each}
  </div>

  {#if pendingImages.length > 0}
    <div class="pending-images">
      {#each pendingImages as img, idx}
        <div class="pending-thumb-wrap">
          <img src={img.dataUrl} alt={img.name} class="pending-thumb" />
          <button class="remove-img" onclick={() => removePendingImage(idx)}>x</button>
        </div>
      {/each}
    </div>
  {/if}

  <div class="input-area">
    <textarea
      bind:value={input}
      onkeydown={handleKeydown}
      placeholder={isConfigured ? 'Ask Grok... (drop/paste images)' : 'Set XAI_API_KEY to enable'}
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
    background: #0f0018;
    border-left: 1px solid #ff007f44;
    color: #ff9ef7;
  }

  .sidebar-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid #ff007f44;
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
    background: #ff174433;
    color: #ff1744;
  }
  .status.connected {
    background: #00ff9433;
    color: #00ff94;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 12px;
  }

  .empty-state {
    text-align: center;
    padding: 24px 16px;
    color: #9933cc;
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
    background: #180028;
  }
  .message.assistant {
    background: #1a0030;
    border-left: 2px solid #00d4ff;
  }

  .role-tag {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    color: #cc44ff;
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
    color: #ff007f;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }

  .input-area {
    display: flex;
    gap: 8px;
    padding: 12px;
    border-top: 1px solid #ff007f44;
  }

  textarea {
    flex: 1;
    background: #0a000f;
    border: 1px solid #330044;
    border-radius: 6px;
    color: #ff9ef7;
    padding: 8px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    resize: none;
    outline: none;
  }
  textarea:focus {
    border-color: #ff007f;
  }

  button {
    background: #ff007f;
    color: #0a000f;
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
    background: #ff3399;
  }

  /* Image thumbnails in messages */
  .msg-images {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-bottom: 6px;
  }
  .msg-thumb {
    width: 64px;
    height: 64px;
    object-fit: cover;
    border-radius: 6px;
    border: 1px solid #292e42;
    cursor: pointer;
  }
  .msg-thumb:hover {
    border-color: #7aa2f7;
  }

  /* Pending images strip above input */
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
    border: 1px solid #292e42;
  }
  .remove-img {
    position: absolute;
    top: -4px;
    right: -4px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #f7768e;
    color: #1a1b26;
    border: none;
    font-size: 10px;
    line-height: 16px;
    text-align: center;
    cursor: pointer;
    padding: 0;
  }
</style>
