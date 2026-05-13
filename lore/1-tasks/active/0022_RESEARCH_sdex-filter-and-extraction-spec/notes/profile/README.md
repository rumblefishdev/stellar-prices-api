# SDEX profile harness — task 0022

Standalone Rust crate that decodes Galexie-style zstd XDR ledger files
(`.xdr.zst`), walks `tx_processing[].result.result.result.results[]`
for SDEX-shaped operation results, and reports:

- per-ledger decode time
- trade-bearing ledger density
- ClaimAtom counts + variant breakdown
- transaction success/failure rate

Inputs come from `.temp/` (gitignored Galexie-format mainnet samples).
Results are captured to [`results.md`](./results.md).

## Build

```sh
cd lore/1-tasks/active/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/profile
cargo build --release
```

The crate is standalone — it is **not** a workspace member of the
main `stellar-prices-api` project (which is currently a TS/Nx
skeleton; the Rust impl lands in task 0012).

## Run

Profile (timing + density):

```sh
./target/release/profile <dir-of-xdr-zst-files> [max_files]
# e.g.
./target/release/profile .temp/FC47D9FF--62400000-62463999 2000
```

Dump one worked example per ClaimAtom variant:

```sh
./target/release/dump-examples <dir-of-xdr-zst-files> <output-dir> [max_files]
# e.g.
./target/release/dump-examples .temp/FC47D9FF--62400000-62463999 ./examples
```

Outputs `examples/{order_book,liquidity_pool,v0}.json` for whichever
variants are found. V0 is only present in pre-protocol-18 history
(approx. pre-Aug 2022); modern samples will not produce it.

## Code layout

- `src/lib.rs` — shared decode + claim-walk helpers
- `src/bin/profile.rs` — timing + density binary
- `src/bin/dump_examples.rs` — one-per-variant example extractor
- `examples/` — captured JSON examples from sample data
- `results.md` — captured profile output

`target/` is gitignored.
