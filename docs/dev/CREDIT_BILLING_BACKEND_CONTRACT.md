# Daanio Credit Billing Backend Contract

## Purpose

Daanio billing is credit-based. The backend is the authority for credit
balances, purchasable credit packages, model availability, model credit rates,
and the final amount charged for every request.

The CLI must not contain subscription tiers, plan prices, included monthly
budgets, or tier-to-model authorization rules. Authentication identifies an
account; it does not grant a locally interpreted subscription entitlement.

This contract replaces the subscription-oriented account contract while
providing a safe transition for already-installed clients.

## Design rules

1. Never use floating-point numbers for money or ledger balances.
2. The backend decides whether an account can use a model.
3. The backend performs every credit reservation, settlement, and refund.
4. A retried request must never be charged twice.
5. Public package prices and model rates come from live backend data.
6. The amount recorded in the ledger is authoritative; displayed model rates
   are estimates until the request is settled.
7. Authentication tokens and API keys must not embed a balance, plan, tier, or
   model catalog that can become stale.

## Credit precision

Use integer microcredits on the wire and in the ledger:

```text
1 credit = 1,000,000 microcredits
```

Examples:

```text
12 credits       = 12,000,000 microcredits
0.125 credits    =    125,000 microcredits
```

Field names containing `_micros` always contain a non-negative JSON integer.
Do not return credit quantities as JSON floating-point values.

Money uses integer minor currency units. For USD, `amount_minor: 1000` means
`$10.00`.

## Account endpoint

### `GET /v1/me`

Requires the Daanio account bearer key.

Response:

```json
{
  "schema_version": 1,
  "account_id": "acct_123",
  "email": "developer@example.com",
  "status": "active",
  "credits": {
    "balance_micros": 740000000,
    "reserved_micros": 12000000,
    "available_micros": 728000000,
    "lifetime_purchased_micros": 1000000000,
    "lifetime_used_micros": 260000000
  },
  "manage_url": "https://daanio.com/account"
}
```

Requirements:

- `balance_micros` is the settled balance.
- `reserved_micros` is the amount held for in-progress requests.
- `available_micros` is the amount that can be reserved immediately and must
  equal `max(balance_micros - reserved_micros, 0)`.
- Lifetime totals are optional accounting fields, but when present they must
  be monotonically increasing.
- `status` describes account usability, not a subscription state. Recommended
  values are `active`, `disabled`, and `closed`.
- `manage_url` must be a stable public HTTPS URL and must never contain a
  bearer key, device code, or other secret.

The response must not require or return `plan_id`, `tier`, monthly allowance,
renewal date, or subscription price.

## Purchasable credit packages

### `GET /v1/credits/packages`

This endpoint may be public. If regional, account-specific, or promotional
pricing is supported, authenticate it and vary the cache by account and
currency.

Response:

```json
{
  "schema_version": 1,
  "currency": "USD",
  "updated_at": "2026-07-26T10:00:00Z",
  "packages": [
    {
      "id": "credits_1000_usd",
      "display_name": "1,000 credits",
      "credits_micros": 1000000000,
      "price": {
        "amount_minor": 1000,
        "currency": "USD"
      },
      "active": true,
      "purchasable": true
    }
  ]
}
```

The package `id` identifies a checkout product. It is not a plan and grants no
continuing entitlement. A successful purchase creates one immutable positive
ledger entry.

Recommended response headers:

```http
ETag: "credit-packages-2026-07-26-1"
Cache-Control: public, max-age=300, stale-if-error=86400
```

Support `If-None-Match` and return `304 Not Modified` when appropriate. Change
the ETag whenever a price, credit amount, availability flag, or display name
changes.

## Model catalog and live credit rates

### `GET /v1/models`

Requires the Daanio account bearer key. The returned list is authoritative for
that account at request time.

Extend each model entry with an optional credit-rate object:

```json
{
  "object": "list",
  "data": [
    {
      "id": "gpt-5.6-sol",
      "object": "model",
      "pricing": {
        "unit": "microcredits_per_million_tokens",
        "input_micros": 1000000,
        "output_micros": 5000000,
        "cache_read_micros": 250000,
        "cache_write_micros": 1250000,
        "minimum_charge_micros": 1000
      }
    }
  ]
}
```

Requirements:

- Only return models the authenticated account may currently request.
- A model absent from the catalog may still produce a normal `model_not_found`
  or `model_not_available` error if requested directly.
- Pricing fields may be omitted when a model cannot be estimated in advance.
- Catalog pricing is informational. The settled ledger transaction is the
  source of truth.
- Use an ETag scoped to the authenticated account's catalog generation.
- Do not require the CLI to translate models into tiers or upstream providers.

## Request charging lifecycle

Every billable inference request follows this lifecycle:

1. Authenticate the account.
2. Resolve the requested model using the live server catalog.
3. Create or reuse the request's idempotency record.
4. Reserve the maximum or safely estimated credit amount atomically.
5. Execute the upstream model request.
6. Calculate the final charge from authoritative upstream usage.
7. Settle the final ledger debit and release unused reservation in one
   transaction.
8. If the upstream request fails before billable work occurs, release the
   reservation without creating a debit.
9. Return the charge, remaining balance, and ledger transaction identifier.

The ledger should be append-only. Corrections are new adjustment or refund
entries; existing entries are never rewritten or deleted.

Minimum ledger fields:

```json
{
  "transaction_id": "ctxn_01J...",
  "account_id": "acct_123",
  "request_id": "req_01J...",
  "kind": "inference_debit",
  "amount_micros": 12000000,
  "model": "gpt-5.6-sol",
  "created_at": "2026-07-26T10:01:12Z"
}
```

Enforce a unique constraint on `(account_id, request_id, kind)` for the final
inference debit. Replaying the same request ID returns the existing result or
transaction and never creates another debit.

## Client request identity

Accept a stable client-generated idempotency key:

```http
Idempotency-Key: 7c75e88c-8844-4dbe-a080-4fe7de6d6f47
```

If absent, the gateway may generate a request ID, but automatic client retries
are only safely deduplicated when they reuse the same idempotency key.

Idempotency records should outlive the maximum client retry window. A minimum
retention of 24 hours is recommended.

## Usage returned by inference endpoints

For non-streaming responses, add a namespaced extension without changing the
provider-compatible token usage fields:

```json
{
  "id": "resp_123",
  "usage": {
    "input_tokens": 1200,
    "output_tokens": 340
  },
  "daanio_credits": {
    "charged_micros": 12000000,
    "balance_after_micros": 728000000,
    "transaction_id": "ctxn_01J..."
  }
}
```

Also return headers so generic clients can observe the charge:

```http
X-Daanio-Credits-Charged-Micros: 12000000
X-Daanio-Credits-Balance-Micros: 728000000
X-Daanio-Credit-Transaction-Id: ctxn_01J...
X-Request-Id: req_01J...
```

For streaming responses, include `daanio_credits` in the final server-sent
event before `[DONE]`. The final charge must also be queryable from the credit
ledger using the request ID in case the connection closes before the final
event arrives.

## Credit ledger endpoint

### `GET /v1/credits/transactions`

Requires authentication and returns only the caller's transactions.

Example:

```http
GET /v1/credits/transactions?limit=50&after=ctxn_01J...
```

```json
{
  "object": "list",
  "data": [
    {
      "id": "ctxn_01J...",
      "kind": "inference_debit",
      "amount_micros": 12000000,
      "model": "gpt-5.6-sol",
      "request_id": "req_01J...",
      "created_at": "2026-07-26T10:01:12Z"
    }
  ],
  "next_after": null
}
```

Use cursor pagination. Do not expose upstream provider credentials, internal
provider costs, payment processor secrets, or another account's data.

## Errors

Use a stable machine-readable error envelope:

```json
{
  "error": {
    "code": "insufficient_credits",
    "message": "Insufficient credits for this request.",
    "required_micros": 12000000,
    "available_micros": 5000000,
    "purchase_url": "https://daanio.com/credits"
  }
}
```

Recommended status codes:

| Status | Code | Meaning |
|---|---|---|
| `401` | `invalid_key` | Account key is missing, expired, or revoked. |
| `403` | `account_disabled` | The account exists but cannot make requests. |
| `402` | `insufficient_credits` | Available balance cannot cover the reservation. |
| `404` | `model_not_found` | The requested model is unknown. |
| `409` | `idempotency_conflict` | The same key was reused with a different request. |
| `429` | `rate_limited` | Request rate limit exceeded; balance is unaffected. |
| `503` | `model_unavailable` | The model is temporarily unavailable; release reservation. |

Never describe credit failures as subscription, plan, tier, or upgrade
failures.

## Purchase settlement

Credit purchases must be granted from verified payment-provider webhooks, not
from browser redirects.

Requirements:

- Verify webhook signatures and timestamp tolerance.
- Use the payment event ID as an idempotency key.
- Persist the payment and positive credit ledger entry atomically.
- Replayed or reordered webhooks must not grant credits twice.
- Refunds and chargebacks create negative adjustment entries.
- Never trust a client-supplied price or credit amount. Checkout accepts only a
  backend-defined active package ID.

## Authentication contract

The device authorization flow continues to issue an account API key. Approved
token responses should become:

```json
{
  "api_key": "secret-returned-once",
  "account_id": "acct_123",
  "email": "developer@example.com",
  "status": "active"
}
```

Do not include a tier, plan, balance, or model list in the key or approval
response. The CLI obtains current state from `/v1/me` and `/v1/models`.

## Migration from subscription fields

Already-released clients expect `tier` and `usage` in `/v1/me` and may expect
`tier` in the approved device-token response. Removing those fields
immediately would break those clients before they can upgrade.

Use a two-stage migration:

### Compatibility stage

- Add `credits` and the credit endpoints.
- Keep legacy `tier`, `usage`, and device-token `tier` fields only for released
  clients.
- Return a neutral legacy tier value accepted by the released client; do not
  use it for gateway authorization.
- Stop returning tier-specific model authorization errors. The gateway checks
  live credits and model availability instead.
- Release a CLI version that reads `credits` and ignores legacy subscription
  fields.

### Removal stage

- Measure active client versions at the gateway without logging secrets.
- After the supported upgrade window, remove legacy `tier`, subscription
  `usage`, plan-price, and monthly-reset fields.
- Keep server errors explicit enough that old clients instruct users to update.

The CLI's legacy route name `daanio-subscription` is an internal compatibility
alias. The backend must not depend on it. New client configuration should use a
neutral route name such as `daanio` or `daanio-credits` while accepting the old
name during migration.

## Required backend tests

### Account and catalog

- `/v1/me` returns exact settled, reserved, and available integer balances.
- `/v1/models` changes when backend model availability changes.
- No tier or cached client claim can unlock a model.
- Package ETags change whenever package data changes.
- Unknown JSON request fields do not affect prices or granted credits.

### Ledger correctness

- Concurrent requests cannot spend the same available credits.
- A retry with the same idempotency key creates exactly one debit.
- Reusing an idempotency key with a different payload returns `409`.
- Failed, canceled, and timed-out upstream calls release reservations.
- Settlement releases unused reservation atomically.
- Balance never becomes negative unless an explicit administrative policy
  permits debt.
- Refund and chargeback events remain auditable.

### Streaming

- The final event contains the same charge as the ledger.
- Disconnect before the final event still settles actual billable usage.
- The client can recover the charge using the request ID.

### Payment processing

- Duplicate webhooks grant credits once.
- Invalid signatures grant no credits.
- Browser success redirects grant no credits by themselves.
- Client-supplied package prices and credit quantities are ignored.

### Security and privacy

- Accounts can read only their own balance and transactions.
- Logs never contain API keys, payment secrets, or device codes.
- URLs never contain bearer credentials.
- Administrative balance adjustments record actor, reason, timestamp, and a
  unique audit identifier.

## Backend completion criteria

The credit backend is ready for CLI integration when:

1. `/v1/me` exposes authoritative credit balances.
2. `/v1/credits/packages` exposes live purchase prices.
3. `/v1/models` exposes the authenticated live catalog and optional rates.
4. Inference responses expose the settled charge and remaining balance.
5. Atomic reservation and idempotent settlement tests pass under concurrency.
6. The compatibility response keeps released clients functional during the
   migration window.
7. No authorization path depends on subscription tiers or plan IDs.
