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
  <a href="#installation">Installation</a> · <a href="#usage">Usage</a> · <a href="#status">Status</a> · <a href="#documentation">Documentation</a>
</p>

Developers who use Codex and Claude Code together lose context each time they switch, because each provider keeps its own sessions and compacts them in its own way. Saturn records every input locally before sending it and gives each provider session only the context it needs from that record. Unlike running each tool in a separate terminal, one chat keeps its history and task state across provider switches, parallel tasks, and fresh sessions.

> [!NOTE]
> In development. There are no releases; build from source.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/overview.en.dark.svg">
  <img src="docs/assets/overview.en.light.svg" alt="Design: every input reaches the Saturn record first, and a new provider session gets only a packet built from that record" width="100%">
</picture>

## How it works

Steps 1, 2, 4, 5, and 6 run on `main`. In step 3, opening a new session with a packet works, but the router does not yet choose what goes into the packet, and Saturn does not yet ask the provider to compact. Constraints you state are recorded and carried in the packet up to a size limit, while releasing them is designed only (see Status).

1. You run `saturn` in a repository and type a request. A background engine process stores the input in a local SQLite database before it sends anything to Codex or Claude Code.
2. While the agent works, you type a follow-up. A router, a small model that answers yes-or-no, multiple-choice, and rating questions about your input, decides whether to add it to the running turn, start a separate task, or queue it.
3. When the context of a session passes a set token limit and no work is running, Saturn either lets the provider compact the session or, when that costs less, starts a new session. The new session gets a packet with the goal, recent turns, and open items taken from Saturn's own record.
4. You switch the chat from Claude Code to Codex. A chat is the conversation you see, and provider sessions open and close behind it. The new session receives only what changed since it last saw the chat.
5. You close the terminal. The engine keeps processing the inputs you already sent, and you can attach again later.
6. A provider asks to run a command or edit a file. Saturn applies its own permission rules instead of the provider settings, and shows an approval prompt only when a rule says to ask.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/provider-switch.en.dark.svg">
  <img src="docs/assets/provider-switch.en.light.svg" alt="After you switch the chat from Claude Code to Codex, the new Codex session gets a packet from the Saturn record and both results stay in one chat" width="100%">
</picture>

Switching providers: the new session gets a packet from the record. [Provider connections and sessions](docs/design/providers-and-sessions.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/permission-decision.en.dark.svg">
  <img src="docs/assets/permission-decision.en.light.svg" alt="A permission request is answered by Saturn rules as allow or deny, and only an ask result reaches the TUI" width="100%">
</picture>

Permissions: a rule answers, and only ask reaches you. [Permissions](docs/design/permissions.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/constraint-lifecycle.en.dark.svg">
  <img src="docs/assets/constraint-lifecycle.en.light.svg" alt="Design: a constraint is registered automatically, after asking, or not at all" width="100%">
</picture>

Constraints: what you say to keep following. [Constraints](docs/design/constraints.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/extension-store.en.dark.svg">
  <img src="docs/assets/extension-store.en.light.svg" alt="Design: an extension is copied into the Saturn store, judged per provider, and injected in each provider format" width="100%">
</picture>

Extensions: one store, injected per provider. [Capability list and extensions](docs/design/extensions.md)

The full design is in the [design documents](docs/README.md), which are written in Korean.

## Installation

Saturn runs on macOS on Apple Silicon. It needs Rust 1.95.0, the version CI builds with (checked 2026-10-04). It also needs Codex CLI (`codex`), Claude Code (`claude`), or both, already signed in. Saturn does not sign in for you, so sign in with each CLI first. Check the sign-in with `codex login status` and `claude auth status`.

`saturn` starts `saturn-engine`, so install both executables together.

```sh
cargo install --locked --git https://github.com/woonyong-choi/saturn saturn-cli saturn-engine
```

Cargo puts both executables in its bin folder, `~/.cargo/bin` by default. Make sure that folder is on `PATH`. `saturn` looks for `saturn-engine` in its own folder first and then on `PATH`.

To build from a clone instead, run `cargo build --release` in the repository root. This creates `target/release/saturn` and `target/release/saturn-engine`. Keep them in the same folder.

## Usage

> [!NOTE]
> The steps below were checked with real Codex and Claude Code, including sending requests and switching with `/model` (see Status).

### Start saturn

Run `saturn` in a repository. It starts the engine in the background and opens the full-screen chat. The first start creates `~/.saturn`.

```sh
saturn
```

If Saturn has no router key, a Router key window asks for it with hidden input. `Enter` confirms and `Esc` quits `saturn`. The engine keeps running after `saturn` exits.

### Provide the router key

The router needs an API key. Saturn gets it by trying these in order when the engine starts, and stops at the first one that works.

1. The `SATURN_KEY` environment variable of the shell that starts the engine. When it is set, Saturn reads it before the saved key.
2. The saved key. On macOS this is the keychain entry with the account `saturn-key`, which Saturn creates when you enter a key in the Router key window. On other systems it is the file `router.key` in the Saturn home with mode 0600.
3. The command in the `router.key.command` setting. Set it in `~/.saturn/config.toml` as `command = ["op", "read", "<item>"]` under `[router.key]`. Saturn runs the command without a shell and reads the first line of its output.

If `SATURN_KEY` is set but the router rejects it, Saturn does not fall back to the saved key and goes on to the command, then the Router key window. Saturn never takes the key from a command-line argument or standard input. A running engine does not read `SATURN_KEY` again.

```sh
export SATURN_KEY=<your key>
saturn
```

### Run without a screen

Without a screen, as in a pipe or CI, Saturn cannot ask for the key. It prints how to set the key and exits with code 77.

```sh
saturn usage
```

```text
Error: Router key required (router key required: router rejected the key): set the SATURN_KEY environment variable or the router.key.command setting, then run again
```

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Other failure, including answering no to a confirmation and a failed task in plain mode |
| 2 | Usage error: change the call, for example no terminal for a picker or a nested run inside an agent without a pass |
| 66 | Nothing to open: no chat to continue, missing folder, unknown router version |
| 69 | Engine unavailable: failed to start, no answer, failed to replace, connection lost |
| 70 | Engine internal error |
| 75 | Not now, try later |
| 77 | Router key missing or rejected |
| 78 | Configuration error |
| 130 | Quit from a window with `Esc` or `Ctrl+C` |

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/exit-codes.en.dark.svg">
  <img src="docs/assets/exit-codes.en.light.svg" alt="Exit codes grouped by what to do: change the call, retry, fix the settings, other failures, and success or abort" width="100%">
</picture>

### Use the plain screen

`saturn --plain` draws the chat as lines without boxes or motion and lists choices by number. Setting `NO_COLOR` or the `tui.screen` setting does the same, and `/plain` switches during a chat. `--plain=false` overrides both. Without a terminal, `saturn` uses the plain screen on its own.

```sh
saturn --plain
```

### Switch the model

Type `/model` in the input box to choose the model for the next inputs. `/model codex` or `/model claude` lists only that provider. The list shows the installed providers.

```text
/model codex
```

`Enter` accepts the command completion, and a second `Enter` runs it. Use the arrow keys to move, `Enter` to choose, and `Esc` to cancel. The chosen model applies to every later input in the chat until you choose again.

### Continue a chat

Run `saturn --continue` in the same folder to open its most recent chat. `saturn --resume` lists the chats of the folder to pick from.

```sh
saturn --continue
```

## Status

Saturn is in development. The message types, core rules, engine, TUI, and the `saturn` command are on `main`. These work in tests with fake providers: the input flow from acceptance to provider send, stop and resume, Saturn permission rules, `/model` and `/usage`, a plain screen mode for pipes and `NO_COLOR`, keeping work running after the TUI closes, recovery after an engine crash, installing extensions into the Saturn store and injecting them when a connection starts, and child connections that let an agent inside Saturn, or an outside Claude Code or Codex, ask the running engine for work. The procedure in `scripts/e2e/README.md` (steps a to k) passed against real Codex, Claude Code, and the router on 2026-10-04. The rest was not run against real providers. The packet carries stored constraints that have not been released, up to a size limit; this is tested with fake providers only and not yet against real providers ([#296](https://github.com/woonyong-choi/saturn/issues/296) condition 2). Designed but not built: ordering the packet by router answers, handoff compaction ([#380](https://github.com/woonyong-choi/saturn/issues/380)), releasing constraints and the `/constraints` screen ([#379](https://github.com/woonyong-choi/saturn/issues/379), [#381](https://github.com/woonyong-choi/saturn/issues/381)), scoring and training a local router model, and sharing data with a server. The design documents, decision records, and experiment reports are public. It targets macOS on Apple Silicon and needs Codex CLI or Claude Code. Commands, file formats, and behavior may change without notice before 1.0. Open design questions and planned experiments are tracked in [GitHub issues](https://github.com/woonyong-choi/saturn/issues), and comments there are welcome.

## Comparison

- Codex CLI or Claude Code on its own: each keeps its own sessions and compaction and needs no extra layer. Use one of them directly if you stay with one provider.
- Both tools in separate terminals: you can run them in parallel, but each keeps its own context and you carry context between them by hand. Saturn keeps one record and passes the needed context between providers.

## Roadmap

The order after the first conversation is not fixed yet.

1. First conversation: a full run against real Codex and Claude Code, including a switch back after a user constraint. (in progress)
2. Constraints and packets: release and exceptions for constraints, packets ordered by router answers. (next)
3. Chat management: chat names and grouping, task-completion notifications. (next)
4. Local router model: score recorded judgments, train a personal router model, and switch to it only when it is not worse than the current router on the same evaluation set. (later)
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

Run the following commands from the repository root. CI runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, and a check that `saturn-protocol/generated` is up to date on every pull request.

```sh
cargo build --workspace
cargo test --workspace
```

## License

[MIT](LICENSE)
