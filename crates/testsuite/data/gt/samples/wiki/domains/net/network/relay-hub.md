---
tags: [domain/network, type/reference]
source: '[[inbox/scraped/Relay-Hub/relay-hub-overview.md]]'
created: 2026-03-20
---

# Relay-Hub

Proxy subscription manager. Fetches, converts, filters, and aggregates subscriptions across the common proxy client formats. Web UI with scripting support.

Repo: [[sources/relay-hub]] (backend) + [[Relay-Hub/relay-hub-front-end]] (frontend)

## Deployment

| Field | Value |
| -- | -- |
| URL | `https://relay.example.test` |
| Host | node-b (declarative service) |
| Port | 3090 (merge mode: API + frontend SPA) |
| Tunnel | tunnel client `edge-agent`, ingress `relay.example.test` |
| Data | `/data/relay-hub` (bind-mount to `/var/lib/relay-hub`) |
| Config | `fleet/hosts/node-b/relay-hub.nix` |
| Backend | `relay-hub.bundle.js` v2.29.0 (pre-built from the upstream release) |
| Frontend | Built from source in `preStart` (Vue 3 + Vite SPA) |

### DNS Setup (manual, one-time)

Create a proxied CNAME in the DNS dashboard for the `example.test` zone:

```
relay.example.test  CNAME  00000000-1111-2222-3333-444444444444.tunnel.example.test
```

### Rebuild Frontend

```bash
ssh node-b 'rm -rf /data/relay-hub/frontend && systemctl restart relay-hub'
```

## Role in Proxy Infrastructure

Relay-Hub complements [[sources/mesh-config|mesh-config]], not replaces it:

|  | Relay-Hub | mesh-config |
| -- | -- | -- |
| Purpose | Manage external provider subscriptions | Generate configs from an owned server registry |
| Input | Subscription URLs from third-party providers | `registry/servers.toml` |
| Strength | Multi-provider aggregation, node filtering, scripting, web UI | Single-source-of-truth compilation, preset system |

Use Relay-Hub to aggregate third-party subscriptions, apply node filters/scripts, and output to any client format. Use mesh-config for configs generated from the owned proxy server fleet.

## Format Support

**Input:** Surge, Clash YAML, QX, Loon, URI schemes (SS, SSR, VMess, VLESS, Trojan, Hysteria 1/2, TUIC, WireGuard, SSH), Socks5, HTTP(S).

**Output:** Surge, Clash, Stash, Loon, Shadowrocket, QX, sing-box, V2Ray, plain JSON, Surfboard.

## See Also

- [[sources/relay-hub]] — source summary
- [[sources/mesh-config]] — owned-server config generation
- [[legacy-converter]] — legacy converter being decommissioned
- [[proxy-config-pipeline-plan]] — migration plan
- [[gui-clients]] — clients that consume subscription output
