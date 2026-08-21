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
KEY_NAME="prices-production-${CUSTOMER}-key-$(date -u +%Y%m%dT%H%M%SZ)"

KEY_ID=$(aws apigateway create-api-key \
  --name "${KEY_NAME}" \
  --description "Manual tier for ${CUSTOMER}" \
  --enabled \
  --tags "Project=stellar-prices-api,ManagedBy=manual,Customer=${CUSTOMER}" \
  --query 'id' --output text)

aws apigateway create-usage-plan-key \
  --usage-plan-id "${PLAN_ID}" \
  --key-id "${KEY_ID}" \
  --key-type API_KEY

echo "key name: ${KEY_NAME}"
echo "key id:   ${KEY_ID}"
```

**The timestamp in the name is load-bearing, not decoration.** Rotation (step 7)
runs step 2 again while the old key is still alive, and AWS does not enforce unique
key names — so without it the customer would briefly hold two keys called the same
thing, and every later lookup by name would have two right answers. With it, each
key names the instant it was issued and stays distinguishable for the rest of its
life. Record the full name in the registry table, not just the id.

It is to the second (`20260812T142317Z`), not to the day, and that is deliberate.
The likeliest rotation is not a scheduled one — it is a leak, rotated the same
hour, and rotated again an hour later because the first replacement went to the
wrong inbox. A day-granular suffix collides in exactly that case, which is the one
the suffix exists for.

The prefix `prices-production-${CUSTOMER}-` is also what keeps hand-made keys out
of the way of the self-service ones: task 0160 issues those as
`discord-<userId>-key` and looks them up by exact name. **Never give a manual key
a `discord-` name**, and never give a self-service user a customer slug that
collides with one here.

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
export AWS_PROFILE=<shared-account-profile>
export AWS_REGION=eu-central-1
export CUSTOMER=acme

PLAN_ID=$(aws apigateway get-usage-plans \
  --query "items[?name=='prices-production-${CUSTOMER}-plan'].id | [0]" --output text)

# `--output text` prints the literal string "None" for an empty result, not an
# empty string — so `[ -z "$PLAN_ID" ]` does NOT catch it, and an unguarded
# "None" flows into `update-usage-plan --usage-plan-id None` below. Unset it so
# the failure lands on the command that needs it, not three snippets later.
[ "${PLAN_ID}" = "None" ] && {
  echo "no plan for ${CUSTOMER} — wrong AWS_PROFILE/AWS_REGION, or the plan is gone"
  unset PLAN_ID
}

# Keys: list every candidate and choose by hand. Do NOT collapse this to
# `items[0].id` — see below for why there can legitimately be more than one.
aws apigateway get-api-keys \
  --query "items[?starts_with(name, 'prices-production-${CUSTOMER}-key')].[id,name,createdDate]" \
  --output table

export KEY_ID=<the id from the row you want>
```

`get-api-keys` without `--include-values` does not return the secret. Then fix
the registry row.

The two `export`s at the top are not ceremony: this section runs in a fresh
shell, and against a different default profile every lookup here quietly targets
the wrong account. Both failures then say the same thing in different ways — the
plan resolves to `None`, and the key table comes back empty — which is why the
guard names the profile rather than the plan. Nothing mutates in the wrong
account either way (`--usage-plan-id None` is rejected), but the error you get
from AWS points nowhere near the actual cause.

The guard warns and unsets rather than exiting, because these snippets are pasted
into an interactive shell where `exit` would close your terminal. Unsetting is the
part that matters: a warning scrolls past — the table below it draws the eye — and
`PLAN_ID` would otherwise still hold `"None"` for every snippet after this one.
Unset, the next command that needs it fails on an empty argument, at the point of
use, saying so.

Two reasons this lists rather than picks, both of which have bitten elsewhere in
this project:

- **A key name is not unique.** AWS enforces uniqueness only on key _values_;
  `name` is optional and duplicable. During a rotation (step 7) the customer
  deliberately holds two keys at once, so a lookup by name has two right answers
  and no way to rank them. Taking the first would suspend or delete whichever one
  the API happened to list first — including, in the worst case, disabling the
  _new_ key while the old one keeps serving.
- **`--name-query` has no documented matching semantics**, which is why it does
  not appear above at all. AWS's entire description of it is _"The name of queried
  API keys."_ — not documented as exact, as a prefix, or as anything else (task
  0156, 2026-08-10). **Measured 2026-08-12 (task 0180): it is a case-sensitive
  prefix match** — a query for a key's own full name also returns any longer key
  that extends it, which is precisely the rotation state step 7 creates. So the
  parameter cannot decide which key you are looking at, whatever its
  documentation had said. `starts_with` is JMESPath, evaluated client-side, and
  does exactly what it says. Task 0160 reaches the same conclusion for the
  automated issuance path and comments it so nobody deletes it as redundant; the
  same applies here. If you re-add `--name-query` as a server-side prefilter it is
  safe to do so — prefix matching can only ever return a superset of what
  `starts_with` keeps — but keep the client-side filter: it is the only part that
  decides.

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
**not documented by AWS**. **Measured 2026-08-12 (task 0180): the counters are
preserved.** A key drained to its quota and then disabled is still at its quota
when re-enabled — the first request after re-enabling was rejected, not served.
Suspension is therefore not a way to give a customer a fresh allowance, and not a
way for one to take it.

Two operational consequences from the same measurement:

- **A disabled key returns `403 Forbidden`, identical to sending no key at all.**
  Expect the customer to report it as "my key stopped working" with no hint that
  it was suspended rather than deleted. Tell them which it was; the gateway will
  not.
- **Suspension is not immediate — allow tens of seconds.** Disable took ~25 s to
  reach the data plane, re-enable the same. If you are suspending a leaked key,
  the leak is still live for that window; if the situation cannot tolerate it,
  delete the key instead of disabling it.

**7. Rotate** — create a new key (step 2), attach it, confirm the customer is
using it, then delete the old one. Capture the outgoing key's id **before** you
create the replacement: once both exist, a lookup by name has two answers.

```bash
# From the registry row. If it is missing or stale, list candidates with the
# snippet above and choose the older one by createdDate — never items[0].
OLD_KEY_ID=<id of the key being replaced>

# ... run step 2 to create the new key; it carries the issuing instant, so the
#     two never share a name — even rotating twice within the hour. Hand it
#     over, confirm it is in use ...

aws apigateway delete-api-key --api-key "${OLD_KEY_ID}"
```

Update the registry row in the same sitting as step 2, not after the customer
confirms. The window in which two keys exist is exactly the window in which
somebody might have to suspend one of them in a hurry — a leak, a missed payment
— and a registry that still names only the old key is worse than useless then,
because it looks authoritative.

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
- **Nothing stops you creating two keys with the same name.** Only key _values_
  are unique in API Gateway; `name` is optional and duplicable, and there is no
  documented way to ask "which of these is the current one". That is why step 2
  timestamps the name, why the registry carries the name as well as the id, and
  why every lookup in this file lists candidates instead of taking the first. If
  you find yourself typing `items[0]`, stop.
- **Quotas are best-effort.** AWS: _"Usage plan throttling and quotas are not hard
  limits… Don't rely on usage plan quotas or throttling to control costs."_ For a
  tier large enough to matter financially, back it with AWS Budgets.
- **A cached response still costs a request.** This half is firm: API Gateway
  charges per call received, cache hit or not — the cache is billed separately by
  the hour and nowhere described as reducing call charges. Whether the _quota_ is
  also decremented before the cache lookup is our inference, not documented; AWS's
  throttling order lists usage plan, stage, account and Regional limits and never
  mentions the cache. See task 0180.
- **The monthly reset schedule is all but undocumented.** The only statement in
  AWS's docs is an example caption — _"creates a usage plan that resets at the
  beginning of the month"_ — with no timezone, no instant, and nothing on whether
  `MONTH` is calendar-aligned or runs from plan creation. Note also that
  `QuotaSettings.offset` is _"the number of requests subtracted from the given
  limit in the initial time period"_ — a request count, not a way to shift the
  reset day, so it cannot be used to force alignment. Task 0180 #7, carried to
  task 0191, which measured the `DAY`-period proxy (result and date in that
  task's Step 0 table) — a `MONTH` rollover itself cannot be observed before
  1 September 2026. The portal states "the 1st, 00:00 UTC" as **our** period
  rule for its own quota cap and dashboard; do not promise a customer that AWS's
  counter resets at that instant until the `MONTH` observation exists.

---

## Issued manual keys

Keep this current. One row per **key**, so a customer mid-rotation has two;
delete a row when its key is deleted, and the last one when the plan goes.

| Customer     | Plan name | Plan ID | Key name | Key ID | Limits | Issued | Issued by |
| ------------ | --------- | ------- | -------- | ------ | ------ | ------ | --------- |
| _(none yet)_ |           |         |          |        |        |        |           |

**Key name** is a column because key names are not unique and now carry the issuing
instant — during a rotation two rows may share a customer, and the name is what
tells them apart. Keep both rows until the old key is deleted.
