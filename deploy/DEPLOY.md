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

## After deployment

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
