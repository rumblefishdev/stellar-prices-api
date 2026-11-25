import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import type { Construct } from 'constructs';

/**
 * CloudWatch caps `AlarmDescription` at 1024 characters, and **nothing local
 * enforces it**: `cdk synth` renders an over-long description happily, the
 * template is valid CloudFormation, and the request is only rejected by the
 * CloudWatch API mid-deploy — after some alarms in the stack have already been
 * created (task 0204, 2026-08-20; three `usd-stranded` rungs at ~1250 chars).
 *
 * ⚠️ The alarms in ObservabilityStack carry deliberately long, runbook-style
 * descriptions, because an operator reading Slack at 03:00 has nothing else.
 * That is worth keeping — but it means this ceiling will be hit again, and a
 * failure discovered at deploy time is the most expensive place to discover it.
 *
 * So the check runs at synth: a walk of the construct tree that throws with the
 * offending alarm and its length. It walks the whole `App` (called from
 * `app.ts`), so EventBridgeStack's per-worker `-errors` alarms
 * (`createWorkerLambda`) and any stack added later are measured too — until
 * task 0288 only ObservabilityStack was walked. It reads the resolved CloudFormation property
 * rather than the constructor argument, so descriptions built from tokens or
 * `Fn::Join` are measured as CloudWatch will actually see them.
 */
export function assertAlarmDescriptionsFitCloudWatch(scope: Construct): void {
  const MAX = 1024;
  const tooLong = scope.node
    .findAll()
    .filter((c): c is cloudwatch.Alarm => c instanceof cloudwatch.Alarm)
    .map((alarm) => {
      const cfn = alarm.node.defaultChild as cloudwatch.CfnAlarm;
      const description = cdk.Stack.of(alarm).resolve(
        cfn.alarmDescription,
      ) as unknown;
      const length = typeof description === 'string' ? description.length : 0;
      return { name: cfn.alarmName, length };
    })
    .filter((a) => a.length > MAX);

  if (tooLong.length > 0) {
    const detail = tooLong
      .map((a) => `  ${String(a.name)} — ${a.length} chars`)
      .join('\n');
    throw new Error(
      `${tooLong.length} CloudWatch alarm description(s) exceed the ${MAX}-character ` +
        `API limit and would fail mid-deploy:\n${detail}\n` +
        'Shorten the description(s); the limit is enforced by CloudWatch, not by synth.',
    );
  }
}
