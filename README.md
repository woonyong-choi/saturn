<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-dark.png">
    <img src="docs/assets/logo-light.png" alt="Saturn logo" width="160">
  </picture>
</p>

<h1 align="center">Saturn</h1>

<p align="center">
  A terminal tool that continues one conversation across Codex and Claude Code.
</p>

<p align="center">
  English | <a href="README.ko.md">한국어</a><br>
  <a href="#how-it-works">How it works</a> · <a href="#status">Status</a> · <a href="#roadmap">Roadmap</a> · <a href="#documentation">Documentation</a>
</p>

Developers who use Codex and Claude Code together lose context each time they switch, because each provider keeps its own sessions and compacts them in its own way. Saturn records every input locally before sending it and gives each provider session only the context it needs from that record. Unlike running each tool in a separate terminal, one chat keeps its history and task state across provider switches, parallel tasks, and fresh sessions.

> [!NOTE]
> In development. There is no runnable command yet.

![Design: after you switch the chat from Claude Code to Codex, the new Codex session gets a packet from the Saturn record and both results stay in one chat](docs/assets/provider-switch.svg)

## How it works

The following is the designed behavior.

1. You run `saturn` in a repository and type a request. A background engine process stores the input in a local SQLite database before it sends anything to Codex or Claude Code.
2. While the agent works, you type a follow-up. A judge, a small model that answers yes-or-no, multiple-choice, and rating questions about your input, decides whether to add it to the running turn, start a separate task, or queue it.
3. When the context of a session passes a set token limit and no work is running, Saturn either lets the provider compact the session or, when that costs less, starts a new session. The new session gets a packet with the goal, recent turns, and open items taken from Saturn's own record.
4. You switch the chat from Claude Code to Codex. A chat is the conversation you see, and provider sessions open and close behind it. The new session receives only what changed since it last saw the chat.
5. You close the terminal. The engine keeps processing the inputs you already sent, and you can attach again later.
6. A provider asks to run a command or edit a file. Saturn applies its own permission rules instead of the provider settings, and shows an approval prompt only when a rule says to ask.

The full design is in the [design documents](docs/README.md), which are written in Korean.

## Status

Saturn is in development. The message types, core rules, engine parts, and TUI are on `main`, but the input flow, the `saturn` command that starts a chat, and permission handling are not built yet. The design documents, decision records, and experiment reports are public. It targets macOS on Apple Silicon and needs Codex CLI or Claude Code. Commands, file formats, and behavior may change without notice before 1.0. Open design questions and planned experiments are tracked in [GitHub issues](https://github.com/woonyong-choi/saturn/issues), and comments there are welcome.

## Comparison

- Codex CLI or Claude Code on its own: each keeps its own sessions and compaction and needs no extra layer. Use one of them directly if you stay with one provider.
- Both tools in separate terminals: you can run them in parallel, but each keeps its own context and you carry context between them by hand. Saturn keeps one record and passes the needed context between providers.

## Roadmap

The order after the first conversation is not fixed yet.

1. First conversation: the `saturn` command, the input flow from acceptance to provider send, provider events with turn end, stop, and resume, approval replies, switching between Codex and Claude Code with context packets. (in progress)
2. Permissions: Saturn permission rules decide provider execution, and always-allow rules are stored. (next)
3. Chat management and recovery: continue without a TUI, crash recovery, chat names and grouping, added folders, task-completion notifications. (later)
4. Local judge model: score recorded judgments, train a personal judge model, and switch to it only when it is not worse than the current judge on the same evaluation set. (later)
5. Service: consent-based data collection, remote API, authentication, and infrastructure. (later)

## Documentation

The design documents are written in Korean.

- [Architecture](docs/architecture.md): code map and invariants
- [Input handling](docs/design/input-handling.md): accepting, queuing, holding, and resuming inputs
- [Provider connections and sessions](docs/design/providers-and-sessions.md): provider connections, session switching, subagent tracking
- [Context management](docs/design/context-management.md): measuring context and continuing in a new session
- [Decision records](docs/decisions/README.md): why each design choice was made
- [All documents](docs/README.md)

## Development

Run the following commands from the repository root. There is no CI yet.

```sh
cargo build --workspace
cargo test --workspace
```
