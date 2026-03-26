# Quick Start

Get Starpod running in under 2 minutes.

## 1. Initialize

```bash
cd your-project
starpod init --default
```

## 2. Set your API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Or use an alternative provider:

```bash
# AWS Bedrock
export AWS_ACCESS_KEY_ID="AKIA..."
export AWS_SECRET_ACCESS_KEY="..."
export AWS_REGION="us-east-1"

# Google Vertex AI (uses Application Default Credentials)
export GOOGLE_CLOUD_PROJECT="my-project"
gcloud auth application-default login
```

Set `provider = "bedrock"` or `provider = "vertex"` in your config — see [Configuration](/getting-started/configuration) for details.

## 3. Start the server

```bash
starpod serve
```

```
  Starpod is running

  Frontend http://127.0.0.1:3000
  API      http://127.0.0.1:3000/api
  WS       ws://127.0.0.1:3000/ws
  Telegram not configured
  Model    claude-haiku-4-5
  Project  /path/to/your-project
```

Open [http://localhost:3000](http://localhost:3000) for the web UI.

## Alternative: CLI

One-shot message:

```bash
starpod chat "What files are in this directory?"
```

Interactive REPL:

```bash
starpod repl
```

## What's Next?

- [Configuration](/getting-started/configuration) — customize the model, personality, and more
- [Memory](/concepts/memory) — learn how Starpod remembers across conversations
- [Skills](/concepts/skills) — teach your agent new abilities
- [Telegram](/integrations/telegram) — connect Starpod to Telegram
