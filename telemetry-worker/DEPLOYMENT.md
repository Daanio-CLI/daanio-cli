# Telemetry Worker Deployment Runbook

This runbook deploys the Cloudflare Worker that serves:

- `POST https://telemetry.daanio.com/v1/event`
- `GET https://telemetry.daanio.com/v1/health`

The event URL is an ingestion endpoint, not an installer or web page. A browser
normally sends `GET /v1/event`, which correctly returns `405 Method Not
Allowed`.

## Production resources

The checked-in [`wrangler.toml`](./wrangler.toml) currently targets:

- Worker: `daanio-telemetry`
- D1 binding: `DB`
- D1 database: `daanio-telemetry`
- Custom domains: `telemetry.daanio.com` and
  `telemetry.solosystems.dev`
- Analytics Engine datasets:
  - `daanio_telemetry_firehose`
  - `daanio_web_firehose`
  - `daanio_discovery_firehose`
  - `daanio_install_firehose`
- Scheduled retention job: `17 4 * * *`

Do not replace the production D1 database ID or dataset names during a normal
deployment.

## Prerequisites

You need:

1. Node.js and npm.
2. Access to the Cloudflare account that owns the worker, D1 database, custom
   domains, and Analytics Engine datasets.
3. A Cloudflare API token or an interactive Wrangler login with permission to
   deploy Workers and update D1.
4. Analytics Engine enabled for the Cloudflare account. All four bindings in
   `wrangler.toml` are active; deployment can fail with Cloudflare error `10089`
   when Analytics Engine is disabled.
5. A clean checkout of the intended production commit.

From the repository root:

```bash
cd telemetry-worker
npm ci
npx wrangler whoami
```

If Wrangler is not authenticated:

```bash
npx wrangler login
```

For CI, provide a narrowly scoped `CLOUDFLARE_API_TOKEN` secret instead of
using interactive login. Never commit the token or print it in logs.

## Normal production deployment

### 1. Review the deployment

Confirm the Git commit and review changes to the worker, schema, migrations,
and bindings:

```bash
git status --short
git log -1 --oneline
git diff HEAD^ -- src/ migrations/ schema.sql wrangler.toml package.json
```

Do not deploy from a dirty checkout unless every uncommitted change is intended
for production.

### 2. Install locked dependencies and run tests

```bash
npm ci
npm test
```

Stop if tests fail.

### 3. Apply required D1 migrations

Apply only migrations that production has not already received, in numeric
order. These files use forward schema changes and must not be blindly replayed.

Available npm aliases:

```bash
npm run migrate:expand
npm run migrate:transport
npm run migrate:usage
npm run migrate:phase123
npm run migrate:workflow
npm run migrate:tokens
npm run migrate:dashboard-indexes
npm run migrate:agent-time
npm run migrate:feedback-text
npm run migrate:daily-active
npm run migrate:daily-active-backfill
npm run migrate:daily-active-ci
npm run migrate:detail-fields
npm run migrate:dau-full-backfill
npm run migrate:auth-failure-reason
npm run migrate:web-subscription
npm run migrate:discovery
npm run migrate:web-quality
npm run migrate:install-conversion
npm run migrate:todo-telemetry
```

Migration `0019_discovery_benchmark_runs.sql` does not currently have an npm
alias. Apply it directly when required:

```bash
npx wrangler d1 execute daanio-telemetry --remote \
  --file=migrations/0019_discovery_benchmark_runs.sql
```

Important migration rules:

- Always include `--remote` for production.
- Apply migrations before deploying code that requires their new tables or
  columns.
- Record the last applied migration in the deployment ticket or operations
  log. The repository currently uses direct SQL execution rather than relying
  on Wrangler's migration ledger.
- Some migrations contain backfills and can take longer than simple schema
  changes. Do not interrupt them merely because they are quiet.
- Do not add more columns to the main `events` table. It is close to D1's
  100-column limit; use the appropriate detail table.

### 4. Deploy the worker

```bash
npm run deploy
```

Wrangler uploads the worker, binds the existing D1 database and Analytics
Engine datasets, configures the cron trigger, and updates the configured custom
domains.

Save the deployment/version identifier printed by Wrangler. It is needed for a
fast worker-code rollback.

### 5. Verify production

Check the public health endpoint:

```bash
curl -fsS https://telemetry.daanio.com/v1/health
```

With `jq` installed:

```bash
curl -fsS https://telemetry.daanio.com/v1/health | jq
```

A healthy response resembles:

```json
{
  "ok": true,
  "db_size_bytes": 123456,
  "db_soft_limit_bytes": 471859200,
  "over_soft_limit": false,
  "last_emergency_prune_at_ms": null
}
```

Confirm that the event endpoint rejects the wrong method as expected:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' \
  https://telemetry.daanio.com/v1/event
```

Expected result: `405`.

Run the D1 health query:

```bash
npm run health
```

Watch production logs while a real opted-in Daanio client sends an event:

```bash
npx wrangler tail daanio-telemetry
```

Avoid sending fabricated production events merely to test ingestion; they
pollute product metrics. Use the worker unit tests or a non-production Wrangler
environment for synthetic payloads.

### 6. Observe after deployment

For at least several minutes, verify:

- `/v1/health` remains `ok: true`.
- `over_soft_limit` remains `false`.
- Worker logs do not show repeated D1 or Analytics Engine failures.
- Normal telemetry POST requests return success.
- Daily-active and install metrics remain plausible rather than suddenly
  dropping to zero or multiplying.

Useful read-only commands:

```bash
npm run health:size
npm run health
npm run dau
npm run users
```

## First-time deployment to a new Cloudflare account

Do not follow this section for a normal production update.

### 1. Create the D1 database

```bash
npx wrangler d1 create daanio-telemetry
```

Copy the returned database ID into the `[[d1_databases]]` section of
`wrangler.toml`.

### 2. Initialize the remote schema

```bash
npx wrangler d1 execute daanio-telemetry --remote --file=schema.sql
```

Then apply any numbered migrations newer than the schema snapshot, in order.
Check the schema and migration history before running them; do not assume every
migration is still needed after initializing from the current `schema.sql`.

### 3. Enable Analytics Engine

Enable Workers Analytics Engine in the Cloudflare dashboard before deploying
with the active dataset bindings. If it cannot be enabled yet, temporarily
remove or comment only the Analytics Engine bindings for that environment. The
worker can still write to D1, but responses will report `firehose: false` and
high-volume telemetry will lack its primary time-series store.

### 4. Deploy and verify

```bash
npm test
npm run deploy
curl -fsS https://telemetry.daanio.com/v1/health
```

Cloudflare manages DNS records and certificates for the custom-domain routes.
New domains may take a short time to become reachable after the first deploy.

## Rollback

### Worker-code rollback

Use the Cloudflare Workers dashboard to select the previous known-good worker
deployment and roll it back. This is the safest incident path because it does
not require rebuilding from a possibly dirty local checkout.

After rollback:

```bash
curl -fsS https://telemetry.daanio.com/v1/health
npx wrangler tail daanio-telemetry
```

### Database rollback

D1 migrations are not reverted automatically when worker code is rolled back.
During an incident, prefer a forward-compatible worker fix or a new corrective
migration. Do not drop columns, tables, or production data as part of an
emergency rollback unless a separately reviewed recovery plan and backup exist.

When a deployment adds schema, keep the previous worker compatible with both
the old and expanded schema whenever practical. This makes worker rollback
independent from database rollback.

## Common failures

### `GET /v1/event` returns `405`

This is correct. `/v1/event` accepts `POST` only. Use `/v1/health` for browser
or monitoring checks.

### Deployment fails with Analytics Engine error `10089`

Enable Analytics Engine for the Cloudflare account, confirm the dataset
bindings, and redeploy. Do not rename production datasets to work around the
error.

### Worker reports missing table or column

The worker was deployed before its required D1 migration. Identify and apply
the missing numbered migration with `--remote`, then verify health and logs.

### Health returns `over_soft_limit: true`

The worker automatically performs retention pruning, but D1 deletes do not
shrink the physical file. Inspect the health query and retention logs. If file
bloat prevents recovery, plan a database rotation: create a new D1 database,
copy required durable rows, update the binding, deploy, and verify before
retiring the old database.

### Custom domain is unavailable

Check the worker's Custom Domains page in Cloudflare, verify that the domain is
active, and allow certificate provisioning to complete. Do not manually place
a conflicting proxied DNS record in front of a Workers custom domain.

### POST requests return `400`

The JSON is malformed, is missing required `id`, `event`, `version`, `os`, or
`arch` fields, or uses an unknown event type. Inspect the client payload against
the accepted event schema without logging identifiers or user-submitted
feedback text.

## Deployment checklist

- [ ] Intended Git commit selected and worktree reviewed
- [ ] `npm ci` completed
- [ ] `npm test` passed
- [ ] Required remote D1 migrations applied in order
- [ ] `npm run deploy` completed and deployment ID recorded
- [ ] `/v1/health` returns `ok: true`
- [ ] Database is below its soft limit
- [ ] `GET /v1/event` returns the expected `405`
- [ ] Production logs show no repeated storage failures
- [ ] Dashboard metrics remain plausible after deployment
- [ ] Previous worker deployment remains available for rollback
