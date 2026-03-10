<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  let termEl: HTMLDivElement;
  let term: Terminal;
  let fitAddon: FitAddon;
  let unlistenOutput: UnlistenFn;
  let unlistenExit: UnlistenFn;

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

    // Forward keystrokes from xterm to the PTY backend.
    term.onData((data: string) => {
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
    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      const { cols, rows } = term;
      invoke('resize_pty', { rows, cols });
    });
    resizeObserver.observe(termEl);

    return () => {
      resizeObserver.disconnect();
    };
  });

  onDestroy(() => {
    if (unlistenOutput) unlistenOutput();
    if (unlistenExit) unlistenExit();
    if (term) term.dispose();
  });
</script>

<div class="terminal-container" bind:this={termEl}></div>

<style>
  .terminal-container {
    width: 100%;
    height: 100%;
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
