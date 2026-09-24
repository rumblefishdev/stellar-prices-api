# Runbook: API key tiers — the five plans, upgrading a user, Custom plans

**When:** a user's key needs limits other than the free plan's — they have
agreed a paid plan, or negotiated custom limits, with us out of band. There is
no self-serve upgrade path and no in-app billing — by design (task 0157, epic
_Self-Service Onboarding_; task 0311 decision 3: an operator changes a plan by
hand in AWS).

**Who:** anyone with `AdministratorAccess` on the shared AWS account
(`750702271865`, `eu-central-1`). The portal's own Lambda cannot do any of this:
it may list a key's plans, read usage and attach a key, but holds no `DELETE`
or `PATCH` on a plan or a plan key (`api-gateway-stack.ts`).

---

## The five plans

Since task 0311 there are five CDK-managed usage plans, all on the production
stage, all `Period.MONTH` with offset 0. The figures come from
`infra/envs/production.json` — `pricingApiFreePlan*` for free,
`pricingApiPaidPlans` for the four paid tiers — and the plans are defined in
`infra/src/lib/stacks/api-gateway-stack.ts`.

| Tier    | AWS plan name                    | Quota / month | Rate     | Burst |
| ------- | -------------------------------- | ------------- | -------- | ----- |
| Free    | `pricing-api-free-production`    | 100 000       | 1 req/s  | 5     |
| Basic   | `pricing-api-basic-production`   | 1 000 000     | 3 req/s  | 15    |
| Analyst | `pricing-api-analyst-production` | 5 000 000     | 5 req/s  | 25    |
| Lite    | `pricing-api-lite-production`    | 20 000 000    | 10 req/s | 50    |
| Pro     | `pricing-api-pro-production`     | 50 000 000    | 25 req/s | 125   |

Read the current figures from `production.json` rather than trusting this
table. **Do not hand-edit any of the five in the console** — the next deploy
reverts you, and the change is invisible in review.

Every key the portal issues lands on free. A key belongs to exactly one usage
plan per stage, so moving a user up (or down) means moving their key from one
plan to another — the procedure below. The dashboard reads whatever plan the
key is on (`GetUsagePlans` by key) and shows that plan's pill, figures, quota
and reset; the **name** is the contract: the tier is parsed from
`pricing-api-<tier>-production`, and any other plan on our stage shows as
**Custom** with its own name.

---

## Upgrade a user

A user asks for a paid plan and the terms are agreed out of band. Their key
keeps its value; only its plan changes.

```bash
export AWS_PROFILE=<shared-account-profile>
export AWS_REGION=eu-central-1
export ID=<the user's Discord id, a snowflake>
export TIER=basic               # free | basic | analyst | lite | pro
```

**1. Find the key — by exact name.** A self-service key is named
`discord-<id>-key` (`packages/prices-api/src/portal/keys/naming.rs`, `key_name`).
`--name-query` is a **prefix** match (measured, task 0180), so the exact match
is the JMESPath filter, and `enabled` drops a revocation record:

```bash
aws apigateway get-api-keys --name-query "discord-$ID-key" \
  --query "items[?name=='discord-$ID-key' && enabled].id" --output text
```

Expect exactly **one** id. None: the user has no live key (never issued, or
revoked — they issue one first). Two: a double-submit duplicate the next issue
will sweep; ask the user to open the dashboard once (a sign-in reconciles) and
look again. Then:

```bash
export K=<the one id>
```

**2. Find the two plans.** The key's current plan, and the target by exact name:

```bash
aws apigateway get-usage-plans --key-id "$K" \
  --query "items[].[id,name,apiStages[0].apiId]" --output table

FROM=<the id of the plan above whose name is pricing-api-…-production>

TO=$(aws apigateway get-usage-plans \
  --query "items[?name=='pricing-api-${TIER}-production'].id | [0]" --output text)
# `--output text` prints the literal "None" for an empty result — see
# "Change or revoke a Custom key" below. Guard it rather than pass it on.
[ "$TO" = "None" ] && { echo "no plan pricing-api-${TIER}-production"; unset TO; }
echo "from ${FROM} to ${TO}"
```

A key may also sit on another API's plan (the partner plan, `q7sd40`, is on a
different API); only the plan on our API — `/prices/production/api-gateway-id`
— is the one to move.

**3. Move it: delete the plan key, then create it on the target plan.**

```bash
aws apigateway delete-usage-plan-key --usage-plan-id "$FROM" --key-id "$K"
aws apigateway create-usage-plan-key --usage-plan-id "$TO" --key-id "$K" \
  --key-type API_KEY
```

Delete first, because a key is on one plan per stage: creating it on the target
while it is still on the old plan is refused. **Between the two commands the
key answers `403`** — it exists but is on no plan — so run them back to back;
it is a matter of seconds, and the data plane can take a few more to follow.
The key's **value does not change**, so the user changes nothing.

**4. Verify.**

```bash
aws apigateway get-usage-plans --key-id "$K" --query "items[].name"
```

Exactly the target plan (`["pricing-api-basic-production"]`), plus any plan on
another API it was already on — never two plans of ours, and never none.

**5. What the user sees.**

- **The counter starts from zero.** Usage is counted per `(plan, key)` pair, so
  the key's usage on the new plan begins at 0 for the rest of the month; what
  it used on the old plan is not carried over. Say so if they ask why their
  "used" figure dropped.
- **The dashboard shows the new plan within 60 s** — the usage cache's TTL:
  the Rate Limit card's pill and figures, and the Monthly Usage card's quota
  and reset date.
- **A rework keeps the plan.** "Replace my key" (task 0191) revokes the key;
  the next issue after the period rolls attaches the new key to the plan the
  revoked key was on, and only then deletes the revoked key (task 0311). A paid
  user stays paid, and nothing needs doing here. The once-per-period rework cap
  is the same on every plan.

**Downgrade** (a paid plan lapses): the same procedure with `TIER=free`.

**6. Record it.** Add a row to the "Issued manual keys" table at the bottom of
this file (customer, plan, key id, date, who) and commit — the move is outside
CDK and this file is its only record.

---

## Post-merge production verification (Adam)

Task 0311's "on dev" checks are a production checklist — there is no dev
environment (`infra/envs/` holds only `production.json` and `cicd.json`).

1. Deploy Compute, then ApiGateway (the Lambda learns `PORTAL_API_ID_PARAM` /
   `PORTAL_API_STAGE` in Compute; the four plans and the widened policy land in
   ApiGateway). In the ApiGateway diff the free plan, its key and its SSM
   parameter must show **no change** — only the four new plans and the policy.
2. `/config` answers `enabled: true` (the portal did not close at cold start).
3. Move a test key free → Basic → Pro → free with the procedure above. After
   each step:
   - `get-usage-plans --key-id` shows exactly the target plan;
   - the gateway throttles at that plan's rate (a burst above it gets `429`);
   - within 60 s both dashboard cards show that plan's figures, and the Rate
     Limit card the plan's pill.
4. A rework on Basic: revoke the test key while on Basic, wait for the next
   period (or use a key revoked last period), issue — the new key is on
   `pricing-api-basic-production`, and the revoked key is gone.
5. A free key's dashboard looks as it did before, apart from the `Free` pill
   and the contact link.

---

## Custom / Enterprise plans (hand-made)

Negotiated limits that match none of the five tiers — an Enterprise customer, a
load test — are still a plan made by hand, deliberately **not** in CDK: its
numbers are case by case, and carrying them as config would mean inventing a
field per customer.

The trade-off is real and worth stating: a hand-made plan is **drift**. It does
not appear in `cdk diff`, nobody reviews it, and it survives only as long as
someone remembers it exists. That is why step 5 below is not optional.

⚠️ **A hand-made plan must be attached to our API stage to be reported as
Custom.** The dashboard keeps only plans whose `apiStages` contain our API id
and `production`; a plan on no stage, or on another API, is not the key's plan
as far as the portal is concerned — the dashboard says the key is on **no
plan**, and the gateway answers the key `403`. Step 1 below attaches the stage;
keep it that way for as long as a key is on the plan.

A self-service `discord-` key moved onto a Custom plan uses the "Upgrade a
user" procedure with the Custom plan's id as `TO`. The rest of this section
issues a separate, hand-made key.

### Issue a Custom key

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

### Change or revoke a Custom key

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
  the bigger plan", and moving a key is delete-then-create (with seconds of
  `403` between). (A key may sit in up to 10 plans overall, across different
  stages; that does not help here.)
- **Do not delete a Custom plan out from under a revoked key.** The portal
  reads a rework's target plan off the revoked key (task 0311); if that key's
  plan is gone, the new key lands on free — a silent downgrade of exactly the
  kind 0311 removed. Wind a customer down only once nothing of theirs is
  revoked-and-waiting.
- **Rename none of the five CDK plans by hand.** The tier is parsed from the
  exact name `pricing-api-<tier>-production`; a renamed plan shows as Custom on
  the dashboard (and the next deploy renames it back).
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

| Customer                           | Plan name                                                          | Plan ID                                               | Key name                                              | Key ID       | Limits                                | Issued     | Issued by      |
| ---------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------- | ----------------------------------------------------- | ------------ | ------------------------------------- | ---------- | -------------- |
| loadtest (internal, task 0121)     | `prices-production-loadtest-plan`                                  | `i12bsj`                                              | `prices-production-loadtest-key-20260921T070812Z`     | `gc22sbmwa2` | 150 req/s, burst 300, 1,000,000/month | 2026-09-21 | stkrolikiewicz |
| scf-reviewer (external, task 0128) | `pricing-api-free-production` (CDK-managed, **not** a manual plan) | see SSM `/prices/production/pricing-api-free-plan-id` | `prices-production-scf-reviewer-key-20260909T120021Z` | `l1kqdj0123` | 1 req/s, burst 5, 100,000/month       | 2026-09-09 | okarcz         |

⚠️ **The `scf-reviewer` row is the one exception to this file's shape:** it is a
hand-made key on the **CDK-managed free plan**, not on a manual plan of its own,
because the SCF reviewer needs the published free tier rather than negotiated
limits. Steps 1 and 3 of the runbook therefore do not apply to it; steps 2, 4 and
5 do. **Its value is published in `docs/scf/milestone-2-evidence.md`**, so treat
it as public and revoke it once the milestone review closes:
`aws apigateway delete-api-key --api-key l1kqdj0123`.

**Key name** is a column because key names are not unique and now carry the issuing
instant — during a rotation two rows may share a customer, and the name is what
tells them apart. Keep both rows until the old key is deleted.
