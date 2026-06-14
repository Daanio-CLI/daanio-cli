# Plan: Making the Memory Graph Earn Its Keep

> Status: Proposal. Companion to MEMORY_ARCHITECTURE.md. Goal: best recall accuracy.

## Current reality (verified in code)

- **Live automatic recall** (`memory_agent::process_context`) uses
  `find_similar_with_embedding` -> `score_and_filter`: flat cosine over all
  active memories, threshold 0.5, gap filter, top 10, then per-candidate binary
  sidecar relevance, max 5 surfaced.
- The **graph traversal** (`cascade_retrieve`, BFS over tags/clusters/RelatesTo)
  is only called by the manual `memory { action: search }` tool
  (`tool/memory.rs:216`). It contributes **nothing** to per-turn surfacing.
- Maintenance (`post_retrieval_maintenance`) WRITES graph structure every turn
  (RelatesTo links, auto-clusters + sidecar naming, inferred tags, confidence
  boost/decay, pruning) but the live path never READS most of it back.

So today the graph's edges/clusters/tags are write-mostly. The parts that
matter (supersede/contradiction -> `active`, reinforcement strength, confidence
gating the active set) help data hygiene, not ranking.

## Goal: maximize recall accuracy in BOTH modes

Both modes are first-class targets. They share Stage 1 (candidate generation)
and the graph layer, but diverge at the quality/judgment stage. Strategy:
push as much accuracy as possible into the **shared local stack** (better
embedder, hybrid, query construction, graph rerank, priors) so Mode 1 gets
strong on its own, then let Mode 2's LLM add a final precision layer on top of
an already-good candidate set rather than compensating for a weak one.

### Shared local stack (lifts both modes)
- recall-2 better embedder + asymmetric prefixes
- recall-3 focused query construction (current intent, not 8k blob)
- recall-4 hybrid dense + BM25 + RRF
- recall-6 recency/confidence/strength/scope priors in the score
- Phases A-C graph: supersede-authoritative, 1-hop expansion, dedup
- A local cross-encoder reranker (small, on-device) as the Mode 1 top stage

### Mode 1 (no LLM) - get it as close to Mode 2 as possible
- Replace raw-cosine top-5 with: hybrid recall -> graph expansion -> local
  cross-encoder rerank -> priors -> calibrated cutoff. No LLM needed for any of
  this; a cross-encoder is the single biggest precision lever available offline.
- Tune the calibrated cutoff per-mode (Mode 1 can afford slightly higher recall
  since there's no LLM filter downstream; rely on the cross-encoder for precision).
- Optional: cheap local query expansion (synonyms/identifier splitting) since
  there's no LLM to do query rewriting.

### Mode 2 (LLM) - add precision, don't redo recall
- Feed the SAME strong candidate set (hybrid + graph + cross-encoder top ~15-20)
  into one **listwise** LLM rerank, replacing today's independent binary calls
  (binary calls can't compare candidates and waste the LLM's judgment).
- Use the LLM for query rewriting / HyDE at Stage 0 to fix hard recall misses
  that the local stack can't reach.
- Keep the LLM as the final arbiter for contradictions and ambiguous relevance.

### Why this converges both
Mode 1 ceiling rises to "best offline retriever + cross-encoder" (very high).
Mode 2 starts from that same ceiling and adds LLM rewriting + listwise judgment,
so it's strictly >= Mode 1. The harness (recall-1) tracks both columns each
change so neither regresses.

## Two operating modes (gate: `agents.memory_sidecar_enabled`, env `JCODE_MEMORY_SIDECAR_ENABLED`, default off)

The system runs in two distinct modes; recall behavior differs substantially.

### Mode 1 - Sidecar OFF (embedding-only, no LLM)
- Surfacing (`evaluate_candidates`): skips LLM, takes top candidates by **raw
  cosine** (up to 5). No relevance verification, no listwise judgment.
- Extraction: `extract_from_context` + final extraction are **skipped**. Memories
  are created ONLY via the explicit `memory` tool, not auto-learned.
- Cluster naming: falls back to `infer_candidate_tag` (heuristic, no LLM).
- Net: fully local, zero LLM cost; weakest precision (no filtering) and no auto
  memory growth.

### Mode 2 - Sidecar ON (LLM-assisted)
- Surfacing: per-candidate binary relevance check by sidecar LLM (parallel, max 5).
- Extraction: auto-extract on topic change + every 12 turns + session end, with
  LLM dedup/contradiction checks.
- Maintenance: LLM-named clusters, contradiction detection.

### Implications for this plan
- Every recall improvement must be evaluated in BOTH modes (recall-1 harness
  should report two columns).
- Phases A-C (graph as reranker/dedup/expansion) are **pure local** and benefit
  Mode 1 the most, since Mode 1 currently has no quality layer beyond cosine.
- recall-5 (rerank) has two implementations: a local cross-encoder path for
  Mode 1, and the listwise LLM rerank for Mode 2 (replacing today's binary calls).
- Phase D maintenance trimming primarily affects Mode 2 cost (cluster naming is
  the LLM line item); Mode 1 already uses the heuristic fallback.

## Edge types and what each is good for

| Edge          | Source of truth | Use in recall |
|---------------|-----------------|----------------|
| `Supersedes`  | contradiction/dedup on write | Keep ONLY newest version in results; demote/hide superseded |
| `Contradicts` | sidecar on write | Surface both + flag conflict; never silently pick one |
| `RelatesTo`   | co-relevance maintenance | 1-hop expansion to rescue near-misses |
| `DerivedFrom` | co-extraction | 1-hop expansion (procedures <-> facts) |
| `HasTag`      | user + inference | Lexical/filter signal, scope narrowing |
| `InCluster`   | auto clustering | Weakest; diversity/dedup at best |

## Design principle

Use the graph as a **structural reranker / recall-rescue layer**, NOT as the
primary retriever. Embeddings (+ future hybrid) generate candidates; the graph
re-scores and expands them. This is where graphs reliably help in RAG: relating,
deduping, and rescuing, not first-stage recall.

## Target live pipeline

```
Stage 1  Candidate generation (existing + future hybrid)
          dense cosine (and later BM25 + RRF), generous top-N (~40)

Stage 2  Graph expansion (NEW, 1-hop only)
          for each seed, pull neighbors via Supersedes / RelatesTo / DerivedFrom
          score_neighbor = seed_score * edge_weight * depth_decay
          this rescues relevant memories that embedding alone missed

Stage 3  Graph-aware dedup/canonicalize (NEW)
          collapse Supersedes chains -> keep newest active only
          group near-duplicate cluster members -> representative + count

Stage 4  Rerank + priors (ties into recall-5 / recall-6)
          listwise rerank, then fold confidence / strength / recency / scope
          apply calibrated cutoff
```

## Phased plan

### Phase A - Make supersede/contradiction authoritative in live recall (cheap, high value)
- In `score_and_filter` / `process_context`, post-filter results through the
  graph: drop any memory whose `superseded_by` is set or that has an incoming
  `Supersedes` edge from an active memory.
- Surface `Contradicts` pairs together with a conflict flag instead of letting
  raw cosine arbitrarily pick one.
- Verifiable: unit test with a superseded chain; assert only newest surfaces.

### Phase B - Wire 1-hop graph expansion into the live path
- Add a `cascade=true` mode to the live retrieval (reuse `cascade_retrieve` but
  cap depth=1 and restrict edges to Supersedes/RelatesTo/DerivedFrom; exclude
  InCluster/HasTag fan-out which explode candidate count).
- Feed expanded set into the reranker, not directly to output.
- Verifiable (needs recall-1 harness): recall@5 with vs without expansion.

### Phase C - Graph-aware dedup before surfacing
- Collapse Supersedes/near-dup cluster members so the 5 surfaced slots aren't
  wasted on restatements of one fact. Improves effective precision and recall.

### Phase D - Decide the fate of expensive maintenance
- Auto-clustering + sidecar cluster-naming + tag inference currently cost LLM
  calls + full graph save per cycle and feed nothing into live recall.
- Options:
  1. Repurpose clusters for Phase C dedup/diversity (keeps them, drops naming).
  2. Cut cluster-naming + tag-inference entirely, redirect budget to embedder
     upgrade + hybrid + rerank (recall-2/4/5).
- Recommended: cut naming + tag-inference now; keep cluster centroids only if
  Phase C uses them. Keep confidence boost/decay, supersede, reinforcement.

### Phase E - Feedback loop closes via graph
- On inject + actual use, reinforce surfaced memories and strengthen the
  RelatesTo edges among co-used memories (already partly there). Once Phase B
  reads those edges, this feedback finally affects future recall.

## Cost to quantify first (before Phase D decision)
- Per maintenance cycle: # sidecar LLM calls (cluster naming), # graph load+save
  round-trips, bytes rewritten. Add a counter / log and measure on the real
  `~/.jcode/memory` graphs.

## Dependencies / ordering
- recall-1 (eval harness) gates B/C/D measurement.
- Phase A is independent and safe to do first (pure correctness win).
- Phases B/C should be measured against the harness; otherwise we're guessing.
```mermaid
graph LR
  A[A: supersede authoritative] --> B[B: 1-hop expansion]
  H[recall-1 harness] --> B
  B --> C[C: graph dedup]
  H --> D[D: trim/repurpose maintenance]
  C --> E[E: feedback via edges]
```
