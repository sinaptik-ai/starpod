# starpod-proxy

Local HTTP/HTTPS proxy that intercepts outbound traffic from tool subprocesses, scans for `starpod:v1:` opaque tokens, decrypts them, verifies host binding, and replaces them with real secret values before forwarding.

## API

```rust
// Start the proxy as a background tokio task
let handle = starpod_proxy::start_proxy(starpod_proxy::ProxyConfig {
    master_key: [0u8; 32],  // 32-byte AES-256-GCM key (same as vault)
    data_dir: PathBuf::from(".starpod/db"),
}).await?;

// Inject into tool subprocesses:
// HTTP_PROXY=http://127.0.0.1:{handle.port()}
// HTTPS_PROXY=http://127.0.0.1:{handle.port()}
// SSL_CERT_FILE={handle.ca_cert_path}  (when MITM enabled)

// Shutdown when done
handle.shutdown().await;
```

## How It Works

```
Tool subprocess (curl, npm, etc.)
    │ HTTP_PROXY=http://127.0.0.1:<port>
    ▼
starpod-proxy (tokio background task)
    ├─ HTTP: scan headers+body → replace tokens → forward
    ├─ HTTPS CONNECT: MITM with ephemeral certs → scan → forward
    ▼
Target server (api.github.com, etc.)
```

1. The proxy binds to `127.0.0.1:0` (OS-assigned port)
2. Tool subprocesses receive `HTTP_PROXY`/`HTTPS_PROXY` env vars
3. For HTTP requests: the proxy scans headers and body for `starpod:v1:` tokens
4. For HTTPS CONNECT: the proxy generates an ephemeral TLS certificate for the target hostname (signed by a local CA), terminates TLS, scans the decrypted traffic, then forwards over a real TLS connection to the target
5. For each token found: decrypt → check host binding → replace with real value (or strip if host mismatch)

## Token Scanning

The scanner finds `starpod:v1:` prefixes in byte buffers and replaces them:

```rust
use starpod_proxy::scan::{scan_and_replace, cipher_from_key};

let cipher = cipher_from_key(&master_key);
let result = scan_and_replace(&cipher, request_body, "api.github.com");
// result.data = body with tokens replaced
// result.replaced = count of tokens swapped for real values
// result.stripped = count of tokens removed (host mismatch)
```

**Host binding enforcement:**
- Token's `allowed_hosts` matches target → replace with real value
- Token's `allowed_hosts` doesn't match → strip token (empty string)
- Token decryption fails → leave token as-is

## Host Matching

```rust
use starpod_proxy::host_match::host_matches;

host_matches("api.github.com", &[]);                        // true (unrestricted)
host_matches("api.github.com", &["api.github.com".into()]); // true (exact)
host_matches("api.github.com", &["*.github.com".into()]);   // true (glob)
host_matches("evil.com", &["api.github.com".into()]);       // false
```

## Certificate Authority (feature: `mitm`)

On first proxy start, a self-signed CA is generated and persisted:

| File | Description |
|------|-------------|
| `.starpod/db/proxy-ca.pem` | CA certificate |
| `.starpod/db/proxy-ca-key.pem` | CA private key (0600 permissions) |
| `.starpod/db/proxy-ca-bundle.pem` | System roots + local CA (set as `SSL_CERT_FILE`) |

Ephemeral per-host certificates are generated on the fly for each HTTPS CONNECT request, signed by the local CA. Tools trust the CA via `SSL_CERT_FILE`, `NODE_EXTRA_CA_CERTS`, and `REQUESTS_CA_BUNDLE` env vars.

## Tiered Isolation

The proxy automatically selects the strongest isolation available:

| Tier | Condition | Mechanism | Bypass-proof? |
|------|-----------|-----------|---------------|
| **Tier 1** | Linux + `CAP_NET_ADMIN` + `netns` feature | Network namespace: veth pair + iptables DNAT | Yes — kernel enforced |
| **Tier 0** | All other platforms | `HTTP_PROXY`/`HTTPS_PROXY` env vars | No — tools can ignore |

Startup logging:
```
proxy: network namespace isolation (linux/netns)   # Tier 1
proxy: env var mode (netns unavailable: macos)     # Tier 0
proxy: env var mode (missing CAP_NET_ADMIN)        # Tier 0
```

## Network Namespace Isolation (feature: `netns`)

When Tier 1 is available, the proxy creates a network namespace:

1. `ip netns add starpod-ns` — create isolated network namespace
2. veth pair: `sp-veth0` (host, 10.200.1.1/24) ↔ `sp-veth1` (namespace, 10.200.1.2/24)
3. Default route in namespace → host veth
4. iptables DNAT: all traffic from namespace on ports 80/443 → proxy
5. Tool subprocesses enter the namespace via `setns()` in a `pre_exec` hook

The namespace is cleaned up when the proxy handle is dropped.

## Feature Flags

| Feature | Dependencies | What it enables |
|---------|-------------|-----------------|
| `mitm` | `rcgen`, `tokio-rustls`, `rustls`, `rustls-pemfile`, `webpki-roots` | HTTPS MITM with ephemeral certs, CA management |
| `netns` | `nix` | Linux network namespace isolation (Tier 1) |

## ProxyHandle

| Field | Type | Description |
|-------|------|-------------|
| `addr` | `SocketAddr` | `127.0.0.1:<port>` |
| `ca_cert_path` | `Option<PathBuf>` | CA bundle path (when MITM enabled) |
| `ns_handle` | `Option<NamespaceHandle>` | Network namespace (when Tier 1) |

| Method | Description |
|--------|-------------|
| `port()` | Assigned proxy port |
| `shutdown()` | Graceful async shutdown |
| `pre_exec_hook()` | Namespace entry closure for `Command::pre_exec()` (Tier 1 only) |

## Error Handling

- **Proxy bind failure**: `StarpodError::Proxy` — agent logs and continues without proxy
- **Token decode failure**: token left as-is in the request (logged at `warn`)
- **Host mismatch**: token stripped (empty string), logged at `warn`
- **MITM CA failure**: falls back to blind CONNECT tunnel
- **Namespace creation failure**: falls back to Tier 0 (env vars)
- **Target connection failure**: HTTP 502 Bad Gateway to subprocess

## Tests

- Token scanning: 10 tests (replacement, stripping, edge cases)
- Host matching: 10 tests (exact, glob, case, multiple)
- CA management: 7 tests (generate, load, issue, concurrent, bundle, permissions)
- Proxy lifecycle: 5 tests (startup, shutdown, token replacement, CA generation)
- Tier detection: 1 test
