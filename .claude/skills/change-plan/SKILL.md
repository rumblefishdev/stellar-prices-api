---
name: change-plan
description: Check, raise or lower a prices-api user's usage plan (free / basic / analyst / lite / pro) by their Discord id, in AWS API Gateway on production. Use when asked e.g. "podnieś plan userowi <id>", "zmień plan na pro", "obniż do free", "na jakim planie jest user X", "upgrade/downgrade user".
argument-hint: '<discord-id> [free|basic|analyst|lite|pro] [profile=<aws-profile>]'
---

# /change-plan — Check or change a user's usage plan

An operator moves a self-service key between the five usage plans by hand in
AWS (task 0311, decision 3). **The commands live in one place only:
`docs/runbooks/manual-api-key-tier.md`.** Read it at the start of every run
and follow it. This skill adds the rules for running it from a Claude session.

## Inputs

- **Discord id** of the user (a snowflake, 17–20 digits). The key is named
  `discord-<id>-key`. A username or an e-mail is not enough; ask for the id.
- **Target tier**: exactly one of `free`, `basic`, `analyst`, `lite`, `pro`.
  Map aliases ("podstawowy" → `basic`, "najwyższy" → `pro`), but ask if the
  request is ambiguous. For a hand-made Custom/Enterprise plan, see
  "Out of scope" below.
- No target tier, only "check": run the check and stop.
- **AWS profile** (`profile=<name>`, or "profilem X", "na profilu X"): the
  developer's own profile for the shared account. Profile names differ per
  machine, so the skill hard-codes none.

## AWS profile and account

Resolve the profile in this order:

1. the profile given in the request or the arguments;
2. otherwise `$AWS_PROFILE`, if it is set in the shell. Say which one you
   are using;
3. otherwise run `aws configure list-profiles` and **ask** which one to use.
   Never pick one yourself, even if only one is listed.

Then export it and check the account before running any other command:

```bash
export AWS_PROFILE=<resolved profile> AWS_REGION=eu-central-1
aws sts get-caller-identity --query '[Account,Arn]' --output text
```

- The account must be **`750702271865`**. On any other account, or on a
  credentials error (expired SSO: tell the developer to run
  `aws sso login --profile <profile>` themselves), stop and report. Do not
  try another profile on your own.
- Use the same `AWS_PROFILE` in every command of the run. The Bash tool does
  not keep variables between calls, so repeat the `export` in each call.
- A read-only profile can run the check (step 1). A move (step 5) needs
  write access to API Gateway usage plan keys. On `AccessDenied`, stop and
  report which call was refused. Nothing needs undoing if the **delete** was
  refused. If the **create** was refused, the key is on no plan: follow the
  runbook's "If the create is refused or fails" with a profile that has
  access, or ask the user to sign in (the portal re-attaches the key).

## Steps

1. **Check (read-only; run it without asking).** Run the runbook's
   "Check a user's plan" section. Report every `discord-<id>-key` record
   (id, enabled, last updated) and the plan of each on our stage.
2. **Pick the key(s) to move** by the runbook's "Upgrade a user" step 1:
   - one enabled record: that one;
   - two enabled records: stop and follow the runbook (the user signs in
     once, then check again);
   - only disabled records (the user is waiting on a rework): the newest one.
     **For a downgrade, every disabled record.**
   - no records: stop. The user has to sign in to the portal first.
3. **Resolve `FROM` / `TO`** with the runbook's step 2, including the `ok_id`
   guard. Stop on anything but `ready`. If `FROM` equals `TO`, report that
   there is nothing to do.
4. **STOP and ask for confirmation.** Show the AWS profile, the Discord id, key id,
   `FROM` plan name and id, and `TO` plan name and id. Say that the key
   answers `403` for a few seconds during the move and that its usage counter
   restarts from zero on the new plan. Mutate nothing until the user answers
   yes to **this** move. Approval of an earlier move does not carry over.
5. **Move** with the runbook's step 3 block, pasted as-is: the guarded
   `delete-usage-plan-key && create-usage-plan-key`. If the create fails,
   follow "If the create is refused or fails". Do not improvise.
6. **Verify** with the runbook's step 4: exactly the target plan on our stage,
   never two, never none. Then re-run the check from step 1.
7. **Report** what the user will see: the dashboard shows the new plan within
   60 s, the counter starts from zero, the key value is unchanged, and a
   rework keeps the plan.
8. **Record.** Propose the row(s) for "Plan changes of self-service keys"
   (Discord id, key id, `from → to`, date, `By` = the git user, note). Add
   them to the runbook, but **do not commit or push without an explicit
   go-ahead for that commit**. Say that the table is the move's only record.

## Rules

- Production only; there is no dev environment (`infra/envs/` holds only
  `production.json`).
- Never edit, rename or delete any of the five CDK plans (`update-usage-plan`,
  `delete-usage-plan`) to "fix" a user. The next deploy reverts it. A limit
  change for everyone is a CDK change in `infra/envs/production.json`.
- Never read or print a key **value**: no `--include-value(s)`, no
  `get-usage-plan-keys` (it returns plaintext values).
- Never `items[0]` a key lookup; always list and choose by the runbook's rules.
- Only `discord-<id>-key` keys. A `prices-production-<customer>-key-…` key is
  a hand-made key: follow the runbook's Custom section and ask first.

## Out of scope (point to the runbook, ask before acting)

- Custom/Enterprise limits (a new hand-made plan): runbook,
  "Custom / Enterprise plans".
- Suspending or revoking a key, or rotating a hand-made key: same section.
- Raising the stage throttle ceiling: a CDK change and a capacity decision.
