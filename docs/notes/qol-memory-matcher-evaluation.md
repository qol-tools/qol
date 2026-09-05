# Memory matcher evaluation

Run `node plugins/qol-memory/scripts/evaluate.mjs compare` from the workspace.
The comparison prepares pinned public model files in the checkout's ignored
target directory, builds the workers, and writes an isolated run under
`reports/qol-memory/comparison`. `--offline` requires verified cached models.
`--model-cache PATH` changes that cache; `--repeats N` controls timed repetitions.
The command without `compare` runs the existing answer-selection regression suite.

`verify [--prepare] [--endpoint URL] [--repeats N]` runs the production-path
answer verification gate over the frozen corpora under
`tests/fixtures/answer-verification`, writing to `reports/qol-memory/verification`.
`contract [--verify [--endpoint URL]]` scores the answer-contract cases under
`tests/fixtures/answer-contract` on the deterministic path and, with `--verify`,
on the verified path, writing to `reports/qol-memory/contract`; it exits nonzero
unless a stage qualifies. Both need a local Ollama with the pinned `qwen3:8b`
digest and a free GPU: two provider stages run back to back can leave the model
on the CPU and inflate completion latency tenfold.

The model registry beside the orchestrator owns repositories, revisions, and
SHA-256 digests. Downloaded weights are data; inference runs locally. Each run
records dataset and source hashes, compiler-reported executable paths, copied
executable digests, commands, scores, timings, and the final decision. The
research worker is outside the product workspace and adds no model dependency
to the memory plugin.

The fixture files under the plugin's `tests/fixtures/matcher-comparison` own the
development and held-out splits. Facts carry recorded questions and answers;
each query identifies the answer that can be reused verbatim, or requires
abstention. An inverse yes/no question therefore needs a different answer.
A changed meaning can have a valid answer when another fact explicitly covers
it. Synthetic facts are curated evaluation fixtures, not new user memories.

The scorer sees expected answers; inference workers receive only facts and
queries. Development results choose the highest-coverage threshold with zero
wrong answers. The policy is written and frozen before held-out evaluation.
Never retune on a held-out result and continue calling that split held out.
Introduce a fresh reserved split for the next development round. This is an
agent-authored preliminary corpus, not an independent human accuracy audit.

The current candidate runs the real cold ask, warm daemon handler, and launcher
row adapter against a fresh fixture store and requires agreement. Its latency
samples include the row adapter and retrieval logging. Desktop rendering and
socket transport are outside this measurement. Guest launcher verification is
required before a production integration.

The embedding candidate compares recorded questions with normalized BGE CLS
vectors. The hybrid candidate combines lexical question retrieval and dense
ranking with reciprocal-rank fusion, then scores the shortlist with the pinned
duplicate-question classifier. Both models score questions; neither performs
answer-aware constraint verification. Their failures determine whether that
additional stage is necessary. The question corpus, model warmup, and initial
indexing are reported separately from warm query latency. Query predictions are
recomputed for each timing repetition, with no answer cache.

Retrieval recall measures whether a gold fact appears in the diagnostic
shortlist. Answer precision counts wrong accepted facts, including wrong answers
to answerable questions. Answer coverage counts correct answers among answerable
questions. Reported abstentions distinguish correct rejection from missed
answers. Empty prediction coverage never earns perfect precision. Acceptance
criteria live in the scorer and are captured before evaluating either split.

A successful workflow means the comparison ran correctly. Its decision can
still reject every candidate. The report never promotes a rejected matcher or
changes the running launcher. Latency is a local benchmark with the recorded
machine, corpus, and repetition count; it is not a large-store service guarantee.

Verification commands:

```
node --test plugins/qol-memory/scripts/comparison/*.test.mjs
cargo test --locked --manifest-path docs/research/qol-memory/tier1/Cargo.toml --all-targets --target-dir target/qol-memory-compare
cargo clippy --locked --manifest-path docs/research/qol-memory/tier1/Cargo.toml --all-targets --target-dir target/qol-memory-compare -- -D warnings
cargo run -q -p qol -- check
```

Trace decision: no new runtime trace target. The production path is unchanged;
the comparison report owns the experimental scores, abstention reasons, and
artifact evidence. Runtime trace enrichment belongs with a future integration.
