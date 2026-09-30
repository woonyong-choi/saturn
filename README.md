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
> Design stage. There is no runnable code yet.

![Design: after you switch the chat from Claude Code to Codex, the new Codex session gets a packet from the Saturn record and both results stay in one chat](docs/assets/provider-switch.svg)

## How it works

The following is the designed behavior.

1. You run `saturn` in a repository and type a request. A background engine process stores the input in a local SQLite database before it sends anything to Codex or Claude Code.
2. While the agent works, you type a follow-up. A judge, a small model that answers yes-or-no, multiple-choice, and rating questions about your input, decides whether to add it to the running turn, start a separate task, or queue it.
3. When the context of a session passes a set token limit and no work is running, Saturn either lets the provider compact the session or, when that costs less, starts a new session. The new session gets a packet with the goal, recent turns, and open items taken from Saturn's own record.
4. You switch the chat from Claude Code to Codex. A chat is the conversation you see, and provider sessions open and close behind it. The new session receives only what changed since it last saw the chat.
5. You close the terminal. The engine keeps processing the inputs you already sent, and you can attach again later.

The full design is in the [design documents](docs/README.md), which are written in Korean.

## Status

Saturn is in the design stage. The design documents, decision records, and experiment plans are public, and there is no code yet. It targets macOS on Apple Silicon and needs Codex CLI or Claude Code. Commands, file formats, and behavior may change without notice before 1.0. Open design questions and planned experiments are tracked in [GitHub issues](https://github.com/woonyong-choi/saturn/issues), and comments there are welcome.

## Comparison

- Codex CLI or Claude Code on its own: each keeps its own sessions and compaction and needs no extra layer. Use one of them directly if you stay with one provider.
- Both tools in separate terminals: you can run them in parallel, but each keeps its own context and you carry context between them by hand. Saturn keeps one record and passes the needed context between providers.

## Roadmap

The order after the design stage is not fixed yet.

1. Design: design documents, decision records, and experiments on provider behavior. (in progress)
2. Core loop: accept inputs before sending, queue and hold inputs, keep one session per chat. (later)
3. Provider switching and context: switch between Codex and Claude Code in one chat, hand over context packets, track subagents and usage. (later)
4. Judge: judge every input with Jev, an external judgment API, as the first judge, and fall back to fixed per-question rules when it fails. (later)
5. Full-screen TUI and data management: status board, usage view, record cleanup, judgment export. (later)
6. Local judge model: train a personal judge model from recorded judgments and switch to it only when it is not worse than the current judge on the same evaluation set. (later)
7. Service: consent-based data collection, remote API, authentication, and infrastructure. (later)

## Documentation

The design documents are written in Korean.

- [Architecture](docs/architecture.md): code map and invariants
- [Input handling](docs/design/input-handling.md): accepting, queuing, holding, and resuming inputs
- [Provider connections and sessions](docs/design/providers-and-sessions.md): provider connections, session switching, subagent tracking
- [Context management](docs/design/context-management.md): measuring context and continuing in a new session
- [Decision records](docs/decisions/README.md): why each design choice was made
- [All documents](docs/README.md)
