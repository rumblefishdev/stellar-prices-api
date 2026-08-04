# SCF Submission — Documents & Build Pipeline

This directory holds the Stellar Community Fund **Deliverable Verification**
package for the Stellar Prices API, plus the reproducible toolchain that
builds it.

The structure mirrors the sibling Soroban Block Explorer repository's
`docs/scf/`, which has shipped two SCF submissions using it.

## File inventory

| File                                  | Role                                                                                                        |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `milestone-1-form-answers.md`         | Exact text to paste into each field of the SCF Deliverable Verification web form.                           |
| `milestone-1-evidence.md`             | Full evidence companion to the M1 submission video — **source of truth**.                                   |
| `milestone-1-evidence.pdf`            | PDF render of the above, attached in Google Drive next to the video. **Build output.**                      |
| `milestone-1-video-scenario.md`       | Scene-by-scene recording script for the 5–7 minute demo video.                                              |
| `api-endpoints.md`                    | Live API surface: every mapped route, its auth posture, and its cache TTL. Kept current as M2 lands.        |
| `ch-demo-queries.sql`                 | Read-only ClickHouse queries the operator runs against production; outputs feed both the video and the PDF. |
| `architecture.mmd`                    | Mermaid source for the architecture diagram.                                                                |
| `architecture.png`                    | Rendered diagram, embedded in the evidence PDF. **Build output.**                                           |
| `build-pdf.sh`                        | Reproducible PDF render script (`pandoc + typst`).                                                          |
| `screenshots/`                        | Evidence images referenced from `milestone-1-evidence.md`.                                                  |
| `header.typ`, `full-width-tables.lua` | Typst/pandoc styling for the PDF render.                                                                    |

## Build the PDF

One-time tooling install:

```bash
# Linux (Debian/Ubuntu)
sudo apt install pandoc poppler-utils      # poppler-utils optional (page count)
cargo install --locked typst-cli           # or: snap install typst

# macOS
brew install pandoc typst poppler
```

`pandoc` must be **≥ 3.1** for `--pdf-engine=typst`; distro packages are often
older. If yours is, install from https://github.com/jgm/pandoc/releases. The
build script checks this and fails early with a clear message.

Render:

```bash
./build-pdf.sh
```

Output: `milestone-1-evidence.pdf`. The script also lists any unresolved
`<TODO:>` markers in the source so you remember to fill in query outputs and
screenshots before the final upload.

## Render the architecture diagram

```bash
npx -y @mermaid-js/mermaid-cli -i architecture.mmd -o architecture.png -s 3 -p puppeteer-config.json
```

`-s 3` renders at 3× scale so the diagram stays legible in the PDF.
`-p puppeteer-config.json` launches headless Chromium with `--no-sandbox`,
required on Ubuntu 23.10+ where AppArmor blocks unprivileged user namespaces
(otherwise the render fails with "No usable sandbox!").

### Why this toolchain

- **Typst** as the engine: handles Unicode (`→`, `—`, `✅`, en-dashes) natively
  — no LaTeX font fiddling.
- **GFM input mode**: gives GitHub-style heading auto-IDs so the in-doc anchor
  links resolve.
- ~10× faster than xelatex; cold render under 2 s.

## Submission workflow (end-to-end)

```
0. Sanity-check the coarse tips are current. The six rollup MVs run in APPEND
   mode (task 0095, deployed 2026-07-17), so 1h/4h/1d/1w/1M advance on their own
   — no manual pre-roll needed. Just confirm the tips track the live frontier
   before recording, so the PDF and video show current numbers:

       SELECT max(timestamp) FROM prices.price_ohlcv_1d;   -- today
       SELECT max(timestamp) FROM prices.price_ohlcv_15m;  -- within ~15 min

   (If a tip ever lags — e.g. after cluster downtime — schema/preroll-live-gap.sql
   still closes a gap, but under normal operation the MVs keep coarse current.)

1. Run  ch-demo-queries.sql  against production ClickHouse over mTLS.
   Paste each output into milestone-1-evidence.md, replacing its
   <TODO: paste output> marker.

2. Capture the screenshots for every <TODO: screenshot> marker — ideally in
   the same session as the video recording, so the PDF and the video show the
   same numbers:
       - `make synth-production` + the no-RDS/VPC/NAT grep
       - CloudWatch alarms list in OK state
       - the Slack alarm notification from the task 0056 fire-test

3. Replace each <TODO:> with a Markdown image embed:
       ![CloudWatch alarms in OK state](./screenshots/ac5-alarms-ok.png)

4. Render the diagram, then run  ./build-pdf.sh  to regenerate the PDF.

5. Record the video from  milestone-1-video-scenario.md  (5–7 min).

6. Upload the PDF + video to a Google Drive folder with link-sharing set to
   "anyone with the link can view".

7. Open  milestone-1-form-answers.md , replace every <ANGLE_BRACKET>
   placeholder (Drive folder link, video URL), and work the
   pre-submission checklist at the bottom.

8. Copy the four field blocks into the SCF Deliverable Verification form
   and submit.
```

## Ground rules for this package

These are not style preferences; they are what keeps the submission
defensible.

- **Claim only what is demonstrated.** Milestone 1 is Infrastructure &
  Real-time Ingestion. The full API surface, the dashboard, and full-chain
  backfill coverage are later tranches — `milestone-1-evidence.md` §6 says so
  explicitly, and Field 1 of the form answers matches it exactly.
- **Never screenshot the CloudWatch dashboard.** `prices-production-overview`
  is a scaffold with no data widgets. The seven alarms are real and
  fire-tested; the dashboard is not evidence.
- **The six rollup MVs are present and running in APPEND mode.** `SHOW TABLES`
  WILL list `mv_ohlcv_1m_to_15m … mv_ohlcv_1w_to_1M`, and a reviewer running that
  query must find exactly that. They were briefly dropped after a replace-mode
  incident (they overwrote pre-rolled coarse history); task 0095 recreated them
  in APPEND mode on 2026-07-17, so they now roll live candles forward without
  clobbering history. The package describes them as present — do not revert the
  text to the old "dropped" wording.
- **History lives in the coarse tables, not `price_ohlcv_1m`.** `1m` is a
  transient feeder pruned at 7 days (`15m` at 30); `1h/4h/1d/1w/1M` are kept
  forever. Any depth-of-history claim or query must read the coarse tables —
  asking `1m` for six months returns a few days and reads as a failure.
- **Scope refinements get disclosed, not buried.** Every deviation from the
  approved plan (RDS → ClickHouse foremost) is named, rationalised, and linked
  to the ADR that records it, in both the PDF and the video.
- **No secrets in any artifact.** No API keys, no certificate material, no
  private keys — in the markdown, the PDF, the screenshots, or any video
  frame. Export `$KEY` outside the recorded shell.
- **No personal names**, no Polish, no internal slang. English-only.

## Known limitations of the current render

- **Long URLs in tables** (§7 "Live endpoints and access") wrap mid-domain.
  Acceptable for a reviewer, not pretty. Fix would be a small typst template;
  not blocking.
- **`<TODO:>` markers** render as inline text until replaced. They stay
  intentionally visible as placeholders.

## Regenerating from scratch

The PDF and PNG are build artifacts, reproducible from `.md` / `.mmd` source.
If the PDF drifts out of sync (someone edits the markdown and forgets to
re-render), just run `./build-pdf.sh` again.

The PDF is **committed** — for reviewer ease, the repo ships the same artifact
the SCF reviewer sees. To keep the repo source-only instead, add
`docs/scf/*.pdf` to `.gitignore`.
