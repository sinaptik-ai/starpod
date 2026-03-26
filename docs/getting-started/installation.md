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

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Or use an alternative provider:

```bash
# AWS Bedrock
export AWS_ACCESS_KEY_ID="AKIA..."
export AWS_SECRET_ACCESS_KEY="..."
export AWS_REGION="us-east-1"

# Google Vertex AI
export GOOGLE_CLOUD_PROJECT="my-project"
gcloud auth application-default login
```

Or add keys to your project `.env` file after [initialization](/getting-started/initialization):

```bash
# .env
ANTHROPIC_API_KEY=sk-ant-...
```

::: tip
API keys must be set via environment variables or `.env` files — they cannot be placed in config files. Any `api_key` found in a config file is ignored and triggers a warning.
:::

## Next Steps

Head to [Project Setup](/getting-started/initialization) to initialize Starpod in your project directory.
