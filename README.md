# Sleepers Researcher

A local-first desktop AI research & dev agent. ReAct-style agent with tool use, persistent
memory (RAG), and a dual-backend router (Gemini + Groq) behind a clean black-and-white GUI.
Built with a Rust backend and a Tauri v2 frontend, packaged as a Windows `.exe`.

> Architected so a local model (Ollama-served Gemma) can be added later as a third backend.
> Only Gemini and Groq are implemented in this build.

## Stack
- **Backend:** Rust (Tauri v2 commands/events), `reqwest` + `serde_json` for LLM REST calls
- **Frontend:** plain HTML/CSS/JS (no framework) — fast to boot, design-system driven
- **Memory:** local embedded vector store (see `src-tauri/src/memory/`)

## Prerequisites
- Rust (stable, MSVC) + Windows 11 SDK + VS Build Tools 2022 (C++ workload)
- Node.js + npm
- Tauri CLI v2 (`cargo install tauri-cli --version "^2.0.0"`)

## Setup
1. Copy `.env.example` to `.env` and fill in `GEMINI_API_KEY`, `GROQ_API_KEY` (and optionally `TAVILY_API_KEY`).
2. From `src-tauri/`, run the dev app with `cargo tauri dev`.
3. For a double-clickable Windows release, double-click `build-release.cmd` from the project root.

## Build a double-clickable app
The app is already a Tauri desktop executable, not an HTTP server. The release helper builds the Rust/Tauri app and copies the final executable to a simple location:

```text
release/Sleepers Researcher.exe
```

Build options:

```powershell
# Easiest from File Explorer
build-release.cmd

# Or from a terminal
powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1
```

Tauri also writes the original release binary to:

```text
src-tauri/target/release/sleepers-researcher.exe
```

and the Windows installer to:

```text
src-tauri/target/release/bundle/nsis/
```

## API keys for the installed app
On launch, the app loads `.env` from these locations, in order of usefulness:

1. The current working directory, useful during development.
2. The folder beside `Sleepers Researcher.exe`, useful for portable builds.
3. `%APPDATA%\com.sleepers.researcher\.env`, useful after installation.

Example portable layout:

```text
release/
  Sleepers Researcher.exe
  .env
```

Minimum `.env`:

```env
GEMINI_API_KEY=your_gemini_key
GROQ_API_KEY=your_groq_key
TAVILY_API_KEY=optional_for_web_search
DEFAULT_BACKEND=gemini
```

## Ingest documents into memory
```
sleepers-researcher ingest ./notes
```

## Design system
Pitch-black background, white/off-white components, subtle gray panel surfaces,
system-ui sans-serif, high contrast, understated transitions.

## Permissions
Any state-mutating tool call (file write, code execution) requires explicit in-app
confirmation. Toggle `/yolo` to disable for a session, or `/autoapprove file_write`
to trust a specific tool type.
