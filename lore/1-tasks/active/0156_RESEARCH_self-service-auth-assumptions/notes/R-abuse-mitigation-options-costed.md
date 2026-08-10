---
title: "Abuse-mitigation options for self-issued keys, costed"
type: research
status: developing
spawned_from: notes/Q-do-the-two-flagged-auth-assumptions-hold.md
spawns:
  - notes/S-account-model-and-abuse-barrier.md
tags: [abuse-prevention, captcha, cost, rate-limits, benchmarking]
links: []
history:
  - date: 2026-08-10
    status: developing
    who: claude
    note: "Costed captcha, email, quota and approval mitigations against sourced pricing"
---

# Abuse-mitigation options for self-issued keys, costed

Scope of this note: **what each candidate mitigation costs, and what the abuse it
prevents costs.** Discord platform mechanics, OAuth scope semantics, the Stellar
Discord server's actual posture, and AWS usage-plan mechanics are covered
elsewhere in this task and are taken as given here.

Every price below was fetched on **2026-08-10**. Prices change; re-check before
quoting these in the ADR if more than a quarter has passed.

## Headline

The thing we are defending is worth **$0.38/month per fully-drained key**. Four of
the five candidate mitigations cost more than that — two of them cost more per
*legitimate* signup than a maximally abusive key costs us per month. The only
mitigations that survive the arithmetic are the two that are free: account-age
gating (b) and, if the server posture justifies it, the guild-membership check
(a). Captcha is free in one specific product (Turnstile) and therefore also
survives, but on engineering cost rather than licence cost.

---

## 1. Cost exposure of the abuse we are defending against

### 1.1 The unit prices

Region: **US East (N. Virginia)**. AWS's own worked examples on the API Gateway
pricing page are explicitly stamped for this region — "Example below reflects
pricing for US East (N. Virginia, Ohio), US West (Oregon), Asia Pacific (Mumbai)".
The Europe (Frankfurt) REST figures are behind a JavaScript region selector that
did not render for automated fetch; **eu-central-1 REST request pricing could not
be established as of 2026-08-10**. Frankfurt is typically at a small premium to
N. Virginia, so treat the numbers below as a floor.

REST API requests, first tier:

> "Amazon API Gateway API call charges = 5 million * $3.50/million = $17.50"

Tier thresholds (only relevant far above our volumes):

> "Amazon API Gateway API call charges = 333 million * $3.50/million = $1,165.50 / 667 million * $2.80/million = $1,867.60 / 14 billion * $2.38/million = $33,320.00"

Data transfer out:

> "Amazon API Gateway data transfer charges = 14.3 GB * $0.09 = $1.29" (for "3 KB * 5 million")

API caching:

> "Pricing Example with Caching Required (US East, US West, EU (Ireland))"
> "If your API needs 1.5 GB of cache for its data, you can provision a 1.6 GB cache at $0.038/hr."
> "$0.038 * 24 = $0.912/day"

The full cache-size × hourly-rate matrix is JS-rendered and **could not be
extracted as of 2026-08-10**; only the 1.6 GB data point is quotable. AWS's own
Private API example uses "720 hours" as a month, so 720 h is used below.

> Source: [Amazon API Gateway Pricing](https://aws.amazon.com/api-gateway/pricing/) — fetched 2026-08-10
> Archived: `sources/mitigation-aws-api-gateway-pricing.md`

### 1.2 What one key costs us

Epic limits: 1 req/s throttle, monthly quota 50k–100k calls. Response size
assumed at 3 KB, matching AWS's own worked example.

**One key, quota 100,000 calls, fully drained:**

```
requests:      100,000 / 1,000,000 × $3.50            = $0.350
data transfer: 100,000 × 3 KB = 0.286 GB × $0.09      = $0.026
                                                        -------
                                                        $0.376  ≈ $0.38 / month
```

**One key, quota 50,000 calls, fully drained:** $0.175 + $0.013 = **$0.19 / month**.

**The quota, not the throttle, is what bounds this.** 1 req/s sustained for a
30-day month is 1 × 86,400 × 30 = 2,592,000 calls, which at $3.50/million is
**$9.07/month**. The 100k quota is therefore a **26× reduction** in worst-case
per-key exposure, and it is enforced natively by AWS with no code of ours in the
path. This is the single most cost-effective control in the design and it is
already in the epic.

### 1.3 What churned keys cost

Assuming every key is drained to its 100k ceiling — the maximum-damage case:

| Churned keys | Calls/month | Request charge | Data transfer | Total/month |
|---|---|---|---|---|
| 1 | 100,000 | $0.35 | $0.03 | **$0.38** |
| 10 | 1,000,000 | $3.50 | $0.26 | **$3.76** |
| 100 | 10,000,000 | $35.00 | $2.57 | **$37.57** |
| 1,000 | 100,000,000 | $350.00 | $25.75 | **$375.75** |

Arithmetic: `n × 100,000 / 1,000,000 × $3.50` for requests;
`n × 100,000 × 3 KB / 1,048,576 KB-per-GB × $0.09` for transfer. All volumes stay
inside the first pricing tier (333 million requests), so the rate stays flat at
$3.50/million throughout.

**Reference points for the epic owner:**

- To reach **$100/month** of gateway request charges, an abuser needs
  `$100 / $3.50 × 1,000,000 = 28.6 million` calls — **286 fully-drained keys**,
  i.e. 286 distinct Discord accounts, each consuming its entire monthly quota.
- To reach **$1,000/month**: 285.7 million calls, **2,857 fully-drained keys**.

### 1.4 The comparison that settles it

Optional API Gateway caching, if we enable it, costs
`$0.038/hr × 720 hr = **$27.36/month**` — flat, regardless of how many keys exist.

That single fixed line item equals **72 fully-drained abusive keys**
(`$27.36 / $0.38`). Any mitigation with a recurring licence cost above roughly
$25/month is, by construction, more expensive than the abuse it prevents at any
plausible abuse volume.

### 1.5 What this number does *not* include

$3.50/million is the **gateway charge only**. The backend cost per call —
compute, ClickHouse query time, storage reads — is not priced here and is very
likely the dominant term. **The true per-call cost could not be established as of
2026-08-10** and needs a separate number from whoever owns the query layer. If
backend cost is, say, 10× the gateway cost, every figure in §1.3 scales by 11×
and the conclusions shift: 100 churned keys become a ~$400/month problem rather
than a ~$38/month one. **The ADR should state the assumed all-in per-call cost
explicitly rather than inheriting the gateway-only figure.**

---

## 2. Captcha / bot-detection options

### 2.1 Cloudflare Turnstile

Free plan, verbatim from the plans matrix:

> "Unlimited challenges (traffic or verification requests)" — listed for **both**
> Free and Enterprise.

Free-plan limits are structural, not volumetric:

> "Up to 20 widgets" (Free) vs "Unlimited" (Enterprise)
> "10 hostnames per widget" (Free) vs "Maximum of 200 hostnames per widget" (Enterprise)
> "7 days maximum" analytics lookback (Free) vs "30 days maximum" (Enterprise)
> "Free users are limited to 20 widgets per account. Customers with Enterprise Bot Management and Enterprise Turnstile can have this limit increased."

Enterprise adds ephemeral IDs, off-label (no-Cloudflare-branding) widgets, and
any-hostname widgets. Price for Enterprise is "contact sales" — **not publicly
published as of 2026-08-10**.

> Source: [Plans · Cloudflare Turnstile docs](https://developers.cloudflare.com/turnstile/plans/) — fetched 2026-08-10
> Archived: `sources/mitigation-cloudflare-turnstile-plans.md`

**Backend verification is mandatory**, and this is the real cost:

> "You must call the Siteverify API to complete your Turnstile implementation. The client-side widget alone does not protect your forms."
> Endpoint: `POST https://challenges.cloudflare.com/turnstile/v0/siteverify`
> "Each token can only be validated once. A replayed token will be rejected with the `timeout-or-duplicate` error code."
> Tokens expire 300 seconds after generation.

> Source: [Server-side validation · Cloudflare Turnstile docs](https://developers.cloudflare.com/turnstile/get-started/server-side-validation/) — fetched 2026-08-10

Implication for our shape: the portal is static, so the siteverify call and its
secret must live in the key-issuance backend ([[0160]]). That is one extra
outbound HTTPS call in the issuance path plus one more SSM secret — real work,
but it lands on a backend that already exists.

**Caveat:** one widget covers our one portal, so the 20-widget Free cap is not
binding. Analytics lookback of 7 days *is* mildly binding if we ever want to
retrospectively investigate an abuse wave older than a week.

### 2.2 hCaptcha

> Basic (Free): "$0"
> Pro: "$139/month" monthly billing, "$99/month" billed annually — "100K monthly evals included, then $0.99/1K"
> Enterprise: "Talk to Sales" — custom pricing
> Pro trial: "14-Day Trial with hCaptcha Pro!" … "you'll be automatically switched to the Free plan after 14 days if you decide not to keep Pro."

Published effectiveness claim, with its own disclaimer:

> Enterprise is "More accurate and up to 50% more cost-effective than reCAPTCHA" — the page carries a disclaimer that cost and accuracy comparisons are "based on customer-reported comparison data."

Treat that claim as marketing, not evidence: it is self-reported, uncontrolled,
and unsourced to any published methodology.

> Source: [hCaptcha Pricing](https://www.hcaptcha.com/pricing) — fetched 2026-08-10
> Archived: `sources/mitigation-hcaptcha-pricing.md`

**The free tier's volume cap could not be established as of 2026-08-10.** Neither
the pricing page nor the docs FAQ states a request or verification limit for the
Basic (Free) plan — the only published volume figure is Pro's "100K monthly
evals". Planning on the free tier without a published cap is a risk: there is no
document to point at if it changes.

> Source: [hCaptcha FAQ](https://docs.hcaptcha.com/faq/) — fetched 2026-08-10 (contains no free-tier volume statement)

Cost framing: **hCaptcha Pro at $139/month is 366 fully-drained abusive keys per
month** (`$139 / $0.38`). It cannot pay for itself against this threat model.

### 2.3 Google reCAPTCHA

Verbatim from Google's billing documentation:

> "0–10,000 assessments per calendar month: Free per organization"
> "10,001–100,000 assessments per month: $8 flat fee"
> "Over 100,000 assessments per month: $0.001 per assessment ($1.00 per 1,000 assessments)"
> "Up to 10,000 assessments per calendar month per organization"
> "Requests return a `Resource Exhausted (429)` quota error after your organization exceeds 10,000 cumulative assessments"

> Source: [Billing information | Google Cloud Fraud Defense](https://docs.cloud.google.com/recaptcha/docs/billing-information) — fetched 2026-08-10

**Two caveats that matter more than the price.**

1. The free allowance is **per organization, aggregated across all accounts and
   all sites** — not per project. If anything else under the same Google Cloud
   org already runs reCAPTCHA, our 10,000 is shared, and we discover this by
   getting 429s.
2. The failure mode on the free tier is a **hard 429**, not a degrade. A signup
   flow gated on reCAPTCHA stops issuing keys entirely when the org allowance is
   exhausted. That is an availability coupling we would be introducing for a
   $0.38/key threat.

At our scale reCAPTCHA would almost certainly cost $0 or $8/month. The objection
is not the price; it is the org-level coupling and the hard failure mode.

### 2.4 Captcha verdict

If a captcha is wanted, **Turnstile is the only one whose published free tier is
volume-unlimited and whose failure mode we control**. hCaptcha's free cap is
undocumented; reCAPTCHA's free cap is shared org-wide and fails closed.

But note what a captcha buys here: it raises the cost of *automating* signups. It
does nothing against a human churning accounts, and Discord OAuth already forces
a Discord account per key. Captcha is a second lock on a door that already has
one — and the first lock is the one whose strength is actually in question.

---

## 3. Email confirmation

### 3.1 Money cost: negligible

> À la carte: "Outbound email | $0.10 / 1,000 emails"
> Essentials plan: "0 – 10M emails / month: $0.16 per 1,000 emails"
> Pro plan: "0 – 10M emails / month: $0.22 per 1,000 emails"; Pro requires a monthly minimum of "$105/account/region/month"
> Across all options: "$0.12 per GB of attachment data sent"

> Source: [Amazon SES Pricing](https://aws.amazon.com/ses/pricing/) — fetched 2026-08-10
> Archived: `sources/mitigation-aws-ses-pricing.md`

At à la carte rates, **1,000 confirmation emails cost $0.10**; 10,000 cost $1.00.
Stay off the Pro plan — its "$105/account/region/month" minimum is 276
fully-drained keys' worth of exposure for a feature we would use a few hundred
times a month.

### 3.2 The real cost: production access

Verbatim from the AWS SES developer guide:

> "We place all new accounts in the Amazon SES *sandbox*. The sandbox status for your account is unique per each AWS Region."
> "You can only send mail **to** verified email addresses and domains, or to the Amazon SES mailbox simulator."
> "You can send a maximum of 200 messages per 24-hour period."
> "You can send a maximum of 1 message per second."
> "For account-level suppression, bulk actions and SES API calls related to suppression list management are disabled."
> "The AWS Support team provides an initial response to your request within 24 hours."
> "In order to prevent our systems from being used to send unsolicited or malicious content, we have to consider each request carefully. If we're able to do so, we'll grant your request within this 24-hour period. However, if we need to obtain additional information from you, it might take longer to resolve your request."

> Source: [Request production access (Moving out of the Amazon SES sandbox)](https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html) — fetched 2026-08-10

So email confirmation **cannot ship without an AWS Support ticket** — sandbox mode
can only mail addresses we have pre-verified, which is exactly the set of people
who do not need confirming. Add to that: a sending domain and DKIM setup, a
bounce/complaint handling process (AWS requires you to attest you have one), and
a reputation to maintain. That is the L-sized part.

### 3.3 What it actually proves against a determined abuser

Very little, and less than usual in our specific case:

- **Discord accounts already carry a verified email.** Discord's own server
  verification level LOW is defined as "must have verified email on account"
  (see §5) — the level exists precisely because email verification is an
  attribute Discord already holds. Adding our own confirmation step re-establishes
  a fact the identity provider has already established.
- Disposable-inbox services defeat email confirmation at zero marginal cost to
  the abuser. Email confirmation proves *control of an inbox at signup time*,
  nothing more — not uniqueness, not persistence, not humanity.

**Recommendation: do not build email confirmation.** It is the only candidate that
is simultaneously the most work (SES production access, domain, DKIM,
bounce handling) and the weakest barrier, and it duplicates something our
identity provider already does.

---

## 4. Industry comparison: how comparable APIs gate free keys (2026)

| Provider | Free tier limits (verbatim) | Published gate |
|---|---|---|
| CoinGecko (Demo) | "10k call credits/mo", "100 calls/min" | "No credit card required" |
| CoinMarketCap (Basic) | "15,000" monthly call credits, "50 requests per minute" | Account signup; verification step not documented first-party |
| Alchemy (Free) | "Free 30M CU per month", "25 requests per second" | Card requirement not stated on pricing page |
| Infura (Free) | "3 Million" daily credit quota, "500 credits/second" | Card requirement not stated on pricing page |

> Source: [CoinGecko API Pricing](https://www.coingecko.com/en/api/pricing) — fetched 2026-08-10
> Source: [CoinMarketCap API Pricing Plans](https://coinmarketcap.com/api/pricing/) — fetched 2026-08-10
> Source: [Alchemy Pricing](https://www.alchemy.com/pricing) — fetched 2026-08-10
> Source: [Infura Pricing](https://www.infura.io/pricing) — fetched 2026-08-10

Paid-tier anchors, for context on what the free tier is protecting:
CoinGecko Basic "$35/mo" for "100k call credits/mo"; CoinMarketCap Builder
"$29/mo" for "150,000" credits.

### 4.1 What could not be established

**No first-party documentation states that any of these four requires captcha, a
credit card, or a completed email-verification click for the free tier.** The
CoinMarketCap quick-start describes only "Sign up for an account at
pro.coinmarketcap.com/signup" and retrieving the key from the dashboard; it does
not mention email confirmation.

> Source: [CoinMarketCap API Documentation — Quick Start Guide](https://coinmarketcap.com/api/documentation/guides/quick-start) — fetched 2026-08-10

**Email-verification requirements could not be established first-party for any of
the four as of 2026-08-10.** Secondary sources describe a confirm-your-email step
at CoinMarketCap, but nothing quotable from the provider. The honest reading:
*email-based account creation is the industry floor; captcha and card
requirements are not published by any of the four.* Discord OAuth is a stronger
gate than the published floor at all four comparators.

### 4.2 The finding the epic should care about

**Our proposed free quota is generous by peer standards.** 50k–100k calls/month
against CoinGecko's "10k call credits/mo" is **5–10×**, and against
CoinMarketCap's "15,000" is **3.3–6.7×**. Two consequences:

- Option (c), lowering the free quota, has substantial headroom before we look
  stingy. Dropping to 25k/month would still be 2.5× CoinGecko's free tier — and
  would halve per-key exposure again, to ~$0.09/month.
- Conversely, a key at our limits is *worth more to an abuser* than a free key at
  any of these peers, which is the one argument for taking churn seriously even
  though the dollar exposure is small.

---

## 5. Precedent for account-age gating

The strongest first-party precedent is Discord's own server verification levels:

| Level | Value | Description (verbatim) |
|---|---|---|
| NONE | 0 | "unrestricted" |
| LOW | 1 | "must have verified email on account" |
| MEDIUM | 2 | "must be registered on Discord for longer than 5 minutes" |
| HIGH | 3 | "must be a member of the server for longer than 10 minutes" |
| VERY_HIGH | 4 | "must have a verified phone number" |

And the membership-screening signal:

> The `pending` field indicates whether "the user has not yet passed the guild's Membership Screening requirements."

> Source: [Guild Resource | Discord Developer Documentation](https://docs.discord.com/developers/resources/guild) — fetched 2026-08-10

**Read this precedent honestly — it cuts against the epic's assumption.** Discord
ships account-age gating as a first-class product feature, which establishes that
the *pattern* is legitimate and expected. But the thresholds Discord chose are
**5 minutes** and **10 minutes**. Discord's own product judgement is that account
age is a raid speed-bump measured in minutes, not a durable identity barrier. If
we adopt account-age gating we are choosing our own threshold (7 days, 30 days)
with no vendor precedent for anything above ten minutes.

### 5.1 Third-party precedent is thin — say so in the ADR

I found **no first-party published policy from any comparable API provider gating
free-tier issuance on OAuth account age**. Targeted searching returned only
age-*verification* (KYC/minor-protection) vendors, which are a different product
category entirely, and third-party blog/bot documentation rather than provider
policy. Two access failures worth recording so nobody re-runs them:

- `support.discord.com` returns **HTTP 403** to automated fetch — Discord's
  moderation help-centre articles on raid protection and verification levels
  could not be cited first-hand.
- `etherscan.io/apis` returns **HTTP 403** to automated fetch; Etherscan free-tier
  limits could not be established as a fifth comparator.

Practical consequence: account-age gating is **cheap and defensible but
unprecedented at the thresholds we would want**. Ship it as a tunable SSM
parameter starting at a low value, not as a hard-coded 30 days, and expect to
justify the number ourselves rather than cite anyone.

### 5.2 Why it is nonetheless the best-value option

The Discord snowflake in the `id` field already returned by `identify` encodes a
creation timestamp. Age gating therefore needs:

- no additional OAuth scope,
- no additional consent screen,
- no third-party vendor, contract, or secret,
- no recurring cost,
- roughly ten lines of code and one SSM parameter.

It is the only mitigation on the list with a **$0 recurring cost and no new
external dependency**.

---

## 6. Comparison table

Recurring costs assume a few hundred issuances/month and are stated per month.
Engineering cost: S ≈ under a day, M ≈ a few days, L ≈ a week-plus including
external process.

| Option | Engineering cost | Recurring cost | What it actually stops | What it does not stop |
|---|---|---|---|---|
| **(a) Guild-membership check** | **M** — new OAuth scope (`guilds.members.read`), revised consent screen, guild ID into SSM, one extra Discord API call, plus handling the not-a-member path in the UI | **$0** — Discord API, no vendor | Users who have never joined the Stellar server; surfaces `pending`, `joined_at`, `roles` for finer rules later | Nothing, if the server is open-join — one click defeats it. Value is entirely contingent on the server posture finding |
| **(b) Minimum account age (snowflake)** | **S** — decode the snowflake we already receive from `identify`, compare against an SSM threshold. No new scope, no consent change | **$0** — no external call at all | Freshly-registered throwaway accounts, at whatever threshold we pick | Aged accounts (bought, dormant, or stockpiled). No vendor precedent above Discord's own 5/10 minutes (§5) |
| **(c) Lower the free quota** | **S** — one number in the usage plan; already the mechanism the design uses | **$0**; *saves* money — 100k→50k drops per-key exposure from **$0.38** to **$0.19**; 100k→25k drops it to ~$0.09 (§1.2, §1.3) | Bounds the blast radius of every key, abusive or not, with AWS enforcing it natively and no code of ours in the path | Nothing about *who* gets a key. Also degrades the product for legitimate users — though at 25k we would still be 2.5× CoinGecko's free tier (§4.2) |
| **(d) Manual approval for first key** | **M** to build (queue, notification, approve/deny UI, audit trail), **L** to operate — it never ends | **$0** licence, but human time. At an assumed $60/h fully-loaded ($1.00/min — *assumption, not a sourced figure*), a 2-minute review is **$2.00 ≈ 5.3 fully-drained keys** (§1.2). Cost scales with *legitimate* signups; the abuse it prevents does not | A determined single abuser, once. Gives a human the chance to spot patterns | Scale. It is the only option whose cost grows with product success, and it deletes the "self-service" property the epic is named after |
| **(e) Captcha — Turnstile** | **M** — widget on the static portal, siteverify secret in SSM, one extra outbound call in the issuance backend, error handling for expired/replayed tokens (300 s TTL, single-use) | **$0** — "Unlimited challenges (traffic or verification requests)" on the Free plan; caps are 20 widgets/account and 10 hostnames/widget, neither binding for one portal (§2.1) | Scripted mass signup — *automation* of the Discord-OAuth-plus-issuance flow | A human churning Discord accounts. Adds a second lock to a door Discord OAuth already locks. Free-plan analytics lookback is 7 days, limiting retrospective abuse forensics |
| **(e′) Captcha — hCaptcha** | **M** — same shape as Turnstile | **$0** on Basic, **but the free-tier volume cap is undocumented** (§2.2). Pro is "$139/month" = **366 fully-drained keys/month** (§1.2) — cannot pay for itself | Same as Turnstile | Same as Turnstile, plus an undocumented free cap we cannot plan against |
| **(e″) Captcha — reCAPTCHA** | **M** — same shape, plus a Google Cloud project and org-level billing coordination | **$0** up to "10,000 assessments per calendar month **per organization**"; then "$8 flat fee" to 100,000; then "$1.00 per 1,000 assessments" (§2.3) | Same as Turnstile | Same as Turnstile, plus: the allowance is shared org-wide, and exhaustion returns a hard `Resource Exhausted (429)` — key issuance stops entirely |
| **(f) Email confirmation** *(not on the original list; costed for completeness)* | **L** — SES production-access request via AWS Support ("initial response … within 24 hours", approval not guaranteed), sending domain, DKIM, bounce/complaint process. Sandbox permits only "200 messages per 24-hour period" to pre-verified addresses (§3.2) | **$0.10 / 1,000 emails** à la carte ≈ **$0.10/month** at a few hundred signups (§3.1). Money is not the problem | Control of an inbox at signup time | Disposable inboxes. And it re-proves something Discord already holds — Discord's LOW verification level is literally "must have verified email on account" (§5). **Recommend against** |

### 6.1 Recommendation to the epic owner

1. **Keep the quota.** It is already the strongest control in the design: it cuts
   worst-case per-key exposure 26× versus the throttle alone (§1.2), costs
   nothing, and AWS enforces it with no code of ours in the path.
2. **Add account-age gating (b).** $0, no new scope, no new consent screen, no new
   dependency. Ship the threshold as an SSM parameter, start low, and record in
   the ADR that we chose the number ourselves — Discord's own precedent tops out
   at ten minutes (§5.1).
3. **Decide (a) on the server-posture finding, not on cost.** Its money cost is
   zero either way; its real cost is consent-screen friction, and its value is
   zero if the Stellar server is open-join. This is the other agents' finding to
   supply.
4. **Do not build (d) or (f).** Manual approval costs ~5× per legitimate signup
   what a maximally abusive key costs per month, and scales the wrong way. Email
   confirmation is the most work for the least proof and duplicates the identity
   provider.
5. **Hold (e) in reserve, and if it is ever needed, use Turnstile.** It is the
   only captcha with a published volume-unlimited free tier and a failure mode we
   control. Revisit if we ever observe *scripted* signup — the threat captcha
   actually addresses.
6. **Get the all-in per-call cost before the ADR is signed** (§1.5). Every number
   in this note is gateway-only. If backend cost dominates, the exposure figures
   move by an order of magnitude and recommendation 1 becomes more urgent, not
   less.

---

## Sources fetched (all 2026-08-10)

| URL | Used for |
|---|---|
| https://aws.amazon.com/api-gateway/pricing/ | $3.50/million REST, $0.09/GB transfer, $0.038/hr for 1.6 GB cache, tier thresholds |
| https://aws.amazon.com/ses/pricing/ | $0.10/1,000 à la carte, $0.16/1,000 Essentials, Pro $105/month minimum |
| https://docs.aws.amazon.com/ses/latest/dg/request-production-access.html | Sandbox: 200 msgs/24 h, 1 msg/s, verified recipients only; 24 h support response |
| https://developers.cloudflare.com/turnstile/plans/ | "Unlimited challenges", 20 widgets, 10 hostnames, 7-day analytics |
| https://developers.cloudflare.com/turnstile/get-started/server-side-validation/ | Siteverify mandatory, single-use tokens, 300 s expiry |
| https://www.hcaptcha.com/pricing | Basic $0, Pro $139/mo ($99 annual), 100K evals then $0.99/1K |
| https://docs.hcaptcha.com/faq/ | Confirms no published free-tier volume cap |
| https://docs.cloud.google.com/recaptcha/docs/billing-information | 10,000 free/org/month, $8 flat 10,001–100,000, $1.00/1,000 above, 429 on exhaustion |
| https://www.coingecko.com/en/api/pricing | Demo 10k credits/mo, 100 calls/min, no credit card |
| https://coinmarketcap.com/api/pricing/ | Basic 15,000 credits, 50 req/min |
| https://coinmarketcap.com/api/documentation/guides/quick-start | Signup steps; no email-confirmation statement |
| https://www.alchemy.com/pricing | Free 30M CU/month, 25 req/s |
| https://www.infura.io/pricing | Free 3M daily credits, 500 credits/s |
| https://docs.discord.com/developers/resources/guild | Verification level enum, `pending` field |
| https://blog.cloudflare.com/turnstile-ga/ | Historical context only — superseded by the plans page; do not cite its beta-era 1M figure |

**Fetch failures (do not retry blind):** `support.discord.com` → HTTP 403;
`etherscan.io/apis` and `docs.etherscan.io` → HTTP 403 / no limits published in
fetchable pages; `cloud.google.com/recaptcha/pricing` → content truncated, use
the `docs.cloud.google.com` billing page instead.

**Could not establish as of 2026-08-10:** eu-central-1 REST request pricing; the
full API Gateway cache-size × hourly-rate table; hCaptcha's free-tier volume cap;
Turnstile Enterprise pricing; first-party confirmation of email-verification
requirements at any of the four comparator APIs; our own all-in per-call backend
cost.
