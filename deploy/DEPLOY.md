# Deploying csaf-trove

## Prerequisites

- Ansible installed on your local machine
- SSH access to the target host (as root or with `become` privileges)
- A DNS record pointing to the target host (for Caddy TLS)

## Quick start

```sh
cd deploy
ansible-playbook -i 'your-host.example.com,' -u root playbook.yml
```

The trailing comma after the hostname is required.

## Overriding defaults

Pass variables with `-e`:

```sh
ansible-playbook -i 'your-host.example.com,' -u root playbook.yml \
  -e csaf_trove_version=v0.2.0 \
  -e csaf_trove_domain=csaf.example.com
```

See `roles/csaf-trove/defaults/main.yml` for all available variables.

## What gets deployed

- A single `csaf-trove-server` binary (dashboard embedded)
- systemd service with security hardening
- Caddy reverse proxy with automatic TLS (enabled by default)
- Auto-generated API token and webhook secret in `/etc/csaf-trove/`

### MCP endpoint

A read-only [MCP](https://modelcontextprotocol.io) endpoint can be served at `/mcp`, e.g. for
`claude mcp add --transport http csaf-trove https://csaf.example.com/mcp`. It is disabled by default:

```sh
ansible-playbook -i 'your-host.example.com,' -u root playbook.yml \
  -e csaf_trove_mcp_enabled=true
```

Requests are only accepted for the hosts in `csaf_trove_mcp_allowed_hosts`, which defaults to
`csaf_trove_domain`. The CVE search cache is tuned with `csaf_trove_cve_cache_capacity`,
`csaf_trove_cve_cache_ttl` and `csaf_trove_cve_cache_max_hits`.

### Rate limiting

API, MCP and aggregator requests are rate limited per client IP address (IPv6 per /64), behind
Caddy using the address it forwards. By default a client may make 60 requests per minute with
bursts of up to 60; requests over that are delayed by up to 10 seconds and then rejected with
`429 Too Many Requests`. Dashboard assets, GitHub webhooks and requests carrying the API token are
not limited. Tune or disable it with `csaf_trove_rate_limit_requests_per_minute`,
`csaf_trove_rate_limit_burst`, `csaf_trove_rate_limit_max_wait` and
`csaf_trove_rate_limit_enabled`.

## After deployment

### File descriptor limits

The service sets `LimitNOFILE=65536` for SQLite databases, Git repositories, and
HTTP sockets. Deploy the updated service template, reload systemd, and restart
the service to apply the new limit. No binary update is needed.

If a job reports `Too many open files`, inspect the running process on the host:

```sh
pid=$(systemctl show csaf-trove --property=MainPID --value)
cat /proc/"$pid"/limits
ls /proc/"$pid"/fd | wc -l
sudo ls -l /proc/"$pid"/fd
```

The last command identifies whether databases, Git packfiles, or sockets account
for most descriptors. A shell's `ulimit` does not show the running service's limit.

### Retrieve the webhook secret

```sh
ssh root@your-host.example.com cat /etc/csaf-trove/webhook_secret
```

Configure this as the webhook secret in your GitHub repository settings:
**Settings > Webhooks > Add webhook**

- Payload URL: `https://your-host.example.com/api/webhook/github`
- Content type: `application/json`
- Secret: (the value from above)
- Events: Just the push event

### Retrieve the API token

```sh
ssh root@your-host.example.com cat /etc/csaf-trove/api_token
```

Use this token to trigger manual syncs:

```sh
curl -X POST https://your-host.example.com/api/sync/redhat.com \
  -H "Authorization: Bearer <token>"
```

To re-validate all stored documents of a source without fetching anything
(runs in the background, returns `202 Accepted`):

```sh
curl -X POST https://your-host.example.com/api/sync/redhat.com/revalidate \
  -H "Authorization: Bearer <token>"
```
