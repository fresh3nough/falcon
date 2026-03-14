# Falcon

A modern, Warp-style AI terminal powered by xAI. Built with Tauri 2.0 (Rust backend + Svelte frontend), featuring a full PTY, block-based output, GPU-accelerated rendering via xterm.js/WebGL, and an integrated AI sidebar for command generation, explanation, and fixing. Falcon is modeled after Warp and xAI's agentic architecture to ensure compatibility with xAI's ecosystem, as Warp is not compatible.

![HDKjzrAXsAA94GV](https://github.com/user-attachments/assets/626c4879-5209-4c37-b90a-c478a75577c0)

## Features

- **Full PTY** -- spawns bash/zsh/fish with signal handling, resize, and raw mode via `portable-pty`
- **Block-based output** -- commands and their output are grouped into discrete, copyable blocks
- **GPU-accelerated rendering** -- xterm.js with WebGL addon for smooth 60fps scrolling
- **AI integration** -- streaming chat completions via the xAI API (`/v1/chat/completions`)
  - Inline command generation from natural language
  - Explain any block's output
  - Fix failed commands
  - Persistent sidebar chat with full session context (cwd, git status, recent commands)
- **Command palette** -- `Ctrl+K` to access quick actions
- **Cross-platform** -- macOS, Linux, Windows (via Tauri)

## Architecture

```
User Input
     |
Tauri Events (IPC)
     |
Rust Backend (src-tauri/)
  |-- pty.rs          PTY Manager (portable-pty)
  |-- ai.rs           xAI streaming client (reqwest + SSE)
  |-- block.rs        Block Manager (command+output grouping)
  |-- context.rs      Context Collector (cwd, git, history)
  |-- lib.rs          Tauri command handlers + app state
  |-- main.rs         Entry point
     |
     | (bidirectional via Tauri events)
     |
Svelte Frontend (frontend/)
  |-- Terminal.svelte       xterm.js + WebGL + FitAddon
  |-- AISidebar.svelte      Streaming AI chat panel
  |-- CommandPalette.svelte Ctrl+K quick actions
  |-- App.svelte            Main layout (terminal + sidebar)
```

## Prerequisites

### System Dependencies (Linux/Ubuntu)

```bash
sudo apt-get install -y \
  build-essential \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev \
  pkg-config \
  libssl-dev
```

### macOS

Xcode Command Line Tools are required:

```bash
xcode-select --install
```

### Toolchain

- **Rust** 1.82+ (install via [rustup](https://rustup.rs))
- **Node.js** 20+ (install via [nvm](https://github.com/nvm-sh/nvm))
- **Tauri CLI** v2:

```bash
cargo install tauri-cli --version "^2"
```

## Setup

1. **Clone the repository:**

```bash
git clone git@github.com:YOUR_USERNAME/falcon.git
cd falcon
```

2. **Install frontend dependencies:**

```bash
cd frontend
npm install
cd ..
```

3. **Set your xAI API key** (optional but required for AI features):

```bash
export XAI_API_KEY="your-xai-api-key-here"
```

You can get an API key from [https://console.x.ai](https://console.x.ai). The terminal works without it, but AI features will be disabled.

4. **Run in development mode:**

```bash
cd src-tauri
cargo tauri dev
```

This starts the Vite dev server for the frontend and the Rust backend simultaneously with hot-reload.

5. **Build for production:**

```bash
cd src-tauri
cargo tauri build
```

The output binary will be in `src-tauri/target/release/`.

## Project Structure

```
.
|-- frontend/                 Svelte + TypeScript frontend
|   |-- src/
|   |   |-- App.svelte        Main layout
|   |   |-- main.ts           Entry point
|   |   |-- app.css           Global styles
|   |   |-- lib/
|   |       |-- Terminal.svelte       xterm.js PTY terminal
|   |       |-- AISidebar.svelte      AI chat sidebar
|   |       |-- CommandPalette.svelte Ctrl+K palette
|   |-- package.json
|   |-- vite.config.ts
|
|-- src-tauri/                Rust backend
|   |-- src/
|   |   |-- main.rs           Binary entry point
|   |   |-- lib.rs            Tauri commands + app state
|   |   |-- pty.rs            PTY manager
|   |   |-- ai.rs             xAI API client
|   |   |-- block.rs          Block manager
|   |   |-- context.rs        Session context collector
|   |-- Cargo.toml
|   |-- tauri.conf.json       Tauri configuration
|   |-- capabilities/         Permission definitions
|   |-- icons/                App icons
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+K` | Open command palette |
| `Ctrl+B` | Toggle AI sidebar |

## Configuration

### Environment Variables

| Variable | Description | Required |
|----------|-------------|----------|
| `XAI_API_KEY` | Your xAI API key for AI features | No (AI features disabled without it) |
| `SHELL` | Override default shell | No (defaults to `/bin/bash`) |

### Tauri Config

Edit `src-tauri/tauri.conf.json` to customize window size, title, and bundling options.

## Key Dependencies

### Rust (Backend)

- `tauri` 2.x -- app framework
- `portable-pty` 0.8 -- cross-platform PTY
- `vte` 0.13 -- terminal escape sequence parsing
- `tokio` 1.x -- async runtime
- `reqwest` 0.12 -- HTTP client for xAI API
- `serde` / `serde_json` -- serialization
- `uuid` -- block IDs
- `chrono` -- timestamps

### Frontend

- `@xterm/xterm` 6.x -- terminal emulator
- `@xterm/addon-fit` -- auto-resize
- `@xterm/addon-webgl` -- GPU-accelerated rendering
- `@tauri-apps/api` 2.x -- Tauri IPC
- `svelte` 5.x -- UI framework

## Development

### Running Tests

```bash
cd src-tauri
cargo test
```

### Checking Compilation

```bash
cd src-tauri
cargo check
```

### Building Frontend Only

```bash
cd frontend
npm run build
```

### Linting

```bash
cd frontend
npm run check
```

## Roadmap

- [ ] Theme system (Tokyo Night, Dracula, Solarized, etc.)
- [ ] Tab support (multiple PTY sessions)
- [ ] Split panes
- [ ] Command history search
- [ ] MCP/Goose agent integration
- [ ] Local LLM fallback (Ollama)
- [ ] Plugin system

## License

MIT
