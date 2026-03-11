<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  // -- Exports for parent component (context menu coordination) --
  interface Props {
    oncontextmenu?: (x: number, y: number, blockId: string) => void;
  }
  let { oncontextmenu }: Props = $props();

  let termEl: HTMLDivElement;
  let term: Terminal;
  let fitAddon: FitAddon;
  let unlistenOutput: UnlistenFn;
  let unlistenExit: UnlistenFn;
  let unlistenAgentStep: UnlistenFn;
  let unlistenAgentToken: UnlistenFn;
  let unlistenInlineToken: UnlistenFn;
  let unlistenInlineDone: UnlistenFn;
  let resizeObs: ResizeObserver | null = null;

  // Mutable agent flags — kept as an object so callback closures always
  // read the current value (plain booleans would be captured by value).
  const agent = { active: false, awaiting: false };

  // -- Inline NL suggestion state --
  // Tracks whether the user is typing a `# ` prefixed natural language query.
  const inline = {
    active: false,       // currently in NL mode
    buffer: '',          // raw NL text after `# `
    suggestion: '',      // streamed suggestion from Grok
    suggesting: false,   // waiting for Grok response
  };

  onMount(async () => {
    // Create xterm instance with a dark theme.
    term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: '"JetBrains Mono", "Fira Code", "Cascadia Code", monospace',
      theme: {
        background: '#1a1b26',
        foreground: '#c0caf5',
        cursor: '#c0caf5',
        selectionBackground: '#33467c',
        black: '#15161e',
        red: '#f7768e',
        green: '#9ece6a',
        yellow: '#e0af68',
        blue: '#7aa2f7',
        magenta: '#bb9af7',
        cyan: '#7dcfff',
        white: '#a9b1d6',
        brightBlack: '#414868',
        brightRed: '#f7768e',
        brightGreen: '#9ece6a',
        brightYellow: '#e0af68',
        brightBlue: '#7aa2f7',
        brightMagenta: '#bb9af7',
        brightCyan: '#7dcfff',
        brightWhite: '#c0caf5',
      },
      allowProposedApi: true,
    });

    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    term.open(termEl);

    // Try loading WebGL renderer for GPU-accelerated drawing.
    try {
      term.loadAddon(new WebglAddon());
    } catch {
      console.warn('WebGL addon unavailable, falling back to canvas renderer');
    }

    fitAddon.fit();

    const { cols, rows } = term;

    // Spawn the PTY backend with the current terminal dimensions.
    await invoke('spawn_pty', { rows, cols });

    // Forward keystrokes — intercept during agent command approval
    // and inline NL suggestion mode.
    term.onData((data: string) => {
      // -- Agent approval intercept --
      if (agent.awaiting) {
        if (data === '\r' || data === '\n') {
          invoke('agent_approve');
          agent.awaiting = false;
          term.write('\r\n');
        } else if (data === '\x1b') {
          invoke('agent_cancel');
          agent.awaiting = false;
          agent.active = false;
        }
        return;
      }

      // -- Inline NL suggestion: accept with Tab, dismiss with Esc --
      if (inline.suggesting && inline.suggestion) {
        if (data === '\t') {
          // Accept suggestion: clear the NL prompt and write the command.
          term.write('\r\n');
          invoke('write_pty', { data: inline.suggestion + '\n' });
          resetInline();
          return;
        }
        if (data === '\x1b') {
          // Dismiss suggestion.
          term.write('\r\n');
          resetInline();
          return;
        }
      }

      // -- Inline NL input: detect `# ` prefix --
      if (inline.active) {
        if (data === '\r' || data === '\n') {
          // Submit the NL query to Grok.
          if (inline.buffer.trim()) {
            triggerInlineSuggest(inline.buffer.trim());
          } else {
            resetInline();
            invoke('write_pty', { data });
          }
          return;
        }
        if (data === '\x7f' || data === '\b') {
          // Backspace.
          if (inline.buffer.length > 0) {
            inline.buffer = inline.buffer.slice(0, -1);
            term.write('\b \b');
          } else {
            // Backspaced past `# ` — exit NL mode.
            resetInline();
          }
          return;
        }
        if (data === '\x1b') {
          // Escape — cancel NL mode.
          term.write('\r\n');
          resetInline();
          return;
        }
        // Accumulate printable characters.
        inline.buffer += data;
        term.write(data);
        return;
      }

      // -- Detect `# ` prefix to enter NL mode --
      if (data === '#') {
        inline.active = true;
        inline.buffer = '';
        inline.suggestion = '';
        inline.suggesting = false;
        term.write(`${P}# ${X}`);
        return;
      }

      invoke('write_pty', { data });
    });

    // Listen for PTY output from the Rust backend.
    unlistenOutput = await listen<string>('pty-output', (event) => {
      term.write(event.payload);
    });

    // Listen for PTY exit.
    unlistenExit = await listen('pty-exit', () => {
      term.write('\r\n[Process exited]\r\n');
    });

    // Handle window resize.
    resizeObs = new ResizeObserver(() => {
      fitAddon.fit();
      const { cols, rows } = term;
      invoke('resize_pty', { rows, cols });
    });
    resizeObs.observe(termEl);

    // ---- Selected text: send highlighted region to Rust for context ----
    term.onSelectionChange(() => {
      const selected = term.getSelection();
      if (selected && selected.length > 0) {
        invoke('set_selected_text', { text: selected });
      } else {
        invoke('set_selected_text', { text: null });
      }
    });

    // ---- Right-click context menu on selected text ----
    termEl.addEventListener('contextmenu', (e: MouseEvent) => {
      const selected = term.getSelection();
      if (selected && selected.length > 0 && oncontextmenu) {
        e.preventDefault();
        // Use the last block's ID for context menu actions.
        invoke<any[]>('get_blocks').then((blocks) => {
          if (blocks.length > 0) {
            oncontextmenu!(e.clientX, e.clientY, blocks[blocks.length - 1].id);
          }
        });
      }
    });

    // ---- Inline NL suggestion streaming events ----
    unlistenInlineToken = await listen<string>('grok-token', (event) => {
      if (inline.suggesting) {
        inline.suggestion += event.payload;
        // Render suggestion in cyan as it streams.
        const text = event.payload.replace(/\n/g, '\r\n');
        term.write(`${C}${text}${X}`);
      }
    });

    unlistenInlineDone = await listen('grok-done', () => {
      if (inline.suggesting) {
        inline.suggesting = false;
        if (inline.suggestion) {
          term.write(`\r\n${D}Tab to accept, Esc to dismiss${X}`);
        }
      }
    });

    // ---- Agent events: render steps inline in the terminal ----
    unlistenAgentStep = await listen<{
      session_id: string;
      step: string;
      data: any;
    }>('agent-step', (event) => {
      renderAgentStep(event.payload.step, event.payload.data);
    });

    unlistenAgentToken = await listen<string>('agent-thinking-token', (event) => {
      // Stream thinking tokens as purple text directly into xterm.
      const text = event.payload.replace(/\n/g, '\r\n');
      term.write(`\x1b[38;5;141m${text}\x1b[0m`);
    });
  });

  onDestroy(() => {
    resizeObs?.disconnect();
    if (unlistenOutput) unlistenOutput();
    if (unlistenExit) unlistenExit();
    if (unlistenAgentStep) unlistenAgentStep();
    if (unlistenAgentToken) unlistenAgentToken();
    if (unlistenInlineToken) unlistenInlineToken();
    if (unlistenInlineDone) unlistenInlineDone();
    if (term) term.dispose();
  });

  // -- Inline NL helpers --

  function resetInline() {
    inline.active = false;
    inline.buffer = '';
    inline.suggestion = '';
    inline.suggesting = false;
  }

  async function triggerInlineSuggest(query: string) {
    inline.suggesting = true;
    inline.suggestion = '';
    term.write(`\r\n${P}Generating command...${X}\r\n`);
    try {
      await invoke('grok_inline_suggest', { partial: query });
    } catch (err) {
      term.write(`\r\n${R}Error: ${err}${X}\r\n`);
      resetInline();
    }
  }

  // ---- Inline agent rendering ----
  //
  // Each agent step is written to xterm.js with ANSI colour codes so the
  // thinking traces, command previews, output, and summary all appear
  // directly in the terminal flow — no popup.

  // ANSI helpers
  const P = '\x1b[38;5;141m'; // purple  (thinking)
  const G = '\x1b[38;5;114m'; // green   (commands / done)
  const Y = '\x1b[38;5;179m'; // yellow  (approval / cancel)
  const R = '\x1b[38;5;210m'; // red     (errors / destructive)
  const C = '\x1b[38;5;81m';  // cyan    (executing)
  const D = '\x1b[2m';        // dim
  const B = '\x1b[1m';        // bold
  const X = '\x1b[0m';        // reset

  function renderAgentStep(step: string, data: any) {
    switch (step) {
      case 'thinking':
        agent.active = true;
        term.write(`\r\n${P}${B}-- Warpify -----------------------------------${X}\r\n`);
        break;

      case 'commands': {
        term.write(`\r\n${Y}${B}Commands to run:${X}\r\n`);
        const cmds = data as { command: string; is_destructive: boolean }[];
        for (const cmd of cmds) {
          if (cmd.is_destructive) {
            term.write(`${R}  [!] $ ${cmd.command}${X}\r\n`);
          } else {
            term.write(`${G}  $ ${cmd.command}${X}\r\n`);
          }
        }
        term.write(`\r\n${D}Press Enter to run, Esc to cancel${X}`);
        agent.awaiting = true;
        break;
      }

      case 'executing':
        term.write(`\r\n${C}>${X} ${G}$ ${data.command}${X}\r\n`);
        break;

      case 'output': {
        const out = String(data.output || '').replace(/\n/g, '\r\n');
        term.write(`${out}\r\n`);
        break;
      }

      case 'done': {
        const summary = String(data.summary || data).replace(/\n/g, '\r\n');
        term.write(`\r\n${G}${B}-- Done --------------------------------------${X}\r\n`);
        term.write(`${G}${summary}${X}\r\n\r\n`);
        agent.active = false;
        agent.awaiting = false;
        break;
      }

      case 'cancelled':
        term.write(`\r\n${Y}-- Cancelled --${X}\r\n\r\n`);
        agent.active = false;
        agent.awaiting = false;
        break;

      case 'error': {
        const err = String(data.error || data).replace(/\n/g, '\r\n');
        term.write(`\r\n${R}${B}-- Error --${X}\r\n`);
        term.write(`${R}${err}${X}\r\n\r\n`);
        agent.active = false;
        agent.awaiting = false;
        break;
      }

      // ---- Autocorrect step types ----

      case 'auto-approved': {
        const cmds = data as { command: string; is_destructive: boolean }[];
        term.write(`\r\n${C}${B}-- Auto-approved --${X}\r\n`);
        for (const cmd of cmds) {
          term.write(`${G}  $ ${cmd.command}${X}\r\n`);
        }
        // No awaiting — commands execute immediately.
        break;
      }

      case 'auto-correcting': {
        term.write(`\r\n${Y}${B}-- Autocorrecting errors --${X}\r\n`);
        break;
      }

      case 'verifying': {
        term.write(`\r\n${C}${B}-- Verifying completion --${X}\r\n`);
        break;
      }
    }
  }
</script>

<div class="terminal-container" bind:this={termEl}></div>

<style>
  .terminal-container {
    width: 100%;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  /* Import xterm CSS */
  :global(.xterm) {
    padding: 8px;
    height: 100%;
  }
  :global(.xterm-viewport) {
    overflow-y: auto !important;
  }
</style>
