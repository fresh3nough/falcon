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

  // -- Role pipeline state --

  interface RoleState {
    id: string;
    label: string;
    status: 'idle' | 'running' | 'done' | 'error';
    output: string;
    summary: string;
    toolCalls: { tool: string; label: string; outputPreview: string }[];
  }

  const ROLES = ['researcher', 'architect', 'implementer', 'reviewer'];

  function freshRoles(): RoleState[] {
    return ROLES.map(r => ({
      id: r,
      label: r.charAt(0).toUpperCase() + r.slice(1),
      status: 'idle',
      output: '',
      summary: '',
      toolCalls: [],
    }));
  }

  let taskInput = $state('');
  let isRunning = $state(false);
  let roles: RoleState[] = $state(freshRoles());
  let finalSummary = $state('');
  let errorMsg = $state('');
  let approvalPending = $state(false);
  let approvalLabel = $state('');
  let approvalDestructive = $state(false);

  let unlistenStep: UnlistenFn;
  let unlistenResearcher: UnlistenFn;
  let unlistenArchitect: UnlistenFn;
  let unlistenImplementer: UnlistenFn;
  let unlistenReviewer: UnlistenFn;

  function getRole(id: string): RoleState | undefined {
    return roles.find(r => r.id === id);
  }

  onMount(async () => {
    // Listen for orchestrator step events.
    unlistenStep = await listen<any>('orchestrator-step', (event) => {
      const { role, step, data } = event.payload;

      if (step === 'started') {
        const r = getRole(role);
        if (r) { r.status = 'running'; roles = [...roles]; }
      } else if (step === 'done') {
        const r = getRole(role);
        if (r) {
          r.status = 'done';
          r.summary = data?.summary || '';
          roles = [...roles];
        }
      } else if (step === 'tool_call') {
        const r = getRole(role);
        if (r) {
          r.toolCalls = [...r.toolCalls, {
            tool: data.tool,
            label: data.label,
            outputPreview: data.output_preview || '',
          }];
          roles = [...roles];
        }
      } else if (step === 'approval_needed') {
        approvalPending = true;
        approvalLabel = data.label || data.tool;
        approvalDestructive = data.is_destructive || false;
      } else if (step === 'complete') {
        finalSummary = data?.summary || 'Pipeline complete.';
        isRunning = false;
      } else if (step === 'error') {
        errorMsg = data?.error || 'Unknown error';
        isRunning = false;
      }
    });

    // Listen for per-role thinking tokens.
    unlistenResearcher = await listen<string>('orchestrator-researcher-token', (e) => {
      const r = getRole('researcher');
      if (r) { r.output += e.payload; roles = [...roles]; }
    });
    unlistenArchitect = await listen<string>('orchestrator-architect-token', (e) => {
      const r = getRole('architect');
      if (r) { r.output += e.payload; roles = [...roles]; }
    });
    unlistenImplementer = await listen<string>('orchestrator-implementer-token', (e) => {
      const r = getRole('implementer');
      if (r) { r.output += e.payload; roles = [...roles]; }
    });
    unlistenReviewer = await listen<string>('orchestrator-reviewer-token', (e) => {
      const r = getRole('reviewer');
      if (r) { r.output += e.payload; roles = [...roles]; }
    });
  });

  onDestroy(() => {
    if (unlistenStep) unlistenStep();
    if (unlistenResearcher) unlistenResearcher();
    if (unlistenArchitect) unlistenArchitect();
    if (unlistenImplementer) unlistenImplementer();
    if (unlistenReviewer) unlistenReviewer();
  });

  async function startPipeline() {
    const task = taskInput.trim();
    if (!task || isRunning) return;

    // Reset state.
    roles = freshRoles();
    finalSummary = '';
    errorMsg = '';
    isRunning = true;

    try {
      await invoke('orchestrate_task', { task });
    } catch (err) {
      errorMsg = `${err}`;
      isRunning = false;
    }
  }

  async function handleApprove() {
    approvalPending = false;
    try { await invoke('agent_approve'); } catch (_) { /* ignore */ }
  }

  async function handleCancel() {
    approvalPending = false;
    try { await invoke('agent_cancel'); } catch (_) { /* ignore */ }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      startPipeline();
    }
    if (e.key === 'Escape') {
      onclose();
    }
  }

  const statusDot: Record<string, string> = {
    idle: '#565f89',
    running: '#e0af68',
    done: '#9ece6a',
    error: '#f7768e',
  };
</script>

{#if visible}
  <div class="orch-panel">
    <div class="orch-header">
      <h3>Multi-Agent Orchestrator</h3>
      <button class="close-btn" onclick={onclose}>x</button>
    </div>

    <!-- Task input -->
    <div class="task-input-area">
      <textarea
        bind:value={taskInput}
        onkeydown={handleKeydown}
        placeholder="Describe a complex task... (Ctrl+Enter to run)"
        disabled={isRunning}
        rows={3}
      ></textarea>
      <button class="run-btn" onclick={startPipeline} disabled={isRunning || !taskInput.trim()}>
        {isRunning ? 'Running...' : 'Run Pipeline'}
      </button>
    </div>

    <!-- Role cards -->
    <div class="role-cards">
      {#each roles as role}
        <div class="role-card" class:active={role.status === 'running'}>
          <div class="role-header">
            <span class="role-dot" style="background: {statusDot[role.status]}"></span>
            <span class="role-name">{role.label}</span>
            <span class="role-status">{role.status}</span>
          </div>

          {#if role.output}
            <pre class="role-output">{role.output}</pre>
          {/if}

          {#if role.toolCalls.length > 0}
            <div class="tool-log">
              {#each role.toolCalls as tc}
                <div class="tool-entry">
                  <span class="tool-name">{tc.tool}</span>
                  <span class="tool-label">{tc.label}</span>
                </div>
              {/each}
            </div>
          {/if}

          {#if role.summary}
            <div class="role-summary">{role.summary}</div>
          {/if}
        </div>
      {/each}
    </div>

    <!-- Approval dialog -->
    {#if approvalPending}
      <div class="approval-bar">
        <span class="approval-label" class:destructive={approvalDestructive}>
          {approvalDestructive ? 'DESTRUCTIVE: ' : ''}{approvalLabel}
        </span>
        <button class="approve-btn" onclick={handleApprove}>Approve</button>
        <button class="cancel-btn" onclick={handleCancel}>Cancel</button>
      </div>
    {/if}

    <!-- Final summary / error -->
    {#if finalSummary}
      <div class="final-bar done">
        <pre>{finalSummary}</pre>
      </div>
    {/if}
    {#if errorMsg}
      <div class="final-bar error">{errorMsg}</div>
    {/if}
  </div>
{/if}

<style>
  .orch-panel {
    display: flex;
    flex-direction: column;
    background: #16161e;
    border: 1px solid #292e42;
    border-radius: 10px;
    margin: 8px;
    max-height: 70vh;
    overflow-y: auto;
  }

  .orch-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 14px;
    border-bottom: 1px solid #292e42;
  }
  .orch-header h3 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: #c0caf5;
  }
  .close-btn {
    background: transparent;
    border: none;
    color: #565f89;
    font-size: 16px;
    cursor: pointer;
  }

  .task-input-area {
    display: flex;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid #292e42;
  }
  .task-input-area textarea {
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
  .task-input-area textarea:focus {
    border-color: #7aa2f7;
  }
  .run-btn {
    background: #bb9af7;
    color: #1a1b26;
    border: none;
    border-radius: 6px;
    padding: 8px 14px;
    font-weight: 600;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .run-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .role-cards {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
  }

  .role-card {
    background: #1a1b26;
    border: 1px solid #292e42;
    border-radius: 8px;
    padding: 8px 10px;
    transition: border-color 0.2s;
  }
  .role-card.active {
    border-color: #e0af68;
  }

  .role-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .role-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .role-name {
    font-weight: 600;
    font-size: 12px;
    color: #c0caf5;
  }
  .role-status {
    font-size: 11px;
    color: #565f89;
    margin-left: auto;
  }

  .role-output {
    margin: 4px 0;
    font-size: 11px;
    line-height: 1.4;
    color: #a9b1d6;
    white-space: pre-wrap;
    word-wrap: break-word;
    max-height: 120px;
    overflow-y: auto;
    font-family: 'JetBrains Mono', monospace;
  }

  .tool-log {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 4px 0;
  }
  .tool-entry {
    display: flex;
    gap: 6px;
    font-size: 11px;
    color: #7dcfff;
  }
  .tool-name {
    font-weight: 600;
    color: #7aa2f7;
  }
  .tool-label {
    color: #a9b1d6;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .role-summary {
    font-size: 11px;
    color: #9ece6a;
    margin-top: 4px;
    padding: 4px 6px;
    background: #9ece6a11;
    border-radius: 4px;
  }

  .approval-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    background: #e0af6822;
    border-top: 1px solid #e0af68;
  }
  .approval-label {
    flex: 1;
    font-size: 12px;
    color: #e0af68;
    font-family: 'JetBrains Mono', monospace;
  }
  .approval-label.destructive {
    color: #f7768e;
  }
  .approve-btn {
    background: #9ece6a;
    color: #1a1b26;
    border: none;
    border-radius: 4px;
    padding: 4px 12px;
    font-weight: 600;
    cursor: pointer;
    font-size: 12px;
  }
  .cancel-btn {
    background: #f7768e;
    color: #1a1b26;
    border: none;
    border-radius: 4px;
    padding: 4px 12px;
    font-weight: 600;
    cursor: pointer;
    font-size: 12px;
  }

  .final-bar {
    padding: 10px 14px;
    font-size: 12px;
    border-top: 1px solid #292e42;
  }
  .final-bar.done {
    color: #9ece6a;
    background: #9ece6a0a;
  }
  .final-bar.done pre {
    margin: 0;
    white-space: pre-wrap;
    word-wrap: break-word;
    font-family: 'JetBrains Mono', monospace;
    font-size: 12px;
  }
  .final-bar.error {
    color: #f7768e;
    background: #f7768e0a;
  }
</style>
