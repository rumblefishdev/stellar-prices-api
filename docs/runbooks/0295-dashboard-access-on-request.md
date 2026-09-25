# Runbook: read-only dashboard access for an external reviewer — on request

Task [0295](../../lore/1-tasks/active/0295_FEATURE_read-only-dashboard-access-for-the-stellar-team.md).
Tranche 3 AC 8 reads _"CloudWatch dashboard accessible to Stellar team
(read-only IAM role); all alarms OK."_ The dashboard is
`prices-production-overview` in `eu-central-1`. **No standing identity exists
for it, on purpose.** The account is shared with the Soroban Block Explorer and
is otherwise SSO-only; a credential nobody asked for is a liability, not a
deliverable. Access is created for a named person when they ask, with MFA, and
removed when the review is over — the same model the block explorer recorded
for its D3 AC 3 (explorer task 0129: "available on request").

History: task 0125 shipped `prices-production-stellar-viewer` (IAM user, scoped
inline read policy) on 2026-09-03 and its console login on 2026-09-04. The
login was deleted on 2026-09-25 and the user removed from the Observability
stack the same day (this task). The policy it carried is the template below.

## 1. What a request must contain

Access is granted only to a **named person**, never to a team address:

| Field               | Why                                                                   |
| ------------------- | --------------------------------------------------------------------- |
| First name, surname | the IAM user name is derived from it and the grant is logged under it |
| E-mail              | where the one-time password goes; must be the requester's own         |
| Purpose and window  | "Tranche 3 review", with an end date — the user is deleted after it   |

**MFA is mandatory.** The policy below denies every CloudWatch read unless the
session is MFA-authenticated, so a password alone opens nothing but the MFA
setup page. Record the request (who, when, until when) in task 0295's history
before creating anything.

## 2. Create the user — operator, `AWS_PROFILE=soroban-admin`

```bash
export AWS_PROFILE=soroban-admin
aws sts get-caller-identity --query Account --output text      # must print 750702271865 — stop otherwise
export VIEWER=prices-production-viewer-<surname>          # lowercase, ASCII
PW=$(openssl rand -base64 24)                              # shown once at the end; never in a log, a ticket or a task file
aws iam create-user --user-name "$VIEWER" \
  --tags Key=Project,Value=stellar-prices-api Key=Purpose,Value=tranche3-review Key=Requester,Value="<name surname>"
aws iam put-user-policy --user-name "$VIEWER" --policy-name prices-production-dashboard-read \
  --policy-document file://dashboard-read.json
aws iam create-login-profile --user-name "$VIEWER" --password "$PW" --password-reset-required
echo "$PW"; unset PW    # copy it into the second channel of §3, then `clear` the terminal
```

The first two lines exist because a block pasted without an exported profile
runs against the shell's default credentials: on 2026-09-25 that created the
user in the operator's personal account, and the sign-in at this account's URL
failed with "Authentication failed".

`dashboard-read.json` — the scoped policy task 0125 wrote (deep-review CR-01:
**not** `CloudWatchReadOnlyAccess`, which also grants `logs:*` and `xray:Get*`
across the shared account), plus the MFA condition and the self-service
statements the console needs to let the person enrol a device:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "ReadDashboardsMetricsAndAlarmsWithMfa",
      "Effect": "Allow",
      "Action": [
        "cloudwatch:GetDashboard",
        "cloudwatch:ListDashboards",
        "cloudwatch:GetMetricData",
        "cloudwatch:GetMetricStatistics",
        "cloudwatch:ListMetrics",
        "cloudwatch:GetMetricWidgetImage",
        "cloudwatch:DescribeAlarms",
        "cloudwatch:DescribeAlarmHistory",
        "cloudwatch:DescribeAlarmsForMetric"
      ],
      "Resource": "*",
      "Condition": { "Bool": { "aws:MultiFactorAuthPresent": "true" } }
    },
    {
      "Sid": "SelfServiceMfaAndPassword",
      "Effect": "Allow",
      "Action": [
        "iam:GetUser",
        "iam:ChangePassword",
        "iam:GetLoginProfile",
        "iam:ListMFADevices",
        "iam:CreateVirtualMFADevice",
        "iam:EnableMFADevice",
        "iam:ResyncMFADevice",
        "iam:ListVirtualMFADevices"
      ],
      "Resource": [
        "arn:aws:iam::750702271865:user/${aws:username}",
        "arn:aws:iam::750702271865:mfa/${aws:username}",
        "arn:aws:iam::750702271865:mfa/*"
      ]
    }
  ]
}
```

The CloudWatch read actions do not support resource-level scoping, hence
`"Resource": "*"` on the first statement — bounded by the action list (read
only, CloudWatch only) and by the MFA condition. This is the one place the
project writes an IAM policy outside CDK; it is deliberately not in the
template so that no deploy can recreate a standing identity.

## 3. Hand over

Send, **separately** (two channels — e-mail and a message, never both in one):
the console sign-in URL `https://750702271865.signin.aws.amazon.com/console`,
the user name, and the one-time password. The first sign-in forces a password
change and the MFA enrolment; until MFA is enrolled every dashboard call is
denied by the policy above.

Then the dashboard URL:
`https://eu-central-1.console.aws.amazon.com/cloudwatch/home?region=eu-central-1#dashboards:name=prices-production-overview`

## 4. Verify as the reviewer would

Before the hand-over, the operator can check the policy headlessly — the IAM
simulator honours the MFA condition and the `${aws:username}` variable:

```bash
ARN=arn:aws:iam::750702271865:user/$VIEWER
for mfa in true false; do
  aws iam simulate-principal-policy --policy-source-arn $ARN \
    --action-names cloudwatch:GetDashboard cloudwatch:DescribeAlarms logs:GetLogEvents secretsmanager:GetSecretValue \
    --context-entries "ContextKeyName=aws:MultiFactorAuthPresent,ContextKeyValues=$mfa,ContextKeyType=boolean" \
    --query 'EvaluationResults[].[EvalActionName,EvalDecision]' --output text
done
```

Expected: the two `cloudwatch:` actions `allowed` only with `true`; `logs:` and
`secretsmanager:` `implicitDeny` both times. (Walked 2026-09-25 on a throwaway
name — task 0295.)

From a browser that is not signed in to anything else: sign in, enrol MFA, open
the dashboard URL — every widget renders. Then confirm the boundary:
`https://eu-central-1.console.aws.amazon.com/cloudwatch/home?region=eu-central-1#logsV2:log-groups`
answers with an access-denied banner, and the explorer's dashboard
`production-soroban-explorer` opens too (the read actions cannot be scoped per
dashboard — say so in the evidence rather than pretend otherwise).

## 5. Remove after the review

```bash
aws iam delete-login-profile --user-name "$VIEWER"
for m in $(aws iam list-mfa-devices --user-name "$VIEWER" --query 'MFADevices[].SerialNumber' --output text); do
  aws iam deactivate-mfa-device --user-name "$VIEWER" --serial-number "$m"
  case "$m" in *:mfa/*) aws iam delete-virtual-mfa-device --serial-number "$m";; esac   # a passkey (…:u2f/…) has nothing to delete
done
aws iam delete-user-policy --user-name "$VIEWER" --policy-name prices-production-dashboard-read
aws iam delete-user --user-name "$VIEWER"
```

Record the removal date in task 0295. `aws iam list-users` should list no
`prices-*` user between reviews.

## 6. What the evidence package says

`docs/scf/milestone-3-evidence.md` AC 8: the dashboard and the alarm state are
the deliverable; access is "available on request to a named reviewer, MFA
enforced" — and `milestone-3-rfp-deviations.md` §4 declares the substitution
(an on-request user instead of a standing IAM role) with this runbook as the
mechanism.
