# Installation

## Prerequisites

- **Rust 1.87+** — install via [rustup](https://rustup.rs/)
- **An LLM provider** — Anthropic API key ([console.anthropic.com](https://console.anthropic.com/)), AWS Bedrock credentials, Google Vertex AI project, or any other [supported provider](/getting-started/configuration#provider-options)

## Install from crates.io

```bash
cargo install starpod
```

## Install from Source

```bash
git clone https://github.com/sinaptik-ai/starpod.git
cd starpod
cargo install --path crates/starpod --locked
```

Both methods install the `starpod` binary to your Cargo bin directory (usually `~/.cargo/bin/`).

## Verify

```bash
starpod --help
```

## Set Your API Key

Seed your API key into the vault during initialization:

```bash
starpod init --env ANTHROPIC_API_KEY="sk-ant-..."
```

Or manage it later via the web UI Settings page after running `starpod dev`.

::: tip
API keys are stored in the encrypted vault — never in config files or `.env` files. The vault injects them into the process environment at startup.
:::

## Next Steps

Head to [Project Setup](/getting-started/initialization) to initialize Starpod in your project directory.
