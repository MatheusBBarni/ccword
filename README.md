<div align="center">

# ccword

**One local word completion inside Claude Code**

[Features](#features) • [Install](#install) • [Usage](#usage) • [Commands](#commands) • [Roadmap](#roadmap) • [How-it-works](#how-it-works)

</div>

`ccword` is a macOS launcher for the Claude Code CLI. It runs Claude Code through a pseudo-terminal and displays one dim word suffix while you type. Your prompt changes only when you accept the hint.

> [!IMPORTANT]
> `ccword` is an early macOS-only project. Install it as `ccword`, not as `claude`.

## Features

- **Two local providers:** Apple `NSSpellChecker` and a learned n-gram store.
- **Private by design:** no network requests, downloaded models, or transcript scanning.
- **Fail-open wrapper:** unrecognized Claude Code layouts pass input through without hints.
- **Native terminal behavior:** forwards Claude arguments, environment, working directory, resize events, and signals.
- **Explicit learning controls:** learning is off by default and stores token counts rather than full prompts.

## Requirements

- macOS
- Rust 1.85 or newer
- [Claude Code](https://docs.claude.com/en/docs/claude-code/overview) installed and available on `PATH`

## Install

Until the [Cargo publishing work](https://github.com/MatheusBBarni/ccword/issues/1) is complete, build and install from source:

```sh
git clone https://github.com/MatheusBBarni/ccword.git
cd ccword
cargo install --path .
```

For local development:

```sh
cargo build --release
./target/release/ccword ctl doctor
```

`ccword` finds the installed `claude` executable and skips its own path. Set `CCWORD_CLAUDE=/path/to/claude` to override discovery.

## Usage

Choose a provider, then launch Claude Code through `ccword`:

```sh
# Apple system completion
ccword ctl config set mode apple
ccword

# Or learned local n-grams
ccword ctl config set mode ngram
ccword ctl config set learning on
ccword
```

Claude Code arguments are forwarded unchanged:

```sh
ccword --resume
ccword --continue
```

### Accepting a hint

| Key | Action |
| --- | --- |
| <kbd>→</kbd> | Accept the suffix and append a space |
| <kbd>Tab</kbd> | Accept the suffix without a space |
| <kbd>Enter</kbd> | Submit only text you typed or accepted |
| <kbd>Esc</kbd>, <kbd>Backspace</kbd>, cursor movement | Dismiss the hint |

> [!NOTE]
> Hints are disabled by default. Slash commands, `@` mentions, paths, shell mode (`!`), an open Claude completion menu, and unknown composer layouts receive no hint.

## Commands

### Configuration

```sh
ccword ctl config show
ccword ctl config set mode apple       # off | apple | ngram
ccword ctl config set language en      # use "system" for the macOS default
ccword ctl config set learning on
ccword ctl config set min-confidence 0.15
ccword ctl config set min-support 2
ccword ctl config set half-life-days 30
ccword ctl config set right-arrow-appends-space on
ccword ctl config set debug off
```

Configuration lives at `~/Library/Application Support/ccword/config.toml`. Changes are picked up during a running session after the next keystroke.

### Diagnostics and completion checks

```sh
ccword ctl doctor
ccword ctl version
ccword ctl complete --mode apple he
ccword ctl complete --mode ngram "please he"
```

`complete` prints only the suggested suffix, or nothing when no candidate qualifies.

### Learning

```sh
ccword ctl learn status
ccword ctl learn pause
ccword ctl learn resume
ccword ctl learn path
ccword ctl learn clear
```

The wrapper can learn prompts submitted through it. The optional hook also captures prompts submitted from Claude Code sessions that were not launched through `ccword`:

```sh
ccword ctl hook status
ccword ctl hook install
ccword ctl hook install --write-settings
ccword ctl hook uninstall
```

`hook install` prints the plugin directory. `--write-settings` also adds the hook to `~/.claude/settings.json`.

### History import

`ccword` imports only files you explicitly provide. Plain text uses one prompt per line; JSONL records may contain a `prompt` or `text` field.

```sh
ccword ctl learn import --preview prompts.txt
ccword ctl learn import --apply prompts.txt
ccword ctl learn undo
```

`--preview` reports counts without changing the database. `--apply` first creates a restorable backup; `undo` restores it.

## Roadmap

- [Publish `ccword` to crates.io](https://github.com/MatheusBBarni/ccword/issues/1)
- [Add an interactive configuration TUI](https://github.com/MatheusBBarni/ccword/issues/2)

## How it works

`ccword` launches Claude Code with [`portable-pty`](https://crates.io/crates/portable-pty), tracks its terminal screen, and overlays a dim suffix only when the prompt composer is confidently recognized. Apple mode calls `NSSpellChecker` through [`objc2-app-kit`](https://crates.io/crates/objc2-app-kit). N-gram mode ranks locally learned one-, two-, and three-word observations, preferring the longest matching context with enough support and confidence.

The n-gram database lives at `~/Library/Application Support/ccword/ngrams.sqlite`. It stores normalized token counts, display forms, and timestamps, not complete prompts. Counts can still reveal sensitive phrases, so protect or clear the database when needed.

## Safety and fallback behavior

- Non-terminal stdin or stdout execs Claude Code directly without wrapping it.
- `CCWORD_DISABLE=1` bypasses the wrapper.
- Unsupported Claude Code layouts pass every key through unchanged.
- `Ctrl-C` is forwarded to the child; `Ctrl-Z` suspends and resumes both processes.
- `ccword` does not patch Claude Code, alter permission mode, or send prompt text over the network.

> [!WARNING]
> Distribution signing and notarization are not configured. Those require an Apple Developer account and a separate release process.
