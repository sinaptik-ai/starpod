# Installation

## Prerequisites

- **Rust 1.87+** — install via [rustup](https://rustup.rs/)
- **An Anthropic API key** — get one at [console.anthropic.com](https://console.anthropic.com/)

## Install from Source

```bash
git clone https://github.com/gabrieleventuri/starpod-rs.git
cd starpod-rs
cargo install --path crates/starpod --locked
```

This installs the `starpod` binary to your Cargo bin directory (usually `~/.cargo/bin/`).

## Verify

```bash
starpod --help
```

## Set Your API Key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Or add it to your project `.env` file after [initialization](/getting-started/initialization):

```bash
# .env
ANTHROPIC_API_KEY=sk-ant-...
```

::: tip
API keys must be set via environment variables or `.env` files — they cannot be placed in config files. Any `api_key` found in a config file is ignored and triggers a warning.
:::

## Next Steps

Head to [Project Setup](/getting-started/initialization) to initialize Starpod in your project directory.
