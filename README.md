# OpenSauce

A modern, fast coding agent that lives in your terminal — built with **Rust**.
_Tracked by_ **Powered By Vexify.**

OpenSauce puts an LLM-driven coding loop in your terminal: streamed replies,
native tool calling (read/write files, list directories, run shell commands,
grep the workspace), persistent sessions, and a polished dual-mode TUI.

## Modes

OpenSauce has two interaction modes, each with its own theme:

| Mode    | Theme  | Behaviour                                                                 |
| ------- | ------ | ------------------------------------------------------------------------- |
| Build   | blue   | Execute: use tools to inspect and edit the workspace and get it done.      |
| Plan    | yellow | Reason first: read files and run read-only commands, then present a plan.  |

Switch modes anytime with `Tab`. Each mode injects its own system prompt so the
agent behaves accordingly.

## Features

- **Rust + ratatui/crossterm** TUI: streaming text, tool activity, themes.
- **Tool calling loop** with user-confirmable, safe fallbacks in Plan mode.
  - `read_file`, `write_file`, `list_files`, `workspace_info`
  - `shell` (guarded; non-mutating only in Plan mode), `grep`
- **Provider abstraction** with an **OpenAI-compatible** client and a
  deterministic **mock** provider for demos/tests/no-key offline use.
- **Session persistence**: conversations are stored to disk and listable.
- **Config** loaded from file + environment (`OPENSAUCE_API_KEY`, etc).

## Quick start

```bash
# Interactive TUI (defaults to Build mode)
cargo run -- start

# Start in Plan (yellow) mode
cargo run -- start --mode plan

# Headless: run a single prompt and print the reply
cargo run -- run "list the files in this repo"

# List saved sessions
cargo run -- sessions
```

Without an API key, OpenSauce falls back to the built-in mock provider so you
can try the full loop offline. Set `OPENSAUCE_API_KEY` (and optionally
`OPENSAUCE_BASE_URL`, `OPENSAUCE_MODEL`) to use a real OpenAI-compatible model.

## CLI

```
opensauce start|run|sessions
```

## Project layout

```
src/
  main.rs            CLI entry point
  ui/                TUI app, view rendering, themes
  core/agent.rs      the agent loop + tool dispatch
  core/tools/        file, shell, search, workspace tools
  core/message.rs    messages, roles, tool calls
  core/session.rs    conversations + persistence
  provider/          openai-compatible client + mock
  config.rs, mode.rs, theme.rs
```