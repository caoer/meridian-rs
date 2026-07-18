---
tags: [domain/network, type/reference]
source: '[[inbox/scraped/Sub-Store/sub-store-overview.md]]'
created: 2026-06-20
---

# Sub-Store

Advanced proxy subscription manager. Fetches, converts, filters, and aggregates subscriptions across all major proxy client formats. Web UI with scripting support.

Repo: [[sources/sub-store]] (backend) + [[Sub-Store/sub-store-front-end]] (frontend)

## Deployment

| Field | Value |
| -- | -- |
| URL | `https://sub.0xdao.app` |
| Host | forge-nyc-2 (NixOS service) |
| Port | 3090 (merge mode: API + frontend SPA) |
| Tunnel | cloudflared `zt-agent`, ingress `sub.0xdao.app` |
| Data | `/data/sub-store` (bind-mount to `/var/lib/sub-store`) |
| Config | `osfiles/hosts/forge-nyc-2/sub-store.nix` |
| Backend | `sub-store.bundle.js` v2.29.0 (pre-built from GitHub release) |
| Frontend | Built from source in `preStart` (Vue 3 + Vite SPA) |

### DNS Setup (manual, one-time)

Create proxied CNAME in Cloudflare dashboard for the `0xtau.com` zone:

```
sub.0xdao.app  CNAME  aa9fe85b-835f-40e5-8cc5-fb3eab15b97e.cfargotunnel.com
```

### Rebuild Frontend

```bash
ssh forge-nyc-2 'rm -rf /data/sub-store/frontend && systemctl restart sub-store'
```

## Role in Proxy Infrastructure

Sub-Store complements [[sources/mesh-network|mesh-network]], not replaces it:

|  | Sub-Store | mesh-network |
| -- | -- | -- |
| Purpose | Manage external provider subscriptions | Generate configs from owned server registry |
| Input | Subscription URLs from third-party providers | `registry/servers.toml` |
| Strength | Multi-provider aggregation, node filtering, scripting, web UI | Single-source-of-truth compilation, preset system |

Use Sub-Store to aggregate third-party subscriptions (airport services), apply node filters/scripts, and output to any client format. Use mesh-network for configs generated from the owned proxy server fleet.

## Format Support

**Input:** Surge, Clash/mihomo YAML, QX, Loon, URI schemes (SS, SSR, VMess, VLESS, Trojan, Hysteria 1/2, TUIC, WireGuard, AnyTLS, SSH), Socks5, HTTP(S).

**Output:** Surge, SurgeMac, Clash.Meta, Stash, Loon, Egern, Shadowrocket, QX, sing-box, V2Ray, V2Ray URI, plain JSON, Surfboard.

## See Also

- [[sources/sub-store]] — source summary
- [[sources/mesh-network]] — owned-server config generation
- [[subconverter]] — legacy converter being decommissioned
- [[proxy-config-pipeline-plan]] — migration plan
- [[gui-clients]] — clients that consume subscription output
