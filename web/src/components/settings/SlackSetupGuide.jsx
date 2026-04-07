import { useState } from "react";
import { apiHeaders } from "../../lib/api";

/**
 * Guided 4-step Slack Socket Mode setup, rendered inside the connector
 * detail panel when the selected template has `socket_mode: true`.
 *
 * Why this exists
 * ===============
 * Slack Socket Mode bots cannot be distributed via OAuth — every user has
 * to create their own custom Slack app and generate an app-level token
 * (`xapp-…`) plus a bot user token (`xoxb-…`). The official Slack admin
 * UI is intimidating, so this component:
 *
 *  1. Hands the user a one-click "Create app from manifest" link with a
 *     pre-filled YAML manifest (copy button included) so they don't have
 *     to know which scopes/events to request.
 *  2. Walks them through generating both tokens with the exact menu paths.
 *  3. Saves both tokens to the vault.
 *  4. Validates them with `auth.test` against the live Slack API and
 *     shows the workspace + bot identity inline before the bot starts.
 *  5. Calls `/connectors/slack/reload` to flip
 *     `[channels.slack].enabled` and (re)start the bot in-process.
 *
 * Props
 * -----
 *  - `vaultKeys` — `Set<string>` of keys already in the vault. Used to
 *    decide whether the inputs should start collapsed (showing "✓ set").
 *  - `onSaved` — callback fired after a successful test+reload so the
 *    parent can refresh the connectors list and toast the user.
 *  - `onError(text)` — callback for surfacing errors via the parent's
 *    status toast (so failures share visual treatment with the rest of
 *    the connectors UI).
 */
export default function SlackSetupGuide({ vaultKeys, onSaved, onError }) {
  const appTokenSet = vaultKeys.has("SLACK_APP_TOKEN");
  const botTokenSet = vaultKeys.has("SLACK_BOT_TOKEN");

  const [appToken, setAppToken] = useState("");
  const [botToken, setBotToken] = useState("");
  const [savingApp, setSavingApp] = useState(false);
  const [savingBot, setSavingBot] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState(null); // {team, team_id, bot_user_id} | null
  const [manifestCopied, setManifestCopied] = useState(false);

  // The pre-filled Slack app manifest. Pasted by the user into Slack's
  // "Create app → From a manifest" wizard. Display name is intentionally
  // generic so this works for any Starpod instance; the user can edit
  // before clicking Create.
  //
  // NOTE: Slack's manifest dialog accepts both JSON and YAML, but the
  // default mode is JSON and switching tabs is non-obvious. We ship JSON
  // so the paste-and-click flow works without any format toggling.
  const manifestJson = `{
  "display_information": {
    "name": "Starpod",
    "description": "Personal AI assistant"
  },
  "features": {
    "bot_user": {
      "display_name": "Starpod",
      "always_online": true
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "bookmarks:read",
        "channels:history",
        "channels:join",
        "channels:read",
        "chat:write",
        "chat:write.public",
        "emoji:read",
        "files:read",
        "files:write",
        "groups:history",
        "groups:read",
        "im:history",
        "im:read",
        "im:write",
        "links:read",
        "links:write",
        "mpim:history",
        "mpim:read",
        "mpim:write",
        "pins:read",
        "reactions:read",
        "reactions:write",
        "team:read",
        "users:read",
        "users:read.email",
        "users.profile:read"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": [
        "app_mention",
        "message.im"
      ]
    },
    "interactivity": {
      "is_enabled": true
    },
    "socket_mode_enabled": true,
    "org_deploy_enabled": false,
    "token_rotation_enabled": false
  }
}`;

  const copyManifest = async () => {
    try {
      await navigator.clipboard.writeText(manifestJson);
      setManifestCopied(true);
      setTimeout(() => setManifestCopied(false), 2000);
    } catch (e) {
      onError?.(`Could not copy manifest: ${e.message}`);
    }
  };

  const saveSecret = async (key, value, setter) => {
    setter(true);
    try {
      const resp = await fetch(
        `/api/settings/vault/${encodeURIComponent(key)}`,
        {
          method: "PUT",
          headers: apiHeaders(),
          body: JSON.stringify({ value: value.trim() }),
        },
      );
      if (!resp.ok) {
        onError?.(`Failed to save ${key}`);
        setter(false);
        return false;
      }
      setter(false);
      return true;
    } catch (e) {
      onError?.(e.message);
      setter(false);
      return false;
    }
  };

  const handleSaveAppToken = async () => {
    if (!appToken.trim()) return;
    if (await saveSecret("SLACK_APP_TOKEN", appToken, setSavingApp)) {
      setAppToken("");
      onSaved?.({ silent: true }); // refresh vault keys, no toast
    }
  };

  const handleSaveBotToken = async () => {
    if (!botToken.trim()) return;
    if (await saveSecret("SLACK_BOT_TOKEN", botToken, setSavingBot)) {
      setBotToken("");
      onSaved?.({ silent: true });
    }
  };

  // Test + reload happen in a single action: validate via auth.test, and
  // only on success do we enable [channels.slack] and start the bot.
  // This means an unhealthy token never causes the bot to flap restart.
  const handleTestAndConnect = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      const testResp = await fetch("/api/settings/connectors/slack/test", {
        method: "POST",
        headers: apiHeaders(),
      });
      const testData = await testResp.json().catch(() => ({}));
      if (!testResp.ok) {
        onError?.(testData.error || "Slack auth.test failed");
        setTesting(false);
        return;
      }
      setTestResult(testData);

      const reloadResp = await fetch("/api/settings/connectors/slack/reload", {
        method: "POST",
        headers: apiHeaders(),
      });
      if (!reloadResp.ok) {
        const err = await reloadResp.json().catch(() => ({}));
        onError?.(err.error || "Failed to start Slack bot");
        setTesting(false);
        return;
      }

      // Bubble up to the parent so the connectors list refreshes and the
      // tile turns green.
      onSaved?.({
        silent: false,
        text: `Connected to ${testData.team}`,
      });
    } catch (e) {
      onError?.(e.message);
    }
    setTesting(false);
  };

  // ── Render ────────────────────────────────────────────────────────────

  // If both tokens are already set AND we have a fresh test result, the
  // setup guide collapses to a success summary. The user can still
  // re-test or reset.
  if (testResult) {
    return (
      <div
        style={{
          padding: "12px",
          border: "1px solid var(--color-ok)",
          background: "rgba(34,197,94,0.05)",
          display: "flex",
          flexDirection: "column",
          gap: "6px",
        }}
      >
        <div
          style={{
            fontSize: "12px",
            color: "var(--color-ok)",
            fontWeight: 600,
          }}
        >
          ✓ Connected
        </div>
        <div style={{ fontSize: "11px", color: "var(--color-secondary)" }}>
          Workspace:{" "}
          <span style={{ fontFamily: "var(--font-mono)" }}>
            {testResult.team}
          </span>{" "}
          ({testResult.team_id})
        </div>
        <div style={{ fontSize: "11px", color: "var(--color-secondary)" }}>
          Bot user:{" "}
          <span style={{ fontFamily: "var(--font-mono)" }}>
            {testResult.bot_user_id}
          </span>
        </div>
        <div style={{ fontSize: "11px", color: "var(--color-dim)", marginTop: "4px" }}>
          @-mention the bot in any channel it's invited to, or DM it
          directly. The bot reconnects automatically if Slack drops the
          WebSocket.
        </div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
      <div style={{ fontSize: "11px", color: "var(--color-dim)", lineHeight: 1.5 }}>
        Slack uses Socket Mode — your bot connects outbound, so it works
        on a laptop, a VM, or behind a firewall with no public URL. You'll
        create a custom Slack app once (~3 minutes), then paste two tokens
        below.
      </div>

      {/* Step 1 — Create the Slack app */}
      <Step n={1} title="Create your Slack app">
        <div style={{ fontSize: "11px", color: "var(--color-secondary)", lineHeight: 1.5 }}>
          We've pre-filled a manifest with the right scopes and event
          subscriptions. Copy it, then open Slack and paste it when asked.
        </div>
        <div
          style={{
            position: "relative",
            border: "1px solid var(--color-border-main)",
            background: "var(--color-bg)",
          }}
        >
          <pre
            style={{
              margin: 0,
              padding: "10px 12px",
              fontSize: "10px",
              fontFamily: "var(--font-mono)",
              color: "var(--color-secondary)",
              maxHeight: "180px",
              overflow: "auto",
              whiteSpace: "pre",
            }}
          >
            {manifestJson}
          </pre>
          <button
            type="button"
            onClick={copyManifest}
            style={{
              position: "absolute",
              top: "6px",
              right: "6px",
              padding: "2px 8px",
              fontSize: "10px",
              fontFamily: "var(--font-mono)",
              background: "var(--color-elevated)",
              color: "var(--color-primary)",
              border: "1px solid var(--color-border-main)",
              cursor: "pointer",
            }}
          >
            {manifestCopied ? "✓ copied" : "copy"}
          </button>
        </div>
        <a
          href="https://api.slack.com/apps?new_app=1"
          target="_blank"
          rel="noreferrer"
          style={{
            fontSize: "11px",
            color: "var(--color-accent)",
            textDecoration: "none",
            display: "inline-flex",
            alignItems: "center",
            gap: "4px",
            marginTop: "4px",
          }}
        >
          Open Slack → Create app from manifest ↗
        </a>
        <div style={{ fontSize: "10px", color: "var(--color-dim)" }}>
          Pick your workspace, choose <em>From a manifest</em>, leave the
          format on <em>JSON</em>, paste, and click <em>Next → Create</em>.
        </div>
      </Step>

      {/* Step 2 — Install to workspace */}
      <Step n={2} title="Install the app to your workspace">
        <div style={{ fontSize: "11px", color: "var(--color-secondary)", lineHeight: 1.5 }}>
          In the left sidebar, open{" "}
          <strong style={{ color: "var(--color-primary)" }}>
            Install App
          </strong>{" "}
          (under <em>Settings</em>), click{" "}
          <strong style={{ color: "var(--color-primary)" }}>
            Install to {"{your workspace}"}
          </strong>
          , and approve the permissions. Without this step the bot token
          in Step 4 will not exist yet.
        </div>
      </Step>

      {/* Step 3 — App-level token */}
      <Step n={3} title="Generate the app-level token (xapp-…)">
        <div style={{ fontSize: "11px", color: "var(--color-secondary)", lineHeight: 1.5 }}>
          Still on <em>Basic Information</em>, scroll to{" "}
          <strong style={{ color: "var(--color-primary)" }}>
            App-Level Tokens
          </strong>{" "}
          → <em>Generate Token and Scopes</em>. Add the{" "}
          <code style={{ fontFamily: "var(--font-mono)" }}>connections:write</code>{" "}
          scope, click Generate, then copy the token.
        </div>
        <SecretInput
          label="SLACK_APP_TOKEN"
          placeholder="xapp-..."
          value={appToken}
          onChange={setAppToken}
          isSet={appTokenSet}
          saving={savingApp}
          onSave={handleSaveAppToken}
        />
      </Step>

      {/* Step 4 — Bot token */}
      <Step n={4} title="Copy the bot user OAuth token (xoxb-…)">
        <div style={{ fontSize: "11px", color: "var(--color-secondary)", lineHeight: 1.5 }}>
          Open the <strong style={{ color: "var(--color-primary)" }}>OAuth & Permissions</strong>{" "}
          page in the sidebar and copy the{" "}
          <em>Bot User OAuth Token</em> at the top — it starts with{" "}
          <code style={{ fontFamily: "var(--font-mono)" }}>xoxb-</code>.
        </div>
        <div style={{ fontSize: "10px", color: "var(--color-warn)", lineHeight: 1.5 }}>
          Don't see the token? You skipped Step 2. Go back to{" "}
          <strong>Install App</strong> in the sidebar and install the app
          to your workspace first — the token is generated on install.
        </div>
        <SecretInput
          label="SLACK_BOT_TOKEN"
          placeholder="xoxb-..."
          value={botToken}
          onChange={setBotToken}
          isSet={botTokenSet}
          saving={savingBot}
          onSave={handleSaveBotToken}
        />
      </Step>

      {/* Final action */}
      <div
        style={{
          borderTop: "1px solid var(--color-border-subtle)",
          paddingTop: "12px",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: "8px",
        }}
      >
        <div style={{ fontSize: "11px", color: "var(--color-dim)" }}>
          We'll call Slack's <code style={{ fontFamily: "var(--font-mono)" }}>auth.test</code>{" "}
          to verify the tokens before starting the bot.
        </div>
        <button
          className="s-save-btn"
          style={{ padding: "6px 16px", fontSize: "12px" }}
          disabled={!appTokenSet || !botTokenSet || testing}
          onClick={handleTestAndConnect}
          title={
            !appTokenSet || !botTokenSet
              ? "Save both tokens first"
              : "Verify and start the Slack bot"
          }
        >
          {testing ? "Testing..." : "Test & Connect"}
        </button>
      </div>
    </div>
  );
}

// ── Sub-components ──────────────────────────────────────────────────────

function Step({ n, title, children }) {
  return (
    <div
      style={{
        display: "flex",
        gap: "10px",
        alignItems: "flex-start",
      }}
    >
      <span
        style={{
          flexShrink: 0,
          width: "20px",
          height: "20px",
          borderRadius: "50%",
          background: "var(--color-elevated)",
          color: "var(--color-accent)",
          fontSize: "11px",
          fontWeight: 600,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          marginTop: "1px",
        }}
      >
        {n}
      </span>
      <div
        style={{
          flex: 1,
          display: "flex",
          flexDirection: "column",
          gap: "8px",
          minWidth: 0,
        }}
      >
        <div
          style={{
            fontSize: "12px",
            fontWeight: 600,
            color: "var(--color-primary)",
          }}
        >
          {title}
        </div>
        {children}
      </div>
    </div>
  );
}

function SecretInput({ label, placeholder, value, onChange, isSet, saving, onSave }) {
  if (isSet) {
    return (
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "8px",
          fontSize: "11px",
        }}
      >
        <span
          style={{
            fontFamily: "var(--font-mono)",
            color: "var(--color-secondary)",
          }}
        >
          {label}
        </span>
        <span style={{ color: "var(--color-ok)" }}>✓ saved</span>
      </div>
    );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
      <span
        style={{
          fontSize: "10px",
          fontFamily: "var(--font-mono)",
          color: "var(--color-dim)",
        }}
      >
        {label}
      </span>
      <div style={{ display: "flex", gap: "6px" }}>
        <input
          className="s-input"
          type="password"
          placeholder={placeholder}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          style={{
            flex: 1,
            fontFamily: "var(--font-mono)",
            fontSize: "12px",
          }}
        />
        <button
          className="s-save-btn"
          style={{
            padding: "4px 12px",
            fontSize: "11px",
            flexShrink: 0,
          }}
          disabled={!value.trim() || saving}
          onClick={onSave}
        >
          {saving ? "..." : "Save"}
        </button>
      </div>
    </div>
  );
}
