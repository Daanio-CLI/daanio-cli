# Telemetry Worker Installation Guide

This guide explains how to install the tooling and Cloudflare resources used by
the Daanio telemetry worker.

For production updates, migrations, verification, and rollback, continue with
the [Telemetry Worker Deployment Runbook](./DEPLOYMENT.md) after installation.

## End users do not install the telemetry endpoint

`https://telemetry.daanio.com/v1/event` is an HTTP ingestion endpoint. It is
not an application, package, or installer.

The Daanio CLI already sends eligible anonymous telemetry events to this
endpoint automatically. Opening the URL in a browser sends a GET request and
correctly returns `405 Method Not Allowed` because event ingestion accepts POST
requests only.

Users who install Daanio do not need to install Wrangler, create a Cloudflare
account, or run anything in this directory.

Telemetry remains optional. Users can disable it with any supported opt-out:

```bash
export DAANIO_NO_TELEMETRY=1
```

or:

```bash
export DO_NOT_TRACK=1
```

or:

```bash
touch ~/.daanio/no_telemetry
```

## Operator prerequisites

Operators installing or maintaining the telemetry service need:

- Git
- A current Node.js LTS release and npm
- Access to the Cloudflare account that owns the Daanio Worker
- Permission to manage Workers, D1, Analytics Engine, routes, and cron triggers
- A narrowly scoped Cloudflare API token for automation, or Wrangler browser
  login for interactive administration

Confirm the local tools:

```bash
git --version
node --version
npm --version
```

## Install the worker dependencies

From the Daanio repository root:

```bash
cd telemetry-worker
npm install
```

This installs Wrangler 4 from the `devDependencies` in `package.json`.

The worker currently has no committed `package-lock.json`, so use
`npm install`, not `npm ci`. If the project later commits a lockfile, CI and
production procedures can move to `npm ci` for reproducible dependency
installation.

Verify Wrangler:

```bash
npx wrangler --version
```

## Authenticate Wrangler

For an interactive operator workstation:

```bash
npx wrangler login
npx wrangler whoami
```

For CI, configure a secret named `CLOUDFLARE_API_TOKEN` in the CI platform.
Grant only the permissions required for this worker and its D1 and Analytics
Engine resources.

Never place an API token in:

- `wrangler.toml`
- shell history
- committed `.env` files
- command output captured in public CI logs

The D1 database ID in `wrangler.toml` identifies a resource and is not a bearer
credential.

## Install for local development

Create the local D1 schema:

```bash
npx wrangler d1 execute daanio-telemetry --local --file=schema.sql
```

Run the worker locally:

```bash
npm run dev
```

Wrangler prints the local URL. In another terminal, replace the example port
with the port Wrangler selected:

```bash
curl -fsS http://127.0.0.1:8787/v1/health
```

Run the test suite:

```bash
npm test
```

Local Wrangler state is separate from the production D1 database. Commands
must include `--remote` before they can affect production.

## Connect to the existing production service

The repository's `wrangler.toml` already names the production resources:

- Worker `daanio-telemetry`
- D1 database `daanio-telemetry`
- Custom domain `telemetry.daanio.com`
- Analytics Engine datasets used by CLI, web, discovery, and install events

Do not run `wrangler d1 create` and do not replace the checked-in database ID
when connecting to the existing service.

Confirm access without mutating production:

```bash
npx wrangler whoami
npx wrangler d1 execute daanio-telemetry --remote --command "SELECT 1"
curl -fsS https://telemetry.daanio.com/v1/health
```

Once these checks succeed, follow [DEPLOYMENT.md](./DEPLOYMENT.md) for migration
and deployment steps.

## First installation in a new Cloudflare account

This section is only for creating a separate new environment. Do not use it to
update the existing production service.

### 1. Create a D1 database

```bash
npx wrangler d1 create daanio-telemetry
```

Copy the returned database ID into that environment's `[[d1_databases]]`
binding.

### 2. Initialize the remote database

```bash
npx wrangler d1 execute daanio-telemetry --remote --file=schema.sql
```

Review the current schema and numbered migrations before applying additional
migrations. Do not blindly replay migration files already represented in the
schema snapshot.

### 3. Enable Analytics Engine

Enable Workers Analytics Engine in the Cloudflare dashboard. The checked-in
worker binds four datasets, and deployment can fail with Cloudflare error
`10089` when Analytics Engine is unavailable.

For an environment intentionally running without Analytics Engine, remove the
Analytics Engine bindings only in that environment's configuration. The worker
can use D1 alone, but high-volume firehose storage will be unavailable.

### 4. Configure domains

Update the custom-domain routes for the new environment. Never point a staging
installation at `telemetry.daanio.com`.

Cloudflare creates the required DNS records and certificates for Workers custom
domains during deployment.

### 5. Deploy

```bash
npm test
npm run deploy
```

Then verify the new environment's `/v1/health` endpoint.

## Verify installation

For production:

```bash
curl -fsS https://telemetry.daanio.com/v1/health
```

A healthy response contains:

```json
{
  "ok": true,
  "over_soft_limit": false
}
```

Confirm the event endpoint exists and rejects browser-style GET requests:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' \
  https://telemetry.daanio.com/v1/event
```

Expected status: `405`.

Do not send invented events to the production endpoint to test it. Synthetic
events contaminate product analytics. Use local development, unit tests, or a
dedicated staging environment.

## Updating the installed tooling

From `telemetry-worker`:

```bash
npm install
npx wrangler --version
npm test
```

Review dependency changes before committing them. If a lockfile is introduced,
commit it and update this guide and the deployment runbook to use `npm ci`.

## Common installation problems

### `npm ci` reports that a lockfile is required

Use `npm install`. This project does not currently commit a package lock.

### `npx wrangler whoami` is unauthenticated

Run `npx wrangler login` interactively or provide a valid, narrowly scoped
`CLOUDFLARE_API_TOKEN` in CI.

### Local health reports a missing D1 table

Initialize local state with:

```bash
npx wrangler d1 execute daanio-telemetry --local --file=schema.sql
```

### Production health reports a missing table or column

Do not reinitialize production. Identify and apply only the missing numbered
migration with `--remote`, following the deployment runbook.

### Deployment fails with error `10089`

Enable Workers Analytics Engine for the Cloudflare account or use an
environment-specific configuration without Analytics Engine bindings.

### `/v1/event` shows `405 Method Not Allowed`

Installation is not broken. GET is intentionally rejected. Use `/v1/health`
for service checks; Daanio clients send POST requests to `/v1/event`.

## Installation checklist

- [ ] Node.js and npm available
- [ ] `npm install` completed
- [ ] `npm test` passed
- [ ] Wrangler authentication confirmed
- [ ] Correct Cloudflare account confirmed
- [ ] Existing production D1 ID preserved, or new environment D1 created
- [ ] Analytics Engine available or intentionally disabled for the environment
- [ ] Local or remote schema initialized as appropriate
- [ ] `/v1/health` returns `ok: true`
- [ ] Deployment runbook reviewed before production changes
