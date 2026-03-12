<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  // Each rendered entry in the activity log.
  interface StepEntry {
    kind: string;
    data: any;
  }

  // --- Reactive state ---
  let visible = $state(false);
  let phase = $state<'idle' | 'thinking' | 'awaiting' | 'executing' | 'done' | 'error' | 'cancelled'>('idle');
  let thinkingBuf = $state('');
  let statusMsg = $state('');
  let entries: StepEntry[] = $state([]);
  let contentEl: HTMLDivElement;

  let unStep: UnlistenFn;
  let unToken: UnlistenFn;
  let spinTimer: ReturnType<typeof setInterval> | null = null;

  // Rotating status messages shown during thinking/API-call phases.
  const spinTexts = ['Analyzing', 'Planning', 'Gathering context', 'Reasoning', 'Processing'];
  let si = 0;
  let dots = 0;

  onMount(async () => {
    unStep = await listen<{ session_id: string; step: string; data: any }>(
      'agent-step',
      (ev) => handleStep(ev.payload.step, ev.payload.data),
    );

    unToken = await listen<string>('agent-thinking-token', (ev) => {
      if (phase === 'thinking') {
        thinkingBuf += ev.payload;
        scroll();
      }
    });
  });

  onDestroy(() => {
    unStep?.();
    unToken?.();
    stopSpin();
  });

  // --- Helpers ---

  function scroll() {
    if (contentEl) requestAnimationFrame(() => (contentEl.scrollTop = contentEl.scrollHeight));
  }

  function startSpin() {
    si = 0;
    dots = 0;
    statusMsg = spinTexts[0];
    stopSpin();
    spinTimer = setInterval(() => {
      dots = (dots + 1) % 4;
      statusMsg = spinTexts[si] + '.'.repeat(dots || 1);
      if (dots === 0) si = (si + 1) % spinTexts.length;
    }, 400);
  }

  function stopSpin() {
    if (spinTimer) {
      clearInterval(spinTimer);
      spinTimer = null;
    }
  }

  // Flush any accumulated thinking text into an entry before changing phase.
  function flushThinking() {
    if (thinkingBuf) {
      entries = [...entries, { kind: 'thinking-text', data: thinkingBuf }];
      thinkingBuf = '';
    }
  }

  // --- Event handler ---

  function handleStep(step: string, data: any) {
    switch (step) {
      case 'thinking':
        // New session — clear previous entries if we were idle/done.
        if (phase === 'idle' || phase === 'done' || phase === 'error' || phase === 'cancelled') {
          entries = [];
          thinkingBuf = '';
        } else {
          flushThinking();
        }
        visible = true;
        phase = 'thinking';
        thinkingBuf = '';
        startSpin();
        break;

      case 'commands':
        flushThinking();
        phase = 'awaiting';
        stopSpin();
        statusMsg = 'Waiting for approval';
        entries = [...entries, { kind: 'commands', data }];
        break;

      case 'auto-approved':
        flushThinking();
        phase = 'executing';
        stopSpin();
        statusMsg = 'Auto-approved';
        entries = [...entries, { kind: 'auto-approved', data }];
        break;

      case 'executing':
        phase = 'executing';
        stopSpin();
        statusMsg = 'Executing: ' + (data.command || '');
        entries = [...entries, { kind: 'executing', data }];
        break;

      case 'output':
        entries = [...entries, { kind: 'output', data }];
        break;

      case 'auto-correcting':
        phase = 'thinking';
        startSpin();
        statusMsg = 'Auto-correcting';
        entries = [...entries, { kind: 'auto-correcting', data }];
        break;

      case 'verifying':
        phase = 'thinking';
        statusMsg = 'Verifying';
        entries = [...entries, { kind: 'verifying', data }];
        break;

      case 'done':
        flushThinking();
        phase = 'done';
        stopSpin();
        statusMsg = 'Complete';
        entries = [...entries, { kind: 'done', data }];
        break;

      case 'cancelled':
        phase = 'cancelled';
        stopSpin();
        statusMsg = 'Cancelled';
        entries = [...entries, { kind: 'cancelled', data }];
        break;

      case 'error':
        phase = 'error';
        stopSpin();
        statusMsg = 'Error';
        entries = [...entries, { kind: 'error', data }];
        break;
    }
    scroll();
  }

  // --- Actions ---

  async function approve() {
    try {
      await invoke('agent_approve');
    } catch {}
  }

  async function cancel() {
    try {
      await invoke('agent_cancel');
    } catch {}
  }

  function dismiss() {
    visible = false;
    phase = 'idle';
    entries = [];
    thinkingBuf = '';
    stopSpin();
  }
</script>

{#if visible}
  <div class="agent-output" class:glow={phase === 'thinking' || phase === 'executing'}>
    <!-- Animated loading bar: sweeps left-to-right during active phases -->
    {#if phase === 'thinking' || phase === 'executing'}
      <div class="loading-bar"></div>
    {/if}

    <!-- Header: status indicator + label + dismiss -->
    <div class="ao-header">
      <div class="ao-status">
        <span
          class="dot"
          class:pulse={phase === 'thinking' || phase === 'executing'}
          class:green={phase === 'done'}
          class:red={phase === 'error'}
          class:yellow={phase === 'awaiting' || phase === 'cancelled'}
        ></span>
        <span class="status-label">Warpify</span>
        <span class="status-msg">{statusMsg}</span>
      </div>
      {#if phase === 'done' || phase === 'error' || phase === 'cancelled'}
        <button class="btn-dismiss" onclick={dismiss}>Dismiss</button>
      {/if}
    </div>

    <!-- Scrollable activity log -->
    <div class="ao-body" bind:this={contentEl}>
      <!-- Live thinking stream -->
      {#if thinkingBuf}
        <pre class="thinking">{thinkingBuf}</pre>
      {/if}

      <!-- Historical entries -->
      {#each entries as e}
        {#if e.kind === 'thinking-text'}
          <pre class="thinking">{e.data}</pre>

        {:else if e.kind === 'commands'}
          <div class="card">
            <div class="card-title">Commands to run:</div>
            {#each e.data as cmd}
              <div class="cmd" class:destructive={cmd.is_destructive}>
                {#if cmd.is_destructive}<span class="badge-warn">[!]</span>{/if}
                <code>$ {cmd.command}</code>
              </div>
            {/each}
          </div>

        {:else if e.kind === 'auto-approved'}
          <div class="card card-auto">
            <div class="card-title">Auto-approved:</div>
            {#each e.data as cmd}
              <div class="cmd"><code>$ {cmd.command}</code></div>
            {/each}
          </div>

        {:else if e.kind === 'executing'}
          <div class="card card-exec">
            <code>$ {e.data.command}</code>
          </div>

        {:else if e.kind === 'output'}
          {#if e.data.output}
            <pre class="output">{e.data.output}</pre>
          {/if}

        {:else if e.kind === 'done'}
          <div class="card card-done">
            <pre class="summary">{e.data.summary || e.data}</pre>
          </div>

        {:else if e.kind === 'error'}
          <div class="card card-error">
            <pre class="error-msg">{e.data.error || JSON.stringify(e.data)}</pre>
          </div>

        {:else if e.kind === 'cancelled'}
          <div class="card card-cancel">Cancelled by user.</div>

        {:else if e.kind === 'auto-correcting'}
          <div class="card card-warn">Auto-correcting errors...</div>

        {:else if e.kind === 'verifying'}
          <div class="card card-info">Verifying task completion...</div>
        {/if}
      {/each}
    </div>

    <!-- Approval action bar (visible only during command approval) -->
    {#if phase === 'awaiting'}
      <div class="ao-approve-bar">
        <button class="btn-approve" onclick={approve}>Approve</button>
        <button class="btn-cancel" onclick={cancel}>Cancel</button>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* --- Container --- */
  .agent-output {
    display: flex;
    flex-direction: column;
    max-height: 45%;
    min-height: 60px;
    background: #0a000f;
    border-bottom: 1px solid #ff007f44;
    animation: ao-in 0.15s ease-out;
    position: relative;
    overflow: hidden;
  }
  .agent-output.glow {
    border-bottom-color: #ff007f66;
    box-shadow: 0 2px 16px rgba(255, 0, 127, 0.2);
  }
  @keyframes ao-in {
    from { opacity: 0; transform: translateY(-6px); }
    to   { opacity: 1; transform: translateY(0); }
  }

  /* --- Animated loading bar --- */
  .loading-bar {
    height: 2px;
    width: 100%;
    background: linear-gradient(
      90deg,
      transparent 0%, #ff007f44 10%, #ff007f 25%,
      #ff6600 40%, #ffe600 50%, #00d4ff 65%, #ff007f 80%, #ff007f44 90%, transparent 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.8s ease-in-out infinite;
    box-shadow: 0 0 8px #ff007f55;
    flex-shrink: 0;
  }
  @keyframes shimmer {
    0%   { background-position: 200% 0; }
    100% { background-position: -200% 0; }
  }

  /* --- Header --- */
  .ao-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 12px;
    border-bottom: 1px solid #200030;
    flex-shrink: 0;
  }
  .ao-status {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  /* Status dot with phase-based colors and pulse animation */
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #660099;
    flex-shrink: 0;
  }
  .dot.pulse {
    background: #ff007f;
    animation: dot-pulse 1.5s ease-in-out infinite;
    box-shadow: 0 0 6px #ff007f88;
  }
  .dot.green  { background: #00ff94; box-shadow: 0 0 6px #00ff9466; }
  .dot.red    { background: #ff1744; box-shadow: 0 0 6px #ff174466; }
  .dot.yellow { background: #ffe600; box-shadow: 0 0 6px #ffe60066; }
  @keyframes dot-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50%      { opacity: 0.4; transform: scale(0.75); }
  }

  .status-label {
    font-size: 12px;
    font-weight: 700;
    color: #ff007f;
    font-family: 'JetBrains Mono', monospace;
  }
  .status-msg {
    font-size: 11px;
    color: #cc44ff;
    font-family: 'JetBrains Mono', monospace;
  }

  .btn-dismiss {
    background: #200038;
    color: #e080ff;
    border: none;
    border-radius: 4px;
    padding: 3px 10px;
    font-size: 11px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
  }
  .btn-dismiss:hover { background: #330055; color: #ff9ef7; }

  /* --- Scrollable body --- */
  .ao-body {
    flex: 1;
    overflow-y: auto;
    padding: 8px 12px;
    min-height: 0;
  }
  .ao-body::-webkit-scrollbar       { width: 6px; }
  .ao-body::-webkit-scrollbar-track  { background: transparent; }
  .ao-body::-webkit-scrollbar-thumb  { background: #ff007f44; border-radius: 3px; }

  /* --- Thinking text (streamed model reasoning) --- */
  .thinking {
    margin: 0 0 8px;
    padding: 6px 8px;
    background: #180028;
    border-left: 2px solid #ff007f44;
    border-radius: 0 4px 4px 0;
    color: #e080ff;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* --- Step cards --- */
  .card {
    margin: 6px 0;
    padding: 8px 10px;
    background: #180028;
    border-radius: 6px;
    border: 1px solid #330044;
  }
  .card-title {
    font-size: 11px;
    font-weight: 600;
    color: #ffe600;
    margin-bottom: 6px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .cmd {
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    color: #00ff94;
    padding: 2px 0;
  }
  .cmd.destructive { color: #ff1744; }
  .badge-warn {
    color: #ff1744;
    font-weight: 700;
    margin-right: 4px;
  }
  .cmd code { background: none; padding: 0; }

  .card-auto    { border-left: 2px solid #00ffff; }
  .card-exec    { border-left: 2px solid #00d4ff; }
  .card-done    { border-left: 2px solid #00ff94; }
  .card-error   { border-left: 2px solid #ff1744; }
  .card-cancel  { border-left: 2px solid #ffe600; color: #ffe600; font-size: 12px; }
  .card-warn    { border-left: 2px solid #ffe600; color: #ffe600; font-size: 12px; }
  .card-info    { border-left: 2px solid #00ffff; color: #00ffff; font-size: 12px; }

  /* --- Command output blocks --- */
  .output {
    margin: 4px 0 8px;
    padding: 6px 8px;
    background: #0a000f;
    border-radius: 4px;
    color: #cc88ff;
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 150px;
    overflow-y: auto;
  }

  .summary {
    margin: 0;
    color: #00ff94;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .error-msg {
    margin: 0;
    color: #ff1744;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }

  /* --- Approval action bar --- */
  .ao-approve-bar {
    display: flex;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid #ff007f44;
    background: #0f0018;
    flex-shrink: 0;
  }
  .btn-approve {
    background: linear-gradient(135deg, #ff007f, #ff6600);
    color: #0a000f;
    border: none;
    border-radius: 6px;
    padding: 6px 20px;
    font-weight: 700;
    font-size: 12px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
  }
  .btn-approve:hover { background: linear-gradient(135deg, #ff3399, #ff8800); }
  .btn-cancel {
    background: #200038;
    color: #ff1744;
    border: none;
    border-radius: 6px;
    padding: 6px 20px;
    font-weight: 600;
    font-size: 12px;
    cursor: pointer;
    font-family: 'JetBrains Mono', monospace;
  }
  .btn-cancel:hover { background: #330055; }
</style>
