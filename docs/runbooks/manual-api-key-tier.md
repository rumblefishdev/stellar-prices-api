# Runbook: issuing a manual higher-tier API key

**When:** someone needs more than the self-service limits and has agreed terms with
us out of band. There is no self-serve upgrade path and no in-app billing — by
design (task 0157, epic _Self-Service Onboarding_).

**Who:** anyone with `AdministratorAccess` on the shared AWS account
(`750702271865`, `eu-central-1`).

---

## What the default tier is

Every key issued through the portal lands on the CDK-managed plan:

|       |                               |
| ----- | ----------------------------- |
| Plan  | `pricing-api-free-production` |
| Rate  | 1 req/s sustained             |
| Burst | 5                             |
| Quota | 100 000 requests/month        |

That plan is defined in `infra/src/lib/stacks/api-gateway-stack.ts` and its values
come from `infra/envs/production.json`. **Do not hand-edit it in the console** —
the next deploy reverts you, and the change is invisible in review.

## Why a manual tier is a separate plan

A key belongs to exactly one usage plan per stage. Raising limits for one holder
therefore means a second plan, not a second set of numbers on the existing one.

The manual plan is deliberately **not** in CDK. The epic settles this as
"a fully manual, out-of-band process for now" with "nothing to build here".
Putting it in CDK would mean carrying a resource with no holders and inventing
config fields for numbers negotiated case by case. Revisit once there is more
than one such customer.

The trade-off is real and worth stating: a hand-made plan is **drift**. It does
not appear in `cdk diff`, nobody reviews it, and it survives only as long as
someone remembers it exists. That is why step 5 below is not optional.

---

## Issue the key

Set the negotiated limits first — these are examples, not defaults:

```bash
export AWS_PROFILE=<shared-account-profile>
export AWS_REGION=eu-central-1
export CUSTOMER=acme            # lowercase, no spaces
export RATE=20                  # req/s sustained
export BURST=40                 # token bucket capacity, conventionally 2x rate
export QUOTA=5000000            # requests per month
```

**1. Create the plan and attach the production stage.**

```bash
API_ID=$(aws ssm get-parameter --name /prices/production/api-gateway-id \
  --query 'Parameter.Value' --output text)

PLAN_ID=$(aws apigateway create-usage-plan \
  --name "prices-production-${CUSTOMER}-plan" \
  --description "Manual tier for ${CUSTOMER}; terms agreed out of band" \
  --api-stages "apiId=${API_ID},stage=production" \
  --throttle "rateLimit=${RATE},burstLimit=${BURST}" \
  --quota "limit=${QUOTA},period=MONTH,offset=0" \
  --tags "Project=stellar-prices-api,Environment=production,ManagedBy=manual,Customer=${CUSTOMER}" \
  --query 'id' --output text)

echo "plan: ${PLAN_ID}"
```

`ManagedBy=manual` is what tells the next person this resource is not CDK's.

**2. Create the key.** Note that `create-api-key` returns the secret `value` in
its response. `--query 'id'` filters client-side, so it never reaches your
terminal — but it did cross the wire and it lands in CLI debug output if you ever
rerun this with `--debug`. Do not.

```bash
KEY_ID=$(aws apigateway create-api-key \
  --name "prices-production-${CUSTOMER}-key" \
  --description "Manual tier for ${CUSTOMER}" \
  --enabled \
  --tags "Project=stellar-prices-api,ManagedBy=manual,Customer=${CUSTOMER}" \
  --query 'id' --output text)

aws apigateway create-usage-plan-key \
  --usage-plan-id "${PLAN_ID}" \
  --key-id "${KEY_ID}" \
  --key-type API_KEY
```

**3. Check the stage ceiling still holds.**

The stage's default method throttle applies to every caller of a given method —
it is a per-method limit, not one pool shared across the stage — and the more
restrictive of it and the plan wins. Read the current value from
`infra/envs/production.json` (`apiGatewayThrottleRate` / `apiGatewayThrottleBurst`)
rather than trusting a number written here.

A manual tier above that ceiling cannot be delivered by creating a plan alone —
`apiGatewayThrottleRate` has to go up too, which is a CDK change and a capacity
decision, not a runbook step.

**4. Read the key value out and hand it over.**

```bash
aws apigateway get-api-key --api-key "${KEY_ID}" --include-value \
  --query 'value' --output text
```

Send it through something that does not retain plaintext indefinitely. Do not
paste it into a ticket, a shared doc, or a chat channel with history — treat it
the way you would a password. If it does leak, rotation is step 7.

`--include-value` is the singular form's flag and it is opt-in; the plural
`get-api-keys` takes `--include-values` and defaults to off. Use the plural
without the flag whenever you only need to find a key, not read it.

**5. Write it down.** Add a row to the table at the bottom of this file and commit.
This file is the only record that the resource exists.

**6. Payment** is a normal bank transfer arranged outside the product. Nothing in
the system tracks it, and nothing enforces it — if a customer stops paying, the
key has to be disabled by hand ("Suspend without destroying", below).

---

## Change or revoke a manual key

Everything below runs weeks or months after the key was issued, in a shell that
has none of step 1's variables. `PLAN_ID` and `KEY_ID` come from the registry
table at the bottom of this file — that is what it is for. If the row is missing
or stale, recover them by name:

```bash
export CUSTOMER=acme
PLAN_ID=$(aws apigateway get-usage-plans \
  --query "items[?name=='prices-production-${CUSTOMER}-plan'].id | [0]" --output text)
KEY_ID=$(aws apigateway get-api-keys --name-query "prices-production-${CUSTOMER}-key" \
  --query 'items[0].id' --output text)
```

`get-api-keys` without `--include-values` does not return the secret. Then fix
the registry row.

**Adjust limits** — safe, no key rotation:

```bash
aws apigateway update-usage-plan --usage-plan-id "${PLAN_ID}" --patch-operations \
  op=replace,path=/throttle/rateLimit,value=30 \
  op=replace,path=/quota/limit,value=8000000
```

**Suspend without destroying** (non-payment, suspected leak, investigation):

```bash
aws apigateway update-api-key --api-key "${KEY_ID}" --patch-operations \
  op=replace,path=/enabled,value=false
```

Whether disabling preserves, freezes or zeroes the usage counters is
**not documented by AWS** and is tracked as task 0171 #8. Do not assume the
quota is preserved across a disable/enable cycle.

**7. Rotate** — create a new key (step 2), attach it, confirm the customer is
using it, then delete the old one. Capture the outgoing key's id **before** you
create the replacement, or you will be picking it out of a list by name:

```bash
OLD_KEY_ID=$(aws apigateway get-api-keys \
  --name-query "prices-production-${CUSTOMER}-key" \
  --query 'items[0].id' --output text)

# ... run step 2 to create the new key, hand it over, confirm it is in use ...

aws apigateway delete-api-key --api-key "${OLD_KEY_ID}"
```

`get-api-keys` without `--include-values` is the safe way to look a key up: the
plural form defaults to omitting the secret. Do **not** reach for
`get-usage-plan-keys` — it returns the plaintext `value` and has no flag to
suppress it, so listing keys that way puts live secrets in your shell history.

**Quota does not carry across rotation.** A new key starts its counter at zero,
so a customer rotated mid-month gets a fresh full month's allowance. During the
overlap — old key alive, new key attached — they effectively hold two quotas,
because quota is tracked per `(usage plan, API key)`. Keep the overlap short.

**Wind down** — detach the stage first, then delete the key and the plan:

```bash
API_ID=$(aws ssm get-parameter --name /prices/production/api-gateway-id \
  --query 'Parameter.Value' --output text)

aws apigateway update-usage-plan --usage-plan-id "${PLAN_ID}" \
  --patch-operations op=remove,path=/apiStages,value="${API_ID}:production"

aws apigateway delete-api-key --api-key "${KEY_ID}"
aws apigateway delete-usage-plan --usage-plan-id "${PLAN_ID}"
```

The detach is not optional. A plan that still has an API stage attached cannot be
deleted — API Gateway returns `BadRequestException: Cannot delete Usage Plan <id>
because there are API Stages associated with it`. Step 1 always attaches a stage,
so every plan created by this runbook hits it. The `value` format is
`apiId:stageName`, colon-separated.

This is **not** in the `DeleteUsagePlan` API docs, which list only generic errors —
which is why it is easy to write the obvious two-line teardown and have it fail.

Then delete the row from the table below.

---

## Things that will bite you

- **Never attach a manual key to `pricing-api-free-production`.** It would
  silently inherit 1 req/s and a 100 000/month quota. A key belongs to exactly
  one usage plan **per stage** — so on this API there is no "also attach it to
  the bigger plan". (A key may sit in up to 10 plans overall, across different
  stages; that does not help here.)
- **Quotas are best-effort.** AWS: _"Usage plan throttling and quotas are not hard
  limits… Don't rely on usage plan quotas or throttling to control costs."_ For a
  tier large enough to matter financially, back it with AWS Budgets.
- **A cached response still costs a request.** This half is firm: API Gateway
  charges per call received, cache hit or not — the cache is billed separately by
  the hour and nowhere described as reducing call charges. Whether the _quota_ is
  also decremented before the cache lookup is our inference, not documented; AWS's
  throttling order lists usage plan, stage, account and Regional limits and never
  mentions the cache. See task 0171.
- **The monthly reset schedule is all but undocumented.** The only statement in
  AWS's docs is an example caption — _"creates a usage plan that resets at the
  beginning of the month"_ — with no timezone, no instant, and nothing on whether
  `MONTH` is calendar-aligned or runs from plan creation. Note also that
  `QuotaSettings.offset` is _"the number of requests subtracted from the given
  limit in the initial time period"_ — a request count, not a way to shift the
  reset day, so it cannot be used to force alignment. Unverified; task 0171 #7.
  Do not promise a customer a specific reset date until it is measured.

---

## Issued manual keys

Keep this current. One row per plan; delete the row when the plan is deleted.

| Customer     | Plan name | Plan ID | Key ID | Limits | Issued | Issued by |
| ------------ | --------- | ------- | ------ | ------ | ------ | --------- |
| _(none yet)_ |           |         |        |        |        |           |
