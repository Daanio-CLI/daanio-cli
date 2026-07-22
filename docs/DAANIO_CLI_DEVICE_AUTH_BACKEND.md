# Daanio CLI browser login: backend implementation guide

## Goal

Allow a user to run:

```bash
daanio login daanio
```

The CLI opens `daanio.com`, the user signs in and approves the CLI, and the CLI
receives a Daanio gateway credential automatically. The user must never copy an
API key into the terminal and must never provide an OpenAI, Anthropic, Google,
OpenRouter, or other upstream-provider key.

This should use an OAuth 2.0 Device Authorization Grant-style flow (RFC 8628).
It is a better fit than a localhost callback because the CLI must work over SSH,
inside containers, and on machines where a browser cannot redirect to the CLI.

The first implementation should use the endpoint and response contract already
implemented by the Daanio CLI:

- `POST https://api.daanio.com/v1/auth/device`
- `POST https://api.daanio.com/v1/auth/token`
- `GET https://api.daanio.com/v1/me`
- `DELETE https://api.daanio.com/v1/keys/current`

The credential returned by this flow is a Daanio credential only. Daanio's
backend remains responsible for selecting and authenticating upstream model
providers.

## User experience

```text
CLI                     Daanio API                 daanio.com
 |                          |                          |
 | POST /v1/auth/device     |                          |
 |------------------------->| create pending flow      |
 |<-------------------------| browser approval URL     |
 |                                                     |
 | open approval URL --------------------------------->|
 |                          |       user signs in       |
 |                          |<------ approve flow ------|
 |                                                     |
 | POST /v1/auth/token      |                          |
 |------------------------->| atomically exchange flow |
 |<-------------------------| Daanio credential         |
 |                                                     |
 | GET /v1/me, Bearer token |                          |
 |------------------------->|                          |
 |<-------------------------| account/plan status       |
```

If a browser cannot be opened, the CLI prints the same public approval URL so
the user can open it on another device.

## Required API contract

All responses containing credentials must send:

```http
Cache-Control: no-store
Pragma: no-cache
Content-Type: application/json
```

### 1. Start device authorization

```http
POST /v1/auth/device
Content-Type: application/json
```

Request:

```json
{
  "client_name": "daanio-cli",
  "requested_tier": "pro"
}
```

`requested_tier` is optional. Login must still be allowed when the account has
no paid plan. Plan enforcement should happen in the gateway or checkout flow,
not by exposing upstream credentials.

Successful response (`200` or `201`):

```json
{
  "device_code": "secret-high-entropy-one-time-value",
  "flow_id": "public-random-flow-reference",
  "verification_uri": "https://daanio.com/device",
  "verification_uri_complete": "https://daanio.com/device?flow=public-random-flow-reference",
  "expires_in": 600,
  "interval": 5
}
```

Requirements:

- `device_code` is a secret known only to the CLI and API. It must never appear
  in the browser URL, application logs, analytics, or error monitoring.
- `flow_id` is a separate random public lookup value used by the browser page.
- Generate at least 256 bits of cryptographically secure randomness for
  `device_code`.
- Store only a hash of `device_code`, never its plaintext value.
- Expire an unapproved flow after 10 minutes by default.
- Accept only known `client_name` values. Initially, allow `daanio-cli`.
- Rate-limit creation by IP and by authenticated account where applicable.

The current CLI accepts `expires_in` from 1 to 3600 seconds and `interval` from
1 to 60 seconds. Recommended values are 600 and 5.

### 2. Browser approval page

```http
GET https://daanio.com/device?flow=<flow_id>
```

The website must:

1. Require the user to sign in to their Daanio account.
2. Preserve the `flow_id` through the sign-in redirect using server-side state
   or a signed, short-lived continuation value.
3. Show that "Daanio CLI" is requesting access.
4. Show Approve and Deny actions.
5. Protect the actions with normal session authentication and CSRF protection.
6. Never display a generated credential or any upstream-provider credential.
7. Clearly show expired, already-used, denied, and invalid-flow states.

Approval should update the pending authorization record with the authenticated
Daanio account ID. It should not return the gateway credential to browser-side
JavaScript.

Suggested approval request:

```http
POST /device/approve
Content-Type: application/x-www-form-urlencoded
Cookie: authenticated Daanio session

flow=<flow_id>&csrf_token=<token>
```

The denial endpoint can use the same shape at `/device/deny`.

### 3. Poll and exchange the device code

```http
POST /v1/auth/token
Content-Type: application/json
```

Request:

```json
{
  "device_code": "secret-high-entropy-one-time-value"
}
```

Pending response (`202 Accepted` or `428 Precondition Required`):

```json
{
  "error": "authorization_pending"
}
```

Polling too quickly (`429 Too Many Requests`):

```http
Retry-After: 10
```

```json
{
  "error": "slow_down"
}
```

Approved response (`200 OK`):

```json
{
  "api_key": "daanio_live_secret_returned_only_once",
  "account_id": "acct_123",
  "email": "user@example.com",
  "tier": "pro",
  "status": "active"
}
```

The field is named `api_key` for compatibility with the existing CLI, but the
user does not manually create, view, or paste it. It is a scoped, revocable
Daanio gateway credential issued through browser authorization.

The exchange must be atomic:

1. Lock or conditionally update the authorization record.
2. Verify it is approved, unexpired, and unused.
3. Mint one Daanio CLI credential.
4. Mark the authorization record consumed.
5. Return the plaintext credential once.

If the response is lost after consumption, the same device code must not issue
another credential. The user can safely restart login to create a new flow.

Terminal error examples:

```json
{ "error": "expired_token" }
```

```json
{ "error": "access_denied" }
```

The existing CLI recognizes these error codes:

- Pending: `authorization_pending` or `pending`
- Slow polling: `slow_down`
- Expired: `expired_token`, `expired`, or `expired_device_code`
- Denied: `access_denied` or `denied`

### 4. Account status

```http
GET /v1/me
Authorization: Bearer <Daanio credential>
```

Successful response (`200 OK`):

```json
{
  "account_id": "acct_123",
  "email": "user@example.com",
  "tier": "pro",
  "status": "active",
  "usage": {
    "used_usd": 3.25,
    "budget_usd": 40.0,
    "resets_at": "2026-08-01T00:00:00Z"
  },
  "manage_url": "https://daanio.com/account"
}
```

Supported stable tier values expected by the current CLI are:

- `none`
- `plus`
- `pro`
- `max`
- `ultra`
- `flagship`

Use `401 Unauthorized` for an invalid, expired, or revoked credential and `403
Forbidden` when the account is valid but the action is not allowed.

### 5. Revoke the current CLI credential

```http
DELETE /v1/keys/current
Authorization: Bearer <Daanio credential>
```

Return `204 No Content` or another `2xx` response after revocation. Revocation
must be idempotent from the user's perspective. The CLI clears its local copy
even if the API is temporarily unreachable.

### 6. Authenticate normal gateway requests

The credential issued by `/v1/auth/token` must work with the existing Daanio
gateway contract:

```http
POST /v1/responses
Authorization: Bearer <Daanio credential>
Content-Type: application/json
```

The backend resolves the Daanio account, verifies plan/usage limits, chooses the
upstream provider, and attaches the upstream credential server-side. Upstream
keys must never be included in a response to the CLI.

## Suggested database model

### `device_authorizations`

| Column | Purpose |
| --- | --- |
| `id` | Internal primary key |
| `flow_id` | Unique public browser lookup value |
| `device_code_hash` | Hash of the secret device code |
| `client_name` | Initially `daanio-cli` |
| `requested_tier` | Optional checkout hint |
| `account_id` | Set after authenticated approval |
| `status` | `pending`, `approved`, `denied`, `consumed`, or `expired` |
| `created_at` | Audit and expiry calculation |
| `expires_at` | Hard expiration time |
| `approved_at` | Approval audit time |
| `consumed_at` | One-time exchange time |
| `last_polled_at` | Poll throttling |
| `poll_count` | Abuse detection |

### `api_keys` or `cli_credentials`

| Column | Purpose |
| --- | --- |
| `id` | Credential identifier |
| `account_id` | Owning Daanio account |
| `secret_hash` | Hash of the bearer credential |
| `prefix` | Non-secret display prefix for account management |
| `name` | For example, `Daanio CLI on MacBook` |
| `scopes` | Minimum required gateway scopes |
| `created_at` | Audit time |
| `last_used_at` | Account security display |
| `expires_at` | Optional credential expiry |
| `revoked_at` | Revocation state |

Do not store bearer credentials or device codes in plaintext. A keyed hash or a
slow password hash is not necessary for high-entropy random tokens; use a
server-side keyed HMAC or a cryptographic hash with secure token generation and
constant-time comparison.

## Recommended scopes

Start with the minimum set required by the CLI:

- `gateway:models:read`
- `gateway:responses:create`
- `account:self:read`
- `credential:self:revoke`

Do not grant billing changes, organization administration, or access to other
credentials. Checkout and account management remain browser-session actions.

## Security requirements

- Require HTTPS everywhere; enable HSTS on the website and API.
- Generate device codes and bearer credentials with a CSPRNG.
- Hash secrets at rest and compare them in constant time.
- Make device-code exchange single-use and transactional.
- Never put `device_code`, `api_key`, bearer headers, or upstream keys in URLs.
- Redact request bodies and `Authorization` headers from logs and traces.
- Apply `Cache-Control: no-store` to authorization and token responses.
- Rate-limit device creation, polling, approval attempts, and gateway use.
- Enforce the advertised polling interval; reply with `slow_down` and
  `Retry-After` when violated.
- Require an authenticated website session and CSRF protection for approval.
- Expire pending flows promptly and periodically delete old authorization rows.
- Allow users to list and revoke CLI credentials from their Daanio account.
- Record security audit events without recording secret values.
- Do not accept arbitrary callback or redirect URLs from the CLI.
- Never return an upstream-provider credential to the browser or CLI.

## Audit events

Recommended events:

- `device_authorization.created`
- `device_authorization.approved`
- `device_authorization.denied`
- `device_authorization.expired`
- `device_authorization.exchanged`
- `cli_credential.used`
- `cli_credential.revoked`

Record account ID, credential ID, flow ID, client name, timestamp, IP risk
metadata, and user agent where appropriate. Do not record secret token values.

## Backend configuration

Production configuration should include:

```text
PUBLIC_WEB_URL=https://daanio.com
PUBLIC_API_URL=https://api.daanio.com/v1
DEVICE_AUTH_TTL_SECONDS=600
DEVICE_AUTH_POLL_INTERVAL_SECONDS=5
DAANIO_CLI_CREDENTIAL_SIGNING_OR_HASH_KEY=<secret-manager-reference>
```

Keep signing, hashing, session, and upstream-provider secrets in the production
secret manager. They must not be committed to either the website or CLI source
repository.

## Backend acceptance tests

The backend is ready when all of these pass:

1. Starting a flow returns all six required fields.
2. The browser URL contains `flow_id`, never `device_code`.
3. Polling before approval returns `authorization_pending`.
4. Polling too quickly returns `429`, `slow_down`, and `Retry-After`.
5. An unauthenticated browser is sent through Daanio sign-in and safely returns
   to the approval page.
6. Approval binds the flow to the authenticated account.
7. Denial returns `access_denied` to the CLI.
8. Expired flows return `expired_token` and cannot be approved or exchanged.
9. An approved flow returns one Daanio credential exactly once.
10. Concurrent exchanges cannot mint two credentials.
11. The credential works for `/v1/me` and Daanio gateway requests.
12. The credential cannot perform account-administration actions.
13. Revocation immediately blocks gateway use.
14. Logs and monitoring contain no device codes, bearer values, or upstream
    credentials.
15. A user without a paid plan can authenticate and receives a clear plan or
    quota response instead of a provider-key prompt.

## CLI activation status

The production backend contract is deployed, and the CLI activation is complete.
The polling and browser orchestration live in:

- `src/cli/login/daanio_device.rs`
- `crates/daanio-base/src/subscription_api.rs`

The activated integration now:

1. Sets the account API default to `https://api.daanio.com/v1` and the account
   management URL to the production `daanio.com` page.
2. Enables `run_daanio_account_login()` in `src/cli/login.rs` using the device
   flow.
3. Routes `daanio login daanio`, `daanio account login`, and TUI `/login daanio`
   through the same device flow.
4. Stores the exchanged credential in the same private Daanio credential file
   used by the gateway provider. Do not maintain separate manual-key and
   subscription-key stores.
5. Keeps the credential file owner-only (`0600` on Unix).
6. Uses "sign in with Daanio" browser-login help text.
7. Rejects manual `--api-key` entry for Daanio login; a new device flow is
   the recovery path.
8. Includes tests covering pending, approval, denial, expiry, slow-down,
   one-time exchange, `/v1/me`, gateway access, and revocation.

## Recommended rollout

1. Implement the database model and `/v1/auth/device`.
2. Implement the authenticated browser approval and denial pages.
3. Implement transactional `/v1/auth/token` exchange.
4. Implement `/v1/me` and `/v1/keys/current` against the same credential store.
5. Verify the issued credential on the Daanio gateway.
6. Run backend security and concurrency tests.
7. Enable the existing device flow in a CLI prerelease.
8. Monitor approval completion, polling errors, and revocation reliability.
9. Make browser login the default after successful staged rollout.

The key architectural boundary is unchanged: the CLI authenticates only to
Daanio, and Daanio authenticates to upstream model providers on the server.
