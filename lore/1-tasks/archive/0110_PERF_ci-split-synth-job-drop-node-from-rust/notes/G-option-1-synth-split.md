---
title: "Option 1: split synth into its own job — complete implementation, not shipped"
type: generation
status: superseded
spawned_from: ../README.md
spawns: []
tags: [ci, cdk, synth, github-actions, artifact, wont-do, reference-only]
links: []
history:
  - date: 2026-08-04
    status: mature
    who: akot
    note: >
      Written before the A/B measurement landed. Complete and internally
      consistent, but never executed by CI.
  - date: 2026-08-05
    status: superseded
    who: akot
    note: >
      Superseded by the measurement in the parent task — the split makes CI
      slower. Moved here from a local `git stash` after PR #165 review finding
      3: a stash lives in one working copy, is not on a branch, is not pushed,
      and dies with `git stash clear` or a fresh clone. The parent task and
      `lore/3-wiki/project/ci-pipeline.md` both pointed readers at it.
---

# Option 1 — separate `synth` job

**Not shipped, and should not be shipped on today's numbers.** Task 0110
measured the trade and closed won't-do: the Node/TS tail this removes costs 29s,
of which only ~20s leaves the `rust` job, while this job costs ~50-70s
serialized after it. See the parent task's [Measurement](../README.md#measurement).

This is kept so that *if* the arithmetic ever changes — realistically only if
`Build Lambda bootstraps` (3m24s, 67% of the job) shrinks by an order of
magnitude — the code is recovered rather than rewritten. It was never run by
CI, so treat it as a reviewed draft, not a tested change.

## What it does

1. Adds an `Upload Lambda bootstraps` step to the end of the `rust` job
   (`path: target/lambda/*/bootstrap`, `retention-days: 1`,
   `if-no-files-found: error`).
2. Adds a `synth` job on `ubuntu-latest` (x86 — it stages and hashes the ARM
   binaries, never executes them) that downloads the artifact and runs
   `make -C infra synth-production`.
3. Moves the synth step and its `setup-node` / `npm ci` prerequisites out of
   the `rust` job.

## The two traps it already handles

- **Zip does not carry the Unix executable bit.** `actions/download-artifact`
  therefore yields non-executable bootstraps, and the verify loop tests `-x`.
  Handled with an explicit `chmod +x target/lambda/*/bootstrap` after download
  — restoring the mode rather than weakening the check to `-f`.
- **`needs: rust` alone does not skip this job correctly.** A job that declares
  its own `if:` has that condition evaluated before the skipped-dependency
  inference, so the `rust` paths-filter guard has to be repeated on the `synth`
  job. Without it, an infra-only PR tries to synth with no artifact to download.

## Not handled

The artifact upload step's own cost is not in the parent task's model and was
never measured — ~110 MB on the critical path between two jobs.

## The patch

Applies to `.github/workflows/ci.yml` at blob `91e84b7` — the pre-0110 state,
which is the current state modulo the pointer comment 0110 added.

```diff
diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index 91e84b7..39e0d65 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -182,11 +182,89 @@ jobs:
           echo "verified ${checked} bootstrap(s)"
           exit $missing
 
+      # Hand the bootstraps to the `synth` job. Synth needs these files and
+      # only this job can produce them, which is why synth used to live here —
+      # at the cost of installing Node and building TypeScript on the ARM Rust
+      # runner (task 0110). An artifact is the seam that lets synth move out.
+      #
+      # Upload the binaries only, not all of `target/lambda/**`: cargo-lambda
+      # also leaves intermediate output there, and the artifact is on the
+      # critical path to the synth job.
+      #
+      # retention-days: 1 — this artifact is scaffolding between two jobs of
+      # one run, never something an operator downloads later. The default 90
+      # days would keep ~110 MB per CI run alive for a quarter.
+      - name: Upload Lambda bootstraps
+        uses: actions/upload-artifact@v4
+        with:
+          name: lambda-bootstraps
+          path: target/lambda/*/bootstrap
+          retention-days: 1
+          if-no-files-found: error
+
+  synth:
+    name: Synth (CDK production app)
+    needs: [changes, rust]
+    # `needs: rust` alone is not enough. When the rust job is skipped, a
+    # dependent job is skipped too — but `if:` runs before that inference on a
+    # job that declares its own condition, so the rust guard has to be repeated
+    # here. Without it an infra-only PR would try to synth with no artifact to
+    # download.
+    if: |
+      needs.changes.outputs.rust == 'true' ||
+      (github.event_name == 'push' && github.ref == 'refs/heads/master')
+    # Plain x86 runner: this job stages and hashes the ARM binaries, it never
+    # executes them, so it does not need the ARM host the build required.
+    runs-on: ubuntu-latest
+    steps:
+      - uses: actions/checkout@v4
+
+      - uses: actions/download-artifact@v4
+        with:
+          name: lambda-bootstraps
+          path: target/lambda
+
+      # GitHub artifacts travel as a zip, and zip does not carry the Unix
+      # executable bit. The verify step below tests `-x`, and a deployed
+      # bootstrap must be executable, so restore the mode rather than weaken
+      # the check to `-f`.
+      - name: Restore executable bit on bootstraps
+        run: chmod +x target/lambda/*/bootstrap
+
+      # Re-run the same assertion the rust job ran, now against the downloaded
+      # copy. The rust job proved the binaries were BUILT; this proves they
+      # ARRIVED — a partial upload or a path change would otherwise surface as
+      # a confusing CDK asset error further down.
+      - name: Verify Lambda artifacts arrived
+        shell: bash
+        run: |
+          set -euo pipefail
+          echo "=== Lambda bootstrap binaries (downloaded) ==="
+          missing=0
+          checked=0
+          while IFS= read -r name; do
+            [[ -z "$name" ]] && continue
+            checked=$((checked + 1))
+            bin="target/lambda/${name}/bootstrap"
+            if [[ -x "$bin" ]]; then
+              echo "$(sha256sum "$bin") $(stat --format='%s bytes' "$bin")"
+            else
+              echo "::error::missing Lambda bootstrap after download: $bin"
+              missing=1
+            fi
+          done < <(tools/scripts/lambda-assets.sh)
+          if [[ $checked -eq 0 ]]; then
+            echo "::error::verified 0 Lambda bootstraps; the guard passed vacuously" >&2
+            exit 1
+          fi
+          echo "verified ${checked} bootstrap(s)"
+          exit $missing
+
       # Final proof that the app the operator deploys actually synthesizes
-      # with the assets this job just built. The build+verify steps above
-      # guarantee the SET matches; synth additionally catches stack-level
-      # errors and any asset an operator would only discover mid-deploy
-      # (which is how 0070's CannotFindAsset surfaced).
+      # with the assets the rust job built. The build+verify steps guarantee
+      # the SET matches; synth additionally catches stack-level errors and any
+      # asset an operator would only discover mid-deploy (which is how 0070's
+      # CannotFindAsset surfaced).
       #
       # Credential-free: every SSM read in the stacks is
       # `valueForStringParameter`, which emits a CloudFormation dynamic
```
