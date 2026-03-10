<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  interface Props {
    visible: boolean;
    onclose: () => void;
  }

  let { visible = $bindable(), onclose }: Props = $props();

  // ---- State ---------------------------------------------------------------

  let input = $state('');
  let isRunning = $state(false);
  let isConfigured = $state(false);
  let sessionId = $state('');

  /** Streaming thinking text for the current iteration. */
  let thinkingText = $state('');

  /** Whether the agent is waiting for the user to approve commands. */
  let awaitingApproval = $state(false);

  /** Commands pending approval in the current iteration. */
  interface CommandPreview {
    tool_call_id: string;
    command: string;
    is_destructive: boolean;
  }
  let pendingCommands: CommandPreview[] = $state([]);

  /** Chronological list of completed agent steps for display. */
  interface AgentStep {
    type: string;
    data: any;
    ts: number;
  }
  let steps: AgentStep[] = $state([]);

  let stepsEl: HTMLDivElement = $state(null!);
  let inputEl: HTMLInputElement = $state(null!);

  // ---- Event listeners -----------------------------------------------------

  let unlistenStep: UnlistenFn;
  let unlistenToken: UnlistenFn;

  onMount(async () => {
    isConfigured = await invoke<boolean>('grok_status');

    unlistenStep = await listen<{
      session_id: string;
      step: string;
      data: any;
    }>('agent-step', (event) => {
      const { step, data } = event.payload;

      if (step === 'thinking') {
        // Reset streaming text for a new thinking phase.
        thinkingText = '';
      } else if (step === 'commands') {
        pendingCommands = data as CommandPreview[];
        awaitingApproval = true;
        // Also save the thinking text we collected as a step.
        if (thinkingText.trim()) {
          steps = [...steps, { type: 'thinking', data: thinkingText.trim(), ts: Date.now() }];
          thinkingText = '';
        }
      } else if (step === 'executing') {
        awaitingApproval = false;
        pendingCommands = [];
        steps = [...steps, { type: 'executing', data, ts: Date.now() }];
      } else if (step === 'output') {
        steps = [...steps, { type: 'output', data, ts: Date.now() }];
      } else if (step === 'done') {
        // Flush any remaining thinking text.
        if (thinkingText.trim()) {
          steps = [...steps, { type: 'thinking', data: thinkingText.trim(), ts: Date.now() }];
          thinkingText = '';
        }
        steps = [...steps, { type: 'done', data, ts: Date.now() }];
        isRunning = false;
        awaitingApproval = false;
        pendingCommands = [];
      } else if (step === 'cancelled') {
        steps = [...steps, { type: 'cancelled', data, ts: Date.now() }];
        isRunning = false;
        awaitingApproval = false;
        pendingCommands = [];
      } else if (step === 'error') {
        steps = [...steps, { type: 'error', data, ts: Date.now() }];
        isRunning = false;
        awaitingApproval = false;
        pendingCommands = [];
      }

      scrollToBottom();
    });

    unlistenToken = await listen<string>('agent-thinking-token', (event) => {
      thinkingText += event.payload;
      scrollToBottom();
    });
  });

  onDestroy(() => {
    if (unlistenStep) unlistenStep();
    if (unlistenToken) unlistenToken();
  });

  // ---- Helpers -------------------------------------------------------------

  function scrollToBottom() {
    requestAnimationFrame(() => {
      if (stepsEl) stepsEl.scrollTop = stepsEl.scrollHeight;
    });
  }

  // ---- Actions -------------------------------------------------------------

  async function submitPrompt() {
    const text = input.trim();
    if (!text || isRunning) return;

    // Reset for a new session.
    steps = [];
    thinkingText = '';
    pendingCommands = [];
    awaitingApproval = false;
    isRunning = true;

    steps = [{ type: 'user', data: text, ts: Date.now() }];
    input = '';

    try {
      sessionId = await invoke<string>('agent_run', { prompt: text });
    } catch (err) {
      steps = [...steps, { type: 'error', data: { error: String(err) }, ts: Date.now() }];
      isRunning = false;
    }
  }

  async function approveCommands() {
    try {
      await invoke('agent_approve');
    } catch (err) {
      console.error('Approval failed:', err);
    }
  }

  async function cancelAgent() {
    try {
      await invoke('agent_cancel');
    } catch (err) {
      console.error('Cancel failed:', err);
    }
  }

  function close() {
    if (isRunning) cancelAgent();
    visible = false;
    onclose();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submitPrompt();
    }
    if (e.key === 'Escape') {
      if (isRunning) {
        cancelAgent();
      } else {
        close();
      }
    }
  }

  // Focus the input when the panel opens.
  $effect(() => {
    if (visible && inputEl) {
      requestAnimationFrame(() => inputEl?.focus());
    }
  });
</script>

{#if visible}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="agent-overlay" onkeydown={handleKeydown}>
    <div class="agent-panel">
      <!-- Header -->
      <div class="agent-header">
        <div class="agent-title">
          <span class="agent-badge">AGENT</span>
          Warpify
        </div>
        <button class="close-btn" onclick={close} title="Close (Esc)">x</button>
      </div>

      <!-- Steps feed -->
      <div class="agent-steps" bind:this={stepsEl}>
        {#each steps as s}
          <!-- User prompt -->
          {#if s.type === 'user'}
            <div class="step step-user">
              <span class="step-label">YOU</span>
              <pre class="step-text">{s.data}</pre>
            </div>

          <!-- Thinking (completed) -->
          {:else if s.type === 'thinking'}
            <div class="step step-thinking">
              <span class="step-label">PLANNING</span>
              <pre class="step-text">{s.data}</pre>
            </div>

          <!-- Executing -->
          {:else if s.type === 'executing'}
            <div class="step step-executing">
              <span class="step-label">RUNNING</span>
              <code class="step-cmd">$ {s.data.command}</code>
            </div>

          <!-- Output -->
          {:else if s.type === 'output'}
            <div class="step step-output">
              <span class="step-label">OUTPUT</span>
              <code class="step-cmd">$ {s.data.command}</code>
              <pre class="step-text output-text">{s.data.output}</pre>
            </div>

          <!-- Done -->
          {:else if s.type === 'done'}
            <div class="step step-done">
              <span class="step-label">DONE</span>
              <pre class="step-text">{s.data.summary ?? s.data}</pre>
            </div>

          <!-- Cancelled -->
          {:else if s.type === 'cancelled'}
            <div class="step step-cancelled">
              <span class="step-label">CANCELLED</span>
              <pre class="step-text">{s.data}</pre>
            </div>

          <!-- Error -->
          {:else if s.type === 'error'}
            <div class="step step-error">
              <span class="step-label">ERROR</span>
              <pre class="step-text">{s.data.error ?? s.data}</pre>
            </div>
          {/if}
        {/each}

        <!-- Live thinking (streaming) -->
        {#if isRunning && thinkingText}
          <div class="step step-thinking live">
            <span class="step-label">THINKING</span>
            <pre class="step-text">{thinkingText}<span class="cursor-blink">|</span></pre>
          </div>
        {/if}

        <!-- Command approval pane -->
        {#if awaitingApproval && pendingCommands.length > 0}
          <div class="step step-commands">
            <span class="step-label">COMMANDS TO RUN</span>
            {#each pendingCommands as cmd}
              <div class="cmd-preview" class:destructive={cmd.is_destructive}>
                <code>$ {cmd.command}</code>
                {#if cmd.is_destructive}
                  <span class="destructive-badge">DESTRUCTIVE</span>
                {/if}
              </div>
            {/each}
            <div class="approval-actions">
              <button class="btn-approve" onclick={approveCommands}>
                Run All
              </button>
              <button class="btn-cancel" onclick={cancelAgent}>
                Cancel
              </button>
            </div>
          </div>
        {/if}
      </div>

      <!-- Input bar -->
      <div class="agent-input">
        <span class="hash-prefix">#</span>
        <input
          type="text"
          bind:this={inputEl}
          bind:value={input}
          onkeydown={handleKeydown}
          placeholder={isConfigured
            ? 'Describe what you need in plain English...'
            : 'Set XAI_API_KEY to enable agent'}
          disabled={!isConfigured || isRunning}
        />
        <button
          onclick={submitPrompt}
          disabled={!isConfigured || isRunning || !input.trim()}
        >
          Go
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay ---------------------------------------------------------- */
  .agent-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    justify-content: center;
    align-items: flex-end;
    z-index: 900;
    padding: 24px;
  }

  .agent-panel {
    display: flex;
    flex-direction: column;
    width: 700px;
    max-width: 90vw;
    max-height: 75vh;
    background: #1a1b26;
    border: 1px solid #292e42;
    border-radius: 14px;
    box-shadow: 0 24px 80px rgba(0, 0, 0, 0.6);
    overflow: hidden;
  }

  /* ---- Header ----------------------------------------------------------- */
  .agent-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 12px 16px;
    border-bottom: 1px solid #292e42;
    background: #16161e;
  }
  .agent-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 600;
    color: #c0caf5;
  }
  .agent-badge {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    padding: 2px 7px;
    border-radius: 4px;
    background: #7aa2f7;
    color: #1a1b26;
  }
  .close-btn {
    background: none;
    border: none;
    color: #565f89;
    font-size: 16px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 4px;
  }
  .close-btn:hover {
    background: #292e42;
    color: #c0caf5;
  }

  /* ---- Steps feed ------------------------------------------------------- */
  .agent-steps {
    flex: 1;
    overflow-y: auto;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .step {
    border-left: 3px solid #292e42;
    padding: 8px 12px;
    border-radius: 0 8px 8px 0;
    background: #16161e;
  }
  .step-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    display: block;
    margin-bottom: 4px;
    color: #565f89;
  }
  .step-text {
    margin: 0;
    white-space: pre-wrap;
    word-wrap: break-word;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 13px;
    line-height: 1.5;
    color: #c0caf5;
  }
  .step-cmd {
    display: block;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    color: #9ece6a;
    margin-top: 2px;
  }

  /* Step variants */
  .step-user {
    border-left-color: #7aa2f7;
  }
  .step-user .step-label { color: #7aa2f7; }

  .step-thinking {
    border-left-color: #bb9af7;
  }
  .step-thinking .step-label { color: #bb9af7; }
  .step-thinking.live {
    border-left-color: #bb9af7;
    background: #1e1e2e;
  }

  .step-commands {
    border-left-color: #e0af68;
  }
  .step-commands .step-label { color: #e0af68; }

  .step-executing {
    border-left-color: #7dcfff;
  }
  .step-executing .step-label { color: #7dcfff; }

  .step-output {
    border-left-color: #565f89;
  }
  .output-text {
    max-height: 200px;
    overflow-y: auto;
    background: #13131d;
    border-radius: 4px;
    padding: 6px 8px;
    margin-top: 4px;
    font-size: 12px;
    color: #a9b1d6;
  }

  .step-done {
    border-left-color: #9ece6a;
    background: #1a2420;
  }
  .step-done .step-label { color: #9ece6a; }

  .step-cancelled {
    border-left-color: #e0af68;
  }
  .step-cancelled .step-label { color: #e0af68; }

  .step-error {
    border-left-color: #f7768e;
    background: #221a1e;
  }
  .step-error .step-label { color: #f7768e; }

  /* ---- Command previews ------------------------------------------------- */
  .cmd-preview {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
    padding: 6px 10px;
    border-radius: 6px;
    background: #1a1b26;
    border: 1px solid #292e42;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    color: #9ece6a;
  }
  .cmd-preview.destructive {
    border-color: #f7768e44;
    color: #f7768e;
  }
  .destructive-badge {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 6px;
    border-radius: 3px;
    background: #f7768e33;
    color: #f7768e;
    white-space: nowrap;
  }

  /* ---- Approval buttons ------------------------------------------------- */
  .approval-actions {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }
  .btn-approve {
    background: #9ece6a;
    color: #1a1b26;
    border: none;
    border-radius: 6px;
    padding: 8px 20px;
    font-weight: 700;
    font-size: 13px;
    cursor: pointer;
  }
  .btn-approve:hover { background: #b4e88d; }
  .btn-cancel {
    background: #292e42;
    color: #a9b1d6;
    border: none;
    border-radius: 6px;
    padding: 8px 20px;
    font-weight: 600;
    font-size: 13px;
    cursor: pointer;
  }
  .btn-cancel:hover { background: #3b4261; color: #c0caf5; }

  /* ---- Input bar -------------------------------------------------------- */
  .agent-input {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
    border-top: 1px solid #292e42;
    background: #16161e;
  }
  .hash-prefix {
    font-family: 'JetBrains Mono', monospace;
    font-size: 16px;
    font-weight: 700;
    color: #7aa2f7;
  }
  .agent-input input {
    flex: 1;
    background: #1a1b26;
    border: 1px solid #292e42;
    border-radius: 6px;
    color: #c0caf5;
    padding: 8px 12px;
    font-family: 'JetBrains Mono', monospace;
    font-size: 13px;
    outline: none;
  }
  .agent-input input:focus {
    border-color: #7aa2f7;
  }
  .agent-input button {
    background: #7aa2f7;
    color: #1a1b26;
    border: none;
    border-radius: 6px;
    padding: 8px 16px;
    font-weight: 700;
    font-size: 13px;
    cursor: pointer;
  }
  .agent-input button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .agent-input button:hover:not(:disabled) {
    background: #89b4fa;
  }

  /* ---- Misc ------------------------------------------------------------- */
  .cursor-blink {
    animation: blink 1s step-end infinite;
    color: #bb9af7;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }
</style>
