---
title: "API Gateway usage plan and key mechanics behind the one-key model"
type: research
status: developing
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - notes/S-account-model-and-abuse-barrier.md
tags: [aws, api-gateway, usage-plan, quota, api-keys]
links: []
history:
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Researched AWS quota scoping, period reset and key management semantics"
---

# API Gateway usage plan and key mechanics behind the one-key model

Scope: AWS REST API Gateway (`apigateway`, the 2015-07-09 control plane) only.
Every claim below is sourced to a page fetched on 2026-08-10. Where AWS does not
state something, that is said plainly rather than inferred.

---

## 1. What a usage plan quota is scoped to

A quota is defined against **an API key**, and is counted **within one usage
plan**, aggregated across all API stages in that plan:

> *A quota limit* sets the target maximum number of requests with a given API key
> that can be submitted within a specified time interval.

> Throttling and quota limits apply to requests for individual API keys that are
> aggregated across all API stages within a usage plan.

So the counter is keyed on the **(usage plan, API key) pair** — not on the plan
alone and not on the key alone. AWS never states the phrase "per (usage plan, API
key) pair", but the API surface makes the pairing explicit: usage is addressed at
`/usageplans/{usageplanId}/keys/{keyId}/usage` (see §3, §9) and `GetUsage`
returns a map indexed by API key ID *inside* a single usage plan.

Load-bearing caveat — the quota is a target, not a ceiling:

> Usage plan throttling and quotas are not hard limits, and are applied on a
> best-effort basis. In some cases, clients can exceed the quotas that you set.
> Don't rely on usage plan quotas or throttling to control costs or block access
> to an API.

> Source: [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — fetched 2026-08-10

---

## 2. Quota period options and when the period resets

The period is one of three values, and that is the whole of the type:

> **period**
> The time period in which the limit applies. Valid values are "DAY", "WEEK" or "MONTH".
> Type: String
> Valid Values: `DAY | WEEK | MONTH`
> Required: No

> **limit**
> The target maximum number of requests that can be made in a given time period.

> **offset**
> The number of requests subtracted from the given limit in the initial time period.

> Source: [QuotaSettings — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_QuotaSettings.html) — fetched 2026-08-10

**`offset` is not a time offset.** It is a *request count* subtracted from the
limit in the first period only. Anyone reading `offset` as "shift the reset day"
is reading it wrong. CloudFormation repeats the identical wording and adds only a
minimum:

> `Offset`
> The number of requests subtracted from the given limit in the initial time period.
> *Required*: No  *Type*: Integer  *Minimum*: `0`

> `Period`
> The time period in which the limit applies. Valid values are "DAY", "WEEK" or "MONTH".
> *Allowed values*: `DAY | WEEK | MONTH`

> Source: [AWS::ApiGateway::UsagePlan QuotaSettings — AWS CloudFormation](https://docs.aws.amazon.com/AWSCloudFormation/latest/UserGuide/aws-properties-apigateway-usageplan-quotasettings.html) — fetched 2026-08-10

The **only** statement AWS makes about when a period rolls over is a caption on a
CLI example:

> The following [create-usage-plan] command creates a usage plan that resets at
> the beginning of the month:
>
> ```
> aws apigateway create-usage-plan \
>     --name "New Usage Plan" \
>     --description "A new usage plan" \
>     --throttle burstLimit=10,rateLimit=5 \
>     --quota limit=500,offset=0,period=MONTH
> ```

> Source: [Set up usage plans for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-create-usage-plans.html) — fetched 2026-08-10

**Not stated in the source:** the timezone or exact instant of the reset. AWS
does not say the period boundary is UTC midnight; does not say which weekday
starts a `WEEK`; and does not say whether a `MONTH` boundary is the calendar
month boundary or an anniversary of plan creation. The phrase "resets at the
beginning of the month" for `period=MONTH, offset=0` is the strongest evidence
available and points at the calendar month, but it is prose in an example
caption, not a specification. **If the reset instant is load-bearing for us, it
must be measured empirically, not cited.**

Corroborating hint (still not a specification): `GetUsage` reports usage as
**daily** buckets keyed by calendar date (`startDate` / `endDate` are dates, e.g.
`2016-08-01`), which means the underlying accounting has at least day
granularity aligned to dates — see §3.

---

## 3. `GetUsage`

> Gets the usage data of a usage plan in a specified time interval.
>
> ```
> GET /usageplans/{usageplanId}/usage?endDate={endDate}&keyId={keyId}&limit={limit}&position={position}&startDate={startDate} HTTP/1.1
> ```

URI parameters, verbatim:

> **endDate** — The ending date (e.g., 2016-12-31) of the usage data. Required: Yes
> **keyId** — The Id of the API key associated with the resultant usage data.
> **limit** — The maximum number of returned results per page. The default value is 25 and the maximum value is 500.
> **position** — The current pagination position in the paged result set.
> **startDate** — The starting date (e.g., 2016-01-01) of the usage data. Required: Yes
> **usageplanId** — The Id of the usage plan associated with the usage data. Required: Yes

Note: `usageplanId`, `startDate`, `endDate` are **required**; `keyId` is
**optional** — omit it and you get every key in the plan.

Response payload:

> **values**
> The usage data, as daily logs of used and remaining quotas, over the specified
> time interval indexed over the API keys in a usage plan. For example,
> `{..., "values" : { "{api_key}" : [ [0, 100], [10, 90], [100, 10]]}`, where
> `{api_key}` stands for an API key ID and the daily log entry is of the format
> `[used quota, remaining quota]`.
> Type: String to array of arrays of longs map

The response field is `values` (a map), **not** `items`. Granularity is **daily**,
one `[used, remaining]` pair per day, indexed by API key ID.

> Source: [GetUsage — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_GetUsage.html) — fetched 2026-08-10

The developer guide describes the same shape from the console export path and
confirms the daily reading:

> The usage data in the example shows the daily usage data for an API client, as
> identified by the API key (`px1KW6...qBazOJH`), between August 1, 2016 and
> August 3, 2016. Each daily usage data shows used and remaining quotas.

> Source: [Maintain a usage plan for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-usage-plan-manage-usage.html) — fetched 2026-08-10

**Documented lag / eventual consistency: not stated in the source.** Neither the
API reference nor the developer guide makes any statement about how soon a
request is reflected in `GetUsage`, nor any consistency guarantee. The only
propagation delay AWS documents anywhere nearby is for key-to-plan association,
which is a different operation:

> After you add an API key to a usage plan, the update operation might take a few
> minutes to complete.

> Source: [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — fetched 2026-08-10

---

## 4. Key ↔ plan cardinality, and whether AWS has a "user"

**A key can belong to multiple plans. A plan can hold multiple keys.** Both
directions are explicitly many:

> An API key can be associated with more than one usage plan. A usage plan can be
> associated with more than one stage. However, a given API key can only be
> associated with one usage plan for each stage of your API.

> Source: [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — fetched 2026-08-10

The same note is repeated verbatim at the bottom of the "Add an API key to a
usage plan" procedure, alongside a bulk-import example that puts one key into
several plans at once ("change each `usageplanIds` column value to a
comma-separated string").

> Source: [Set up usage plans for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-create-usage-plans.html) — fetched 2026-08-10

The plan-holds-many-keys direction is confirmed by the collection endpoint:

> **GetUsagePlanKeys** — Gets all the usage plan keys representing the API keys
> added to a specified usage plan.

> Source: [GetUsagePlanKeys — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_GetUsagePlanKeys.html) — fetched 2026-08-10

Cardinality is also a published service quota:

> | Usage plans per API key | Each supported Region: 10 | Yes | The maximum number of usage plans that you can associate with an API key |

> Source: [Amazon API Gateway endpoints and quotas](https://docs.aws.amazon.com/general/latest/gr/apigateway.html) — fetched 2026-08-10

**There is no AWS-native "user" that aggregates keys.** The `ApiKey` resource has
exactly these fields — `createdDate`, `customerId`, `description`, `enabled`,
`id`, `lastUpdatedDate`, `name`, `stageKeys`, `tags`, `value` — and none of them
is a quota-aggregating principal. The nearest candidate is `customerId`, and AWS
defines it purely as a *label*:

> **customerId** — An AWS Marketplace customer identifier, when integrating with
> the AWS SaaS Marketplace.

> Source: [CreateApiKey — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_CreateApiKey.html) — fetched 2026-08-10

> **customerId** — The identifier of a customer in AWS Marketplace or an external
> system, such as a developer portal.

> Source: [GetApiKeys — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_GetApiKeys.html) — fetched 2026-08-10

`customerId` is a **filter for listing keys**. No AWS page states that quota or
throttling is summed over `customerId`, and §1 states the opposite — quota
applies to "requests for individual API keys". Aggregating usage across several
keys held by one person is therefore **our** work, not AWS's: it means N
`GetUsage` calls and our own summation.

---

## 5. Throttling: how the layers compose

Four layers, and an explicit precedence order:

> Amazon API Gateway provides four basic types of throttling-related settings:
> + *AWS throttling limits* are applied across all accounts and clients in a Region. These limit settings exist to prevent your API—and your account—from being overwhelmed by too many requests. These limits are set by AWS and can't be changed by a customer.
> + Per-account limits are applied to all APIs in an account in a specified Region. The account-level rate limit can be increased upon request... Note that these limits can't be higher than the AWS throttling limits.
> + Per-API, per-stage throttling limits are applied at the API method level for a stage. You can configure the same settings for all methods, or configure different throttle settings for each method. Note that these limits can't be higher than the AWS throttling limits.
> + *Per-client throttling limits* are applied to clients that use API keys associated with your usage plan as client identifier. Note that these limits can't be higher than the per-account limits.

> API Gateway applies your throttling-related settings in the following order:
>
> 1. Per-client or per-method throttling limits that you set for an API stage in a usage plan
> 2. Per-method throttling limits that you set for an API stage
> 3. Account-level throttling per Region
> 4. AWS Regional throttling

Mechanism and defaults:

> API Gateway throttles requests to your API using the token bucket algorithm,
> where a token counts for a request. Specifically, API Gateway examines the rate
> and a burst of request submissions against all APIs in your account, per Region.

> By default, API Gateway limits the steady-state requests per second (RPS)
> across all APIs within an AWS account, per Region. It also limits the burst
> (that is, the maximum bucket size) across all APIs within an AWS account, per
> Region.

> Source: [Throttle requests to your REST APIs for better throughput in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-request-throttling.html) — fetched 2026-08-10

The per-plan knobs themselves:

> **burstLimit** — The API target request burst rate limit. This allows more requests through for a period of time than the target rate limit. Type: Integer
> **rateLimit** — The API target request rate limit. Type: Double

> Source: [ThrottleSettings — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_ThrottleSettings.html) — fetched 2026-08-10

Composition, stated plainly: per-client throttle is the *innermost* gate and is
capped by the account limit — it does not add capacity, it only subdivides it.
Per-key throttles across many keys all draw on the same regional account bucket
(§8).

---

## 6. `CreateApiKey` / `GetApiKeys` — names, uniqueness, and `nameQuery`

`nameQuery` is documented in one sentence and that sentence says nothing about
matching:

> **nameQuery** — The name of queried API keys.

> Source: [GetApiKeys — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_GetApiKeys.html) — fetched 2026-08-10

The AWS CLI reference reproduces the same sentence for `--name-query` with no
elaboration.

> Source: [get-api-keys — AWS CLI Command Reference](https://docs.aws.amazon.com/cli/latest/reference/apigateway/get-api-keys.html) — fetched 2026-08-10

The sibling parameter on the usage-plan collection is equally thin:

> **nameQuery** — A query parameter specifying the name of the to-be-returned usage plan keys.

> Source: [GetUsagePlanKeys — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_GetUsagePlanKeys.html) — fetched 2026-08-10

**Prefix vs. exact match: not stated in the source.** No AWS page fetched
describes the matching semantics of `nameQuery`. Do not design a
lookup-key-by-Discord-ID scheme on an assumption here. (Widely-reported behaviour
is prefix matching, but that is community knowledge, not AWS documentation — it
must be measured before we rely on it, and even then it is undocumented
behaviour that AWS has not committed to.)

On uniqueness — **values are unique and enforced; names are not**:

> API key values must be unique. If you try to create two API keys with different
> names and the same value, API Gateway considers them to be the same API key.

> An API key has a name and a value. (The terms "API key" and "API key value" are
> often used interchangeably.) The name cannot exceed 1024 characters. The value
> is an alphanumeric string between 20 and 128 characters, for example,
> `apikey1234abcdefghij0123456789`.

> Source: [Usage plans and API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-api-usage-plans.html) — fetched 2026-08-10

In `CreateApiKey`, `name` is `Required: No` and carries no uniqueness constraint:

> **name** — The name of the ApiKey. Type: String  Required: No

> Source: [CreateApiKey — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_CreateApiKey.html) — fetched 2026-08-10

And the value is immutable once set:

> After you create an API key value, it cannot be changed.

> You cannot change the value of the new API key.

> Source: [Set up API keys for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-setup-api-keys.html) — fetched 2026-08-10

Practical read: nothing stops two keys carrying the same `name`. If we name keys
after a Discord ID, AWS will not enforce one-key-per-name for us — that
invariant lives in our registry ([[0158]]), not in API Gateway.

---

## 7. `DeleteApiKey` then `CreateApiKey` — does the counter reset?

`DeleteApiKey` is documented in four lines and says nothing about usage data:

> Deletes the ApiKey resource.
>
> ```
> DELETE /apikeys/{api_Key} HTTP/1.1
> ```
> ... If the action is successful, the service sends back an HTTP 202 response with an empty HTTP body.

> Source: [DeleteApiKey — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_DeleteApiKey.html) — fetched 2026-08-10

**AWS does not state anywhere that a replacement key inherits the deleted key's
consumption.** The conclusion is reached by construction from §1 + §3, not by
assertion:

- Quota is counted against **an API key** (§1) and usage is stored **indexed by
  API key ID** (§3, `values` is a map from `{api_key}` to daily
  `[used, remaining]`).
- `CreateApiKey` mints a **new** `id`:
  > **id** — The identifier of the API Key. Type: String
  > Source: [CreateApiKey — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_CreateApiKey.html) — fetched 2026-08-10
- Nothing in the `ApiKey` model links a new key to a deleted one — there is no
  predecessor, lineage, or carry-over field among `createdDate`, `customerId`,
  `description`, `enabled`, `id`, `lastUpdatedDate`, `name`, `stageKeys`, `tags`,
  `value`.
- Therefore a delete-then-create cycle produces a key ID for which no prior usage
  exists, and its `remaining` starts at the plan `limit` (less `offset` in the
  initial period, §2).

**This is a derivation, not a documented guarantee, and it is exactly the abuse
vector the rework cap exists to close**: a user who can freely delete and
re-create a key can reset their own consumption at will. The cap is what makes
the derivation safe.

---

## 8. Account and region limits that bound the design

Resource counts (Service Quotas, per account per Region):

> | API keys | Each supported Region: 10,000 | No | The maximum number of API keys that you can create in this account in the current region |
> | Usage plans | Each supported Region: 300 | Yes | The maximum number of usage plans that you can create in this account in the current region |
> | Usage plans per API key | Each supported Region: 10 | Yes | The maximum number of usage plans that you can associate with an API key |
> | API Stage throttles in a usage plan | Each supported Region: 20 | Yes | The maximum number of API-stage throttle settings you can create in a usage plan in this account in the current region |
> | Throttle rate | ... Each of the other supported Regions: 10,000 | Yes | The maximum number of requests per second that your APIs can receive in this account in the current region |
> | Throttle burst rate | ... Each of the other supported Regions: 5,000 | No | The maximum number of additional requests per second (RPS) that you can send in one burst in this account in the current region |

Note the asymmetry: **API keys — 10,000, `Can be increased: No`.** That is a hard
ceiling on how many self-issued keys can exist in one region. **Usage plans — 300,
adjustable.**

> Source: [Amazon API Gateway endpoints and quotas](https://docs.aws.amazon.com/general/latest/gr/apigateway.html) — fetched 2026-08-10

The developer guide states the same account throttle with the burst caveat:

> | Throttle quota per account, per Region across HTTP APIs, REST APIs, WebSocket APIs, and WebSocket callback APIs | 10,000 requests per second (RPS) with an additional burst capacity provided by the token bucket algorithm, using a maximum bucket capacity of 5,000 requests. * The burst quota is determined by the API Gateway service team based on the overall RPS quota for the account in the Region. It is not a quota that a customer can control or request changes to. | Yes |

Control-plane (management API) throttles — these are the ones that bite a
self-service issuance flow:

> The following fixed quotas apply to creating, deploying, and managing an API in
> API Gateway, using the AWS CLI, the API Gateway console, or the API Gateway
> REST API and its SDKs. **These quotas can't be increased.**

> | CreateApiKey | 5 requests per second per account | No |
> | DeleteApiKey | 5 requests per second per account | No |
> | UpdateUsagePlan | 1 request every 20 seconds per account | No |
> | Other operations | No quota up to the total account quota. | No |
> | Total operations | 10 requests per second with a burst quota of 40 requests per second. | No |

> Source: [Amazon API Gateway quotas](https://docs.aws.amazon.com/apigateway/latest/developerguide/limits.html) — fetched 2026-08-10

`GetUsage` falls under "Other operations" / the 10 RPS "Total operations" cap —
it has no dedicated row. So a backend that fans out `GetUsage` per key to compute
a per-user total (§4) is spending from a **10 RPS, burst 40, non-adjustable**
account-wide budget shared with every other control-plane call we make,
including deploys.

`UpdateUsagePlan` at **1 request every 20 seconds, non-adjustable** rules out any
design that gives each user their own usage plan and edits it — even 300 users
would take 100 minutes to reconfigure serially.

---

## 9. `UpdateApiKey` with `enabled=false`

The operation exists and `/enabled` is an explicitly supported patch path:

> **UpdateApiKey** — Changes information about an ApiKey resource.
>
> ```
> PATCH /apikeys/{api_Key} HTTP/1.1
> ```

> **enabled** — Specifies whether the API Key can be used by callers. Type: Boolean

> Source: [UpdateApiKey — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_UpdateApiKey.html) — fetched 2026-08-10

> ## UpdateApiKey
> | Path | op:add | op:replace | op:remove | op:copy |
> | `/customerId` | Not supported | Supported | Not supported | Not supported |
> | `/description` | Not supported | Supported | Not supported | Not supported |
> | `/enabled` | Not supported | Supported | Not supported | Not supported |
> | `/labels` | Supported | Not supported | Supported | Not supported |
> | `/name` | Not supported | Supported | Not supported | Not supported |
> | `/stages` | Supported | Not supported | Supported | Not supported |

> Source: [Patch Operations — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/patch-operations.html) — fetched 2026-08-10

So `op=replace, path=/enabled, value=false` is the documented disable path. It
keeps the `ApiKey` resource, its `id`, and its `value` in place.

**Effect on usage counters: not stated in the source.** No AWS page fetched says
whether disabling zeroes, freezes, or preserves accumulated usage. Because the
key `id` survives (unlike §7), the derivation of §7 does **not** carry over —
disable is not documented to reset anything. If we need "revoke without granting
a free quota reset", `enabled=false` is the operation whose semantics we should
verify empirically before relying on it.

Related, and worth knowing exists: the quota counter *can* be moved directly,
without touching the key at all:

> **UpdateUsage** — Grants a temporary extension to the remaining quota of a usage
> plan associated with a specified API key.
>
> ```
> PATCH /usageplans/{usageplanId}/keys/{keyId}/usage HTTP/1.1
> ```
> Example: `{"op": "replace", "path": "/remaining", "value": "10"}`

> Source: [UpdateUsage — Amazon API Gateway](https://docs.aws.amazon.com/apigateway/latest/api/API_UpdateUsage.html) — fetched 2026-08-10

`/remaining` is `op:replace` only, per the patch-operations table. The console
exposes this as "Grant usage extension", and it works in both directions:

> You can increase the renaming [sic] requests or decrease the remaining requests
> for the time period of your usage plan.

> Source: [Maintain a usage plan for REST APIs in API Gateway](https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-usage-plan-manage-usage.html) — fetched 2026-08-10

This is a *sharper* tool than key rework for the case "a user needs more quota
this month": it costs one control-plane call, leaves the key untouched, and does
not require the user to re-integrate against a new secret.

---

## Consequence for the one-key model

- **AWS bounds a key, never a person.** Quota is charged per (usage plan, API
  key) (§1) and there is no principal that sums keys — `customerId` is a listing
  filter, not an accounting unit (§4). Under a multi-key model, bounding a user's
  total consumption becomes our aggregation code over N `GetUsage` calls, which
  is precisely the work [[0160]]'s rework rule exists to avoid. **The epic's
  claim holds.**
- **Multi-key is not prevented by AWS, only unaccounted-for.** A key may sit in
  up to 10 usage plans, and a plan holds arbitrarily many keys (§4); nothing
  enforces one key per name (§6). One-active-key is therefore an invariant our
  registry ([[0158]]) must own — API Gateway will not maintain it for us.
- **Delete-then-create almost certainly resets the counter, and AWS never says
  so.** It follows from usage being indexed by a key ID that `CreateApiKey`
  re-mints (§7), not from any documented guarantee. This is the exact loophole
  the once-per-quota-period rework cap closes; the cap should be justified in the
  ADR on that derivation, with the derivation shown.
- **The month boundary is undocumented.** Only an example caption says a
  `period=MONTH, offset=0` plan "resets at the beginning of the month"; timezone
  and exact instant are not stated, and `offset` is a request count, not a time
  shift (§2). Any ADR wording of the form "quota resets at 00:00 UTC on the 1st"
  is currently unsourced and must be measured before it is written down.
- **Control-plane rates cap the issuance design, not the traffic design.**
  `CreateApiKey` 5 RPS, total control-plane 10 RPS / burst 40, `UpdateUsagePlan`
  1 per 20s — all non-adjustable (§8). Per-user usage plans are ruled out; a
  single shared plan with one key per user is the only shape these numbers
  support, and per-user `GetUsage` fan-out competes with our deploys for the same
  10 RPS.
