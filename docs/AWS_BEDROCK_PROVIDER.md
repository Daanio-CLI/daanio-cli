# AWS Bedrock provider

Daanio supports a native AWS Bedrock provider that talks directly to Bedrock Runtime with the AWS Rust SDK and `ConverseStream`.

## Configure credentials

Daanio supports two Bedrock auth styles:

- **Bedrock API key / bearer token**: easiest for local onboarding. Daanio stores the token in its config env file and sends it through the AWS SDK as `AWS_BEARER_TOKEN_BEDROCK`.
- **AWS IAM credentials**: best for normal AWS customer environments. This can be an AWS CLI/SSO profile, environment access keys, web identity, EC2/ECS metadata credentials, or another standard AWS SDK credential source.

For the guided API-key flow, run:

```bash
daanio login --provider bedrock
```

This saves `AWS_BEARER_TOKEN_BEDROCK` and `DAANIO_BEDROCK_REGION` to `~/.config/daanio/bedrock.env`.

You can also configure manually:

```bash
export AWS_BEARER_TOKEN_BEDROCK=your-bedrock-api-key
export AWS_REGION=us-east-1
```

For AWS CLI/IAM/SSO credentials:

```bash
export AWS_PROFILE=my-profile
export AWS_REGION=us-east-1
# Optional Daanio-specific overrides:
export DAANIO_BEDROCK_PROFILE=my-profile
export DAANIO_BEDROCK_REGION=us-east-1
```

If you rely on instance/container metadata credentials and have no local profile env vars, opt in explicitly:

```bash
export DAANIO_BEDROCK_ENABLE=1
export AWS_REGION=us-east-1
```

For AWS SSO profiles, run:

```bash
aws sso login --profile my-profile
```

For AWS CLI console-login profiles, Daanio can also use credentials exported by:

```bash
aws configure export-credentials --profile my-profile --format env-no-export
```

Daanio does not store these exported session credentials; it asks the AWS CLI profile provider when the Bedrock provider initializes.

## IAM permissions

The runtime path needs, at minimum:

```json
{
  "Effect": "Allow",
  "Action": [
    "bedrock:InvokeModel",
    "bedrock:InvokeModelWithResponseStream"
  ],
  "Resource": "*"
}
```

Model discovery additionally uses:

```json
{
  "Effect": "Allow",
  "Action": [
    "bedrock:ListFoundationModels",
    "bedrock:ListInferenceProfiles"
  ],
  "Resource": "*"
}
```

If you enable STS validation with `DAANIO_BEDROCK_VALIDATE_STS=1`, allow `sts:GetCallerIdentity`.

## Run Daanio with Bedrock

```bash
daanio --provider bedrock --model anthropic.claude-3-5-sonnet-20241022-v2:0
```

or:

```bash
daanio --model bedrock:anthropic.claude-3-5-sonnet-20241022-v2:0
```

Inference profile IDs/ARNs are accepted as model IDs, for example:

```bash
daanio --model bedrock:us.anthropic.claude-3-5-sonnet-20241022-v2:0
```

Recommended active profile-style choices, when your account has access, include:

```text
us.amazon.nova-2-lite-v1:0
us.amazon.nova-lite-v1:0
us.amazon.nova-micro-v1:0
us.anthropic.claude-sonnet-4-6
us.anthropic.claude-haiku-4-5-20251001-v1:0
us.deepseek.r1-v1:0
```

Prefer the region/profile ID such as `us.amazon.nova-2-lite-v1:0` when both a foundation model ID and a profile ID appear. Some Bedrock models do not support on-demand invocation and must be invoked through an inference profile.

## Model picker UX

`/model` keeps Bedrock rows compact:

- `×` means the route is not selectable. Select the row to see the full reason, such as legacy model access or missing credentials.
- `⚠` means the route is selectable but limited, most commonly no tool-use support.
- A selected inference-profile route shows which foundation model it targets.

If the model list looks stale after enabling model access, changing region, or refreshing credentials, run:

```text
/refresh-model-list
```

This forces `ListFoundationModels` and `ListInferenceProfiles`, updates cached legacy/profile metadata, and removes stale duplicate foundation rows when a usable inference profile route is available.

## Optional request parameters

```bash
export DAANIO_BEDROCK_MAX_TOKENS=4096
export DAANIO_BEDROCK_TEMPERATURE=0.2
export DAANIO_BEDROCK_TOP_P=0.9
export DAANIO_BEDROCK_STOP_SEQUENCES='</done>,STOP'
```

## Model discovery

Daanio will use a static Bedrock model list immediately. When model prefetch/catalog refresh runs, it calls `ListFoundationModels` and `ListInferenceProfiles`, then caches results in Daanio's config directory. Cached Bedrock catalogs are region-scoped; if you switch `DAANIO_BEDROCK_REGION`/`AWS_REGION`, Daanio ignores the old-region cache and refreshes for the new region.

## Live smoke test

The live test is ignored by default. Run it only with valid AWS credentials and enabled model access:

```bash
DAANIO_BEDROCK_LIVE_TEST=1 \
AWS_PROFILE=my-profile \
AWS_REGION=us-east-1 \
cargo test -p daanio --lib provider::bedrock::tests::bedrock_live_smoke_test -- --ignored
```

## Troubleshooting

- `AccessDenied`: grant Bedrock invoke/list permissions and enable model access in the AWS Console.
- `model not found` or validation errors: verify model ID/inference profile and region support.
- SSO token errors: run `aws sso login --profile <profile>`.
- API key auth: set `AWS_BEARER_TOKEN_BEDROCK` and `AWS_REGION`.
- Missing region: set `AWS_REGION` or `DAANIO_BEDROCK_REGION`.
