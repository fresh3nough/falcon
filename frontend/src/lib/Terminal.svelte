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
  let unlistenInlineToken: UnlistenFn;
  let unlistenInlineDone: UnlistenFn;
  let resizeObs: ResizeObserver | null = null;

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
        background: '#0a000f',
        foreground: '#ff9ef7',
        cursor: '#ff007f',
        selectionBackground: '#ff007f44',
        black: '#0f0018',
        red: '#ff1744',
        green: '#00ff94',
        yellow: '#ffe600',
        blue: '#00d4ff',
        magenta: '#ff007f',
        cyan: '#00ffff',
        white: '#ff9ef7',
        brightBlack: '#4d0066',
        brightRed: '#ff4466',
        brightGreen: '#00ff94',
        brightYellow: '#ffff33',
        brightBlue: '#66e0ff',
        brightMagenta: '#ff66b3',
        brightCyan: '#66ffff',
        brightWhite: '#ffffff',
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

    // Forward keystrokes — intercept during inline NL suggestion mode.
    term.onData((data: string) => {
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
  });

  onDestroy(() => {
    resizeObs?.disconnect();
    if (unlistenOutput) unlistenOutput();
    if (unlistenExit) unlistenExit();
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

  // ANSI helpers (used by inline NL suggestions)
  const P = '\x1b[38;2;255;0;127m';   // hot pink
  const R = '\x1b[38;2;255;23;68m';   // neon red
  const C = '\x1b[38;2;0;212;255m';   // electric cyan
  const D = '\x1b[2m';                // dim
  const X = '\x1b[0m';                // reset
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
