import { useState, useEffect, useCallback, useMemo } from "react";
import { apiHeaders } from "../../lib/api";
import { Loading } from "../ui/EmptyState";
import SlackSetupGuide from "./SlackSetupGuide";
import { LOGOS, FALLBACK_LOGO } from "./connectorLogos";

// ── Main component ────────────────────────────────────────────────────────

export default function ConnectorsTab() {
  const [connectors, setConnectors] = useState(null);
  const [templates, setTemplates] = useState([]);
  const [vaultKeys, setVaultKeys] = useState(new Set());
  const [status, setStatus] = useState(null);
  const [saving, setSaving] = useState(null);
  const [selected, setSelected] = useState(null);
  const [secretValues, setSecretValues] = useState({});
  const [editConfig, setEditConfig] = useState({});
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [namingType, setNamingType] = useState(null);
  const [instanceName, setInstanceName] = useState("");
  const [search, setSearch] = useState("");
  const [oauthProxyUrl, setOauthProxyUrl] = useState(null);
  const [proxyProviders, setProxyProviders] = useState({});
  const [oauthPolling, setOauthPolling] = useState(null); // { sessionId, provider }

  const load = useCallback(() => {
    Promise.all([
      fetch("/api/settings/connectors", { headers: apiHeaders() }).then((r) =>
        r.json(),
      ),
      fetch("/api/settings/connector-templates", {
        headers: apiHeaders(),
      }).then((r) => r.json()),
      fetch("/api/settings/vault", { headers: apiHeaders() }).then((r) =>
        r.json(),
      ),
    ])
      .then(([c, t, v]) => {
        setConnectors(Array.isArray(c) ? c : c.connectors || []);
        setTemplates(Array.isArray(t) ? t : t.templates || []);
        const entries = v.entries || (Array.isArray(v) ? v : []);
        setVaultKeys(new Set(entries.map((e) => e.key)));
      })
      .catch(() =>
        setStatus({ type: "error", text: "Failed to load connectors" }),
      );
  }, []);

  useEffect(load, [load]);

  // Load OAuth proxy config and available providers
  useEffect(() => {
    fetch("/api/config", { headers: apiHeaders() })
      .then((r) => r.json())
      .then((cfg) => {
        if (cfg.oauth_proxy_url) {
          setOauthProxyUrl(cfg.oauth_proxy_url);
          fetch(`${cfg.oauth_proxy_url}/api/v1/oauth/providers`)
            .then((r) => r.json())
            .then((data) => {
              // Convert array [{name, scopes}] to map {name: {scopes}}
              const map = {};
              const arr = Array.isArray(data) ? data : data.providers || [];
              arr.forEach((p) => {
                map[p.name] = { scopes: p.scopes };
              });
              setProxyProviders(map);
            })
            .catch(() => {});
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const handler = (e) => {
      if (e.data?.type === "oauth-complete") {
        load();
        setStatus({ type: "ok", text: `Connected via OAuth` });
      }
    };
    window.addEventListener("message", handler);
    return () => window.removeEventListener("message", handler);
  }, [load]);

  const filteredTemplates = useMemo(() => {
    if (!search.trim()) return templates;
    const q = search.toLowerCase();
    return templates.filter(
      (t) =>
        t.name.toLowerCase().includes(q) ||
        t.display_name.toLowerCase().includes(q) ||
        (t.description || "").toLowerCase().includes(q),
    );
  }, [templates, search]);

  if (!connectors) return <Loading />;

  const handleAdd = async (templateName, name) => {
    setSaving("add");
    setStatus(null);
    try {
      const body = { type: templateName };
      if (name) body.name = name;
      const resp = await fetch("/api/settings/connectors", {
        method: "POST",
        headers: apiHeaders(),
        body: JSON.stringify(body),
      });
      if (resp.ok) {
        const created = await resp.json();
        setStatus({ type: "ok", text: `Added ${created.display_name}` });
        load();
        openDetail(created);
      } else {
        const err = await resp.json().catch(() => ({}));
        setStatus({ type: "error", text: err.error || "Failed to add" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleTileClick = (tpl) => {
    // For multi-instance, find all connectors of this type
    const conns = connectors.filter((c) => c.type === tpl.name);
    if (conns.length === 1) {
      openDetail(conns[0]);
    } else if (conns.length > 1) {
      // If multiple instances, open the first one (user can browse via grid for named ones)
      openDetail(conns[0]);
    } else if (tpl.multi_instance) {
      setNamingType(tpl.name);
      setInstanceName("");
    } else {
      handleAdd(tpl.name);
    }
  };

  const handleNameSubmit = () => {
    if (!instanceName.trim()) return;
    handleAdd(namingType, instanceName.trim());
    setNamingType(null);
    setInstanceName("");
  };

  const handleDelete = async (name) => {
    setSaving("del");
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/connectors/${encodeURIComponent(name)}`,
        {
          method: "DELETE",
          headers: apiHeaders(),
        },
      );
      if (resp.ok) {
        setSelected(null);
        setConfirmDelete(false);
        setStatus({ type: "ok", text: "Removed" });
        load();
      } else {
        setStatus({ type: "error", text: "Failed to delete" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleSaveSecret = async (key) => {
    const value = secretValues[key];
    if (!value?.trim()) return;
    setSaving(key);
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/vault/${encodeURIComponent(key)}`,
        {
          method: "PUT",
          headers: apiHeaders(),
          body: JSON.stringify({ value: value.trim() }),
        },
      );
      if (resp.ok) {
        setSecretValues((prev) => ({ ...prev, [key]: "" }));
        setStatus({ type: "ok", text: `Saved ${key}` });
        load();
      } else {
        setStatus({ type: "error", text: `Failed to save ${key}` });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  const handleSaveConfig = async (name) => {
    setSaving("config");
    setStatus(null);
    try {
      const resp = await fetch(
        `/api/settings/connectors/${encodeURIComponent(name)}`,
        {
          method: "PUT",
          headers: apiHeaders(),
          body: JSON.stringify({ config: editConfig }),
        },
      );
      if (resp.ok) {
        setStatus({ type: "ok", text: "Config saved" });
        load();
      } else {
        setStatus({ type: "error", text: "Failed to save config" });
      }
    } catch (e) {
      setStatus({ type: "error", text: e.message });
    }
    setSaving(null);
  };

  // OAuth via Spawner proxy
  const handleProxyOAuth = async (connName, providerKey) => {
    setSaving("oauth-connect");
    setStatus(null);
    const sessionId = crypto.randomUUID();
    const provider = proxyProviders[providerKey];
    const scopes = provider?.scopes?.join(",") || "";
    const url = `${oauthProxyUrl}/api/v1/oauth/connect/${encodeURIComponent(providerKey)}?session=${sessionId}&scopes=${encodeURIComponent(scopes)}`;

    const w = 600,
      h = 700;
    const left = window.screenX + (window.innerWidth - w) / 2;
    const top = window.screenY + (window.innerHeight - h) / 2;
    window.open(url, "oauth", `width=${w},height=${h},left=${left},top=${top}`);

    setOauthPolling({ sessionId, connName, providerKey });
    setStatus({ type: "ok", text: "Waiting for authorization..." });

    // Poll for completion
    const poll = async () => {
      const maxAttempts = 60; // 5 minutes at 5s intervals
      for (let i = 0; i < maxAttempts; i++) {
        await new Promise((r) => setTimeout(r, 3000));
        try {
          const resp = await fetch(
            `${oauthProxyUrl}/api/v1/oauth/sessions/${sessionId}`,
          );
          if (!resp.ok) continue;
          const data = await resp.json();
          if (data.status === "completed" && data.access_token) {
            // Store the token in the local vault
            const conn = connectors.find((c) => c.name === connName);
            const tokenKey =
              conn?.oauth_token_key ||
              connName.toUpperCase().replace(/-/g, "_") + "_TOKEN";
            await fetch(`/api/settings/vault/${encodeURIComponent(tokenKey)}`, {
              method: "PUT",
              headers: apiHeaders(),
              body: JSON.stringify({ value: data.access_token }),
            });
            // Store refresh token if present
            if (data.refresh_token) {
              const refreshKey =
                connName.toUpperCase().replace(/-/g, "_") + "_REFRESH_TOKEN";
              await fetch(
                `/api/settings/vault/${encodeURIComponent(refreshKey)}`,
                {
                  method: "PUT",
                  headers: apiHeaders(),
                  body: JSON.stringify({ value: data.refresh_token }),
                },
              );
            }
            // Update connector status + OAuth metadata
            const connUpdate = { status: "connected" };
            if (data.refresh_token) {
              connUpdate.oauth_refresh_key =
                connName.toUpperCase().replace(/-/g, "_") + "_REFRESH_TOKEN";
            }
            if (data.expires_in) {
              const expiresAt = new Date(
                Date.now() + data.expires_in * 1000,
              ).toISOString();
              connUpdate.oauth_expires_at = expiresAt;
            }
            await fetch(
              `/api/settings/connectors/${encodeURIComponent(connName)}`,
              {
                method: "PUT",
                headers: apiHeaders(),
                body: JSON.stringify(connUpdate),
              },
            );
            setStatus({ type: "ok", text: "Connected via OAuth" });
            setOauthPolling(null);
            setSaving(null);
            load();
            return;
          } else if (data.status === "error") {
            setStatus({
              type: "error",
              text: data.error_message || "OAuth failed",
            });
            setOauthPolling(null);
            setSaving(null);
            return;
          }
        } catch {
          // Network error, keep polling
        }
      }
      setStatus({ type: "error", text: "OAuth timed out" });
      setOauthPolling(null);
      setSaving(null);
    };
    poll();
  };

  const openDetail = (conn) => {
    setSelected(conn.name);
    setEditConfig(conn.config || {});
    setSecretValues({});
    setConfirmDelete(false);
    setStatus(null);
  };

  const selectedConn = connectors.find((c) => c.name === selected);
  const selectedTpl = selectedConn
    ? templates.find((t) => t.name === selectedConn.type)
    : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "16px" }}>
      {/* Header + Search */}
      <div
        style={{
          display: "flex",
          alignItems: "flex-end",
          justifyContent: "space-between",
          gap: "16px",
        }}
      >
        <div>
          <div
            style={{
              fontSize: "13px",
              fontWeight: 600,
              color: "var(--color-secondary)",
              marginBottom: "4px",
            }}
          >
            Connectors
          </div>
          <div style={{ fontSize: "11px", color: "var(--color-dim)" }}>
            Connect external services. Click to configure.
          </div>
        </div>
        <div style={{ position: "relative", flexShrink: 0 }}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="var(--color-dim)"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              position: "absolute",
              left: "8px",
              top: "50%",
              transform: "translateY(-50%)",
              pointerEvents: "none",
            }}
          >
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            className="s-input"
            placeholder="Filter..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            style={{
              width: "180px",
              fontSize: "12px",
              paddingLeft: "28px",
              fontFamily: "var(--font-mono)",
            }}
          />
        </div>
      </div>

      {/* Grid */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(100px, 1fr))",
          gap: "1px",
          background: "var(--color-border-subtle)",
          border: "1px solid var(--color-border-subtle)",
        }}
      >
        {filteredTemplates.map((tpl) => {
          const conn = connectors.find((c) => c.type === tpl.name);
          const isConnected = conn?.status === "connected";
          const isConfigured = !!conn;
          const isActive = selected === conn?.name;

          return (
            <button
              key={tpl.name}
              onClick={() => handleTileClick(tpl)}
              disabled={saving === "add"}
              style={{
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                justifyContent: "center",
                gap: "8px",
                padding: "16px 8px",
                background: isActive
                  ? "var(--color-elevated)"
                  : isConfigured
                    ? "var(--color-surface)"
                    : "var(--color-bg)",
                border: "none",
                cursor: "pointer",
                transition: "background 0.15s ease",
                position: "relative",
              }}
              onMouseEnter={(e) => {
                if (!isActive)
                  e.currentTarget.style.background = "var(--color-elevated)";
              }}
              onMouseLeave={(e) => {
                if (!isActive)
                  e.currentTarget.style.background = isConfigured
                    ? "var(--color-surface)"
                    : "var(--color-bg)";
              }}
              title={
                isConfigured
                  ? `${tpl.display_name} — ${conn.status}`
                  : `Add ${tpl.display_name}`
              }
            >
              {isConfigured && (
                <span
                  style={{
                    position: "absolute",
                    top: 8,
                    right: 8,
                    width: 6,
                    height: 6,
                    borderRadius: "50%",
                    background: isConnected
                      ? "var(--color-ok)"
                      : "var(--color-dim)",
                  }}
                />
              )}
              <span
                style={{
                  color: isConfigured
                    ? "var(--color-primary)"
                    : "var(--color-dim)",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  transition: "color 0.15s ease",
                }}
              >
                {LOGOS[tpl.name] || FALLBACK_LOGO}
              </span>
              <span
                style={{
                  fontSize: "10px",
                  textAlign: "center",
                  lineHeight: 1.2,
                  maxWidth: "80px",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  color: isConfigured
                    ? "var(--color-secondary)"
                    : "var(--color-dim)",
                }}
              >
                {tpl.display_name}
              </span>
            </button>
          );
        })}
      </div>

      {filteredTemplates.length === 0 && (
        <div
          style={{
            textAlign: "center",
            padding: "24px",
            color: "var(--color-dim)",
            fontSize: "12px",
          }}
        >
          No connectors match "{search}"
        </div>
      )}

      {/* Multi-instance name prompt */}
      {namingType && (
        <div
          style={{
            border: "1px solid var(--color-border-subtle)",
            background: "var(--color-surface)",
            padding: "12px 16px",
            display: "flex",
            flexDirection: "column",
            gap: "8px",
          }}
        >
          <span style={{ fontSize: "12px", color: "var(--color-secondary)" }}>
            Name this{" "}
            {templates.find((t) => t.name === namingType)?.display_name}{" "}
            instance
          </span>
          <div style={{ display: "flex", gap: "8px" }}>
            <input
              className="s-input"
              placeholder="e.g. analytics-db"
              value={instanceName}
              onChange={(e) =>
                setInstanceName(
                  e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, ""),
                )
              }
              onKeyDown={(e) => e.key === "Enter" && handleNameSubmit()}
              style={{
                flex: 1,
                fontFamily: "var(--font-mono)",
                fontSize: "13px",
              }}
              autoFocus
            />
            <button
              className="s-save-btn"
              style={{ padding: "4px 12px", fontSize: "12px" }}
              disabled={!instanceName.trim() || saving === "add"}
              onClick={handleNameSubmit}
            >
              {saving === "add" ? "..." : "Add"}
            </button>
            <button
              className="s-save-btn"
              style={{
                background: "transparent",
                color: "var(--color-muted)",
                border: "1px solid var(--color-border-main)",
                padding: "4px 12px",
                fontSize: "12px",
              }}
              onClick={() => setNamingType(null)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Detail panel */}
      {selectedConn && (
        <div
          style={{
            border: "1px solid var(--color-border-subtle)",
            background: "var(--color-surface)",
          }}
        >
          {/* Header */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "12px 16px",
              borderBottom: "1px solid var(--color-border-subtle)",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
              <span style={{ color: "var(--color-primary)", display: "flex" }}>
                {LOGOS[selectedConn.type] || FALLBACK_LOGO}
              </span>
              <div>
                <div
                  style={{
                    fontSize: "13px",
                    fontWeight: 600,
                    color: "var(--color-primary)",
                  }}
                >
                  {selectedConn.display_name}
                  {selectedConn.name !== selectedConn.type && (
                    <span
                      style={{
                        fontWeight: 400,
                        fontSize: "11px",
                        color: "var(--color-dim)",
                        marginLeft: "8px",
                        fontFamily: "var(--font-mono)",
                      }}
                    >
                      {selectedConn.name}
                    </span>
                  )}
                </div>
                <div style={{ fontSize: "11px", color: "var(--color-dim)" }}>
                  {selectedConn.description}
                </div>
              </div>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
              {selectedConn.status === "connected" && (
                <span
                  style={{
                    fontSize: "11px",
                    color: "var(--color-ok)",
                    display: "flex",
                    alignItems: "center",
                    gap: "4px",
                  }}
                >
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: "50%",
                      background: "var(--color-ok)",
                    }}
                  />
                  Connected
                </span>
              )}
              <button
                style={{
                  background: "none",
                  border: "none",
                  color: "var(--color-dim)",
                  cursor: "pointer",
                  padding: "4px",
                  fontSize: "16px",
                  lineHeight: 1,
                }}
                onClick={() => setSelected(null)}
              >
                &times;
              </button>
            </div>
          </div>

          <div
            style={{
              padding: "12px 16px",
              display: "flex",
              flexDirection: "column",
              gap: "16px",
            }}
          >
            {/* Slack Socket Mode guided setup. Replaces the generic
                Secrets / OAuth UI for connectors flagged socket_mode in
                their template, because Socket Mode bots can't be set up
                via OAuth distribution and need a specific manifest+token
                walkthrough. */}
            {selectedTpl?.socket_mode && selectedConn.type === "slack" && (
              <SlackSetupGuide
                vaultKeys={vaultKeys}
                onSaved={(opts) => {
                  load();
                  if (opts && !opts.silent) {
                    setStatus({
                      type: "ok",
                      text: opts.text || "Connected",
                    });
                  }
                }}
                onError={(text) => setStatus({ type: "error", text })}
              />
            )}

            {/* Secrets */}
            {!selectedTpl?.socket_mode && selectedConn.secrets?.length > 0 && (
              <Section title="Secrets">
                {selectedConn.secrets.map((key) => {
                  const isSet = vaultKeys.has(key);
                  return (
                    <div
                      key={key}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "8px",
                        minHeight: "32px",
                      }}
                    >
                      <span
                        style={{
                          fontSize: "12px",
                          fontFamily: "var(--font-mono)",
                          width: "200px",
                          flexShrink: 0,
                          color: isSet
                            ? "var(--color-secondary)"
                            : "var(--color-warn)",
                        }}
                      >
                        {key}
                      </span>
                      {isSet ? (
                        <span
                          style={{ fontSize: "11px", color: "var(--color-ok)" }}
                        >
                          &check; set
                        </span>
                      ) : (
                        <>
                          <input
                            className="s-input"
                            type="password"
                            placeholder="Paste value..."
                            value={secretValues[key] || ""}
                            onChange={(e) =>
                              setSecretValues((prev) => ({
                                ...prev,
                                [key]: e.target.value,
                              }))
                            }
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
                            disabled={
                              !secretValues[key]?.trim() || saving === key
                            }
                            onClick={() => handleSaveSecret(key)}
                          >
                            {saving === key ? "..." : "Save"}
                          </button>
                        </>
                      )}
                    </div>
                  );
                })}
              </Section>
            )}

            {/* OAuth (suppressed for socket_mode connectors — those use
                their own guided setup component above). */}
            {!selectedTpl?.socket_mode &&
              (selectedConn.auth_method === "oauth" ||
                selectedConn.oauth_token_url) &&
              selectedTpl?.has_oauth &&
              (() => {
                // Check if this connector's type matches a proxy provider
                const proxyKey = Object.keys(proxyProviders).find((k) => {
                  // Match provider key to connector type (e.g. "github-connector" → "github")
                  const base = k.replace(/-connector$/, "");
                  return selectedConn.type === base || selectedConn.type === k;
                });
                return (
                  <Section title="OAuth">
                    {selectedConn.oauth_token_key && (
                      <KVRow
                        label="Access token"
                        value={selectedConn.oauth_token_key}
                        ok={vaultKeys.has(selectedConn.oauth_token_key)}
                      />
                    )}
                    {selectedConn.oauth_refresh_key && (
                      <KVRow
                        label="Refresh token"
                        value={selectedConn.oauth_refresh_key}
                        ok={vaultKeys.has(selectedConn.oauth_refresh_key)}
                      />
                    )}
                    {selectedConn.oauth_expires_at && (
                      <KVRow
                        label="Expires"
                        value={selectedConn.oauth_expires_at}
                      />
                    )}
                    {(selectedTpl.oauth_scopes ||
                      proxyProviders[proxyKey]?.scopes) && (
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "8px",
                          minHeight: "28px",
                        }}
                      >
                        <span
                          style={{
                            fontSize: "11px",
                            color: "var(--color-dim)",
                            width: "200px",
                            flexShrink: 0,
                          }}
                        >
                          Scopes
                        </span>
                        <div
                          style={{
                            display: "flex",
                            flexWrap: "wrap",
                            gap: "4px",
                          }}
                        >
                          {(
                            selectedTpl.oauth_scopes ||
                            proxyProviders[proxyKey]?.scopes ||
                            []
                          ).map((s) => (
                            <span
                              key={s}
                              style={{
                                padding: "1px 6px",
                                fontSize: "10px",
                                fontFamily: "var(--font-mono)",
                                border: "1px solid var(--color-border-main)",
                                color: "var(--color-dim)",
                              }}
                            >
                              {s}
                            </span>
                          ))}
                        </div>
                      </div>
                    )}

                    {oauthProxyUrl && proxyKey ? (
                      <div
                        style={{
                          marginTop: "8px",
                          display: "flex",
                          flexDirection: "column",
                          gap: "8px",
                        }}
                      >
                        <div
                          style={{
                            fontSize: "11px",
                            color: "var(--color-dim)",
                          }}
                        >
                          OAuth managed by Starpod — no credentials needed
                        </div>
                        {oauthPolling?.connName === selectedConn.name ? (
                          <div
                            style={{
                              display: "flex",
                              alignItems: "center",
                              gap: "8px",
                            }}
                          >
                            <span
                              style={{
                                display: "inline-block",
                                width: 8,
                                height: 8,
                                borderRadius: "50%",
                                background: "var(--color-accent)",
                                animation: "pulse 1.5s ease-in-out infinite",
                              }}
                            />
                            <span
                              style={{
                                fontSize: "12px",
                                color: "var(--color-secondary)",
                              }}
                            >
                              Waiting for authorization...
                            </span>
                          </div>
                        ) : (
                          <div
                            style={{
                              display: "flex",
                              justifyContent: "flex-end",
                            }}
                          >
                            <button
                              className="s-save-btn"
                              style={{ padding: "6px 16px", fontSize: "12px" }}
                              disabled={saving === "oauth-connect"}
                              onClick={() =>
                                handleProxyOAuth(selectedConn.name, proxyKey)
                              }
                            >
                              {selectedConn.status === "connected"
                                ? `Reconnect with ${selectedTpl.display_name}`
                                : `Connect with ${selectedTpl.display_name}`}
                            </button>
                          </div>
                        )}
                      </div>
                    ) : (
                      <div
                        style={{
                          marginTop: "8px",
                          fontSize: "11px",
                          color: "var(--color-dim)",
                        }}
                      >
                        OAuth not available — {oauthProxyUrl ? "provider not registered on proxy" : "no proxy configured"}
                      </div>
                    )}
                  </Section>
                );
              })()}

            {/* Config */}
            {Object.keys(editConfig).length > 0 && (
              <Section title="Configuration">
                {Object.entries(editConfig).map(([k, v]) => (
                  <div
                    key={k}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                      minHeight: "32px",
                    }}
                  >
                    <span
                      style={{
                        fontSize: "11px",
                        fontFamily: "var(--font-mono)",
                        color: "var(--color-secondary)",
                        width: "200px",
                        flexShrink: 0,
                      }}
                    >
                      {k}
                    </span>
                    <input
                      className="s-input"
                      value={v}
                      onChange={(e) =>
                        setEditConfig((prev) => ({
                          ...prev,
                          [k]: e.target.value,
                        }))
                      }
                      style={{
                        flex: 1,
                        fontFamily: "var(--font-mono)",
                        fontSize: "12px",
                      }}
                    />
                  </div>
                ))}
                <div
                  style={{
                    display: "flex",
                    justifyContent: "flex-end",
                    marginTop: "4px",
                  }}
                >
                  <button
                    className="s-save-btn"
                    style={{ padding: "4px 12px", fontSize: "11px" }}
                    disabled={saving === "config"}
                    onClick={() => handleSaveConfig(selectedConn.name)}
                  >
                    {saving === "config" ? "..." : "Save config"}
                  </button>
                </div>
              </Section>
            )}

            {/* Remove */}
            <div
              style={{
                borderTop: "1px solid var(--color-border-subtle)",
                paddingTop: "12px",
                display: "flex",
                justifyContent: "flex-end",
              }}
            >
              {confirmDelete ? (
                <div
                  style={{ display: "flex", alignItems: "center", gap: "8px" }}
                >
                  <span style={{ fontSize: "12px", color: "var(--color-err)" }}>
                    Remove this connector?
                  </span>
                  <button
                    className="s-save-btn"
                    style={{
                      background: "var(--color-err)",
                      padding: "4px 12px",
                      fontSize: "11px",
                    }}
                    onClick={() => handleDelete(selectedConn.name)}
                    disabled={saving === "del"}
                  >
                    {saving === "del" ? "..." : "Yes, remove"}
                  </button>
                  <button
                    className="s-save-btn"
                    style={{
                      background: "transparent",
                      color: "var(--color-muted)",
                      border: "1px solid var(--color-border-main)",
                      padding: "4px 12px",
                      fontSize: "11px",
                    }}
                    onClick={() => setConfirmDelete(false)}
                  >
                    Cancel
                  </button>
                </div>
              ) : (
                <button
                  style={{
                    background: "none",
                    border: "none",
                    color: "var(--color-dim)",
                    cursor: "pointer",
                    fontSize: "12px",
                    padding: "4px 0",
                  }}
                  onClick={() => setConfirmDelete(true)}
                >
                  Remove connector
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Status toast */}
      {status && (
        <div
          style={{
            fontSize: "12px",
            color:
              status.type === "ok" ? "var(--color-ok)" : "var(--color-err)",
          }}
        >
          {status.text}
        </div>
      )}
    </div>
  );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function Section({ title, children }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
      <span
        style={{
          fontSize: "10px",
          color: "var(--color-dim)",
          textTransform: "uppercase",
          letterSpacing: "0.08em",
          fontWeight: 600,
        }}
      >
        {title}
      </span>
      {children}
    </div>
  );
}

function KVRow({ label, value, ok }) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "8px",
        minHeight: "28px",
      }}
    >
      <span
        style={{
          fontSize: "11px",
          color: "var(--color-dim)",
          width: "200px",
          flexShrink: 0,
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontSize: "11px",
          fontFamily: "var(--font-mono)",
          color:
            ok === true
              ? "var(--color-ok)"
              : ok === false
                ? "var(--color-warn)"
                : "var(--color-secondary)",
        }}
      >
        {value} {ok === true && "\u2713"}
        {ok === false && "(missing)"}
      </span>
    </div>
  );
}
