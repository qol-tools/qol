#!/usr/bin/env python3
"""Two-stage retrieval simulation from existing dense dumps and BM25 reports.

Layer A (user units) is the precision layer; layer B (assistant units) is a
recall net consulted only when layer A misses. Hit@k = target in A-top-k OR
(target not in A-top-k AND target in B-top-k). Costs zero embedding time.

Usage:
  python3 twostage.py --dense-user <dump> --dense-ua <dump> \
      --bm25-user <report.json> --bm25-assistant <report.json> \
      --questions <questions.json> --snapshot <snapshot.jsonl>
"""
import argparse
import json
import os


def load_units(snapshot_path):
    kinds = {}
    for line in open(snapshot_path):
        u = json.loads(line)
        kinds[u["key"]] = u["kind"]
    return kinds


def layer_ranks(dump, kinds, wanted_kind):
    out = {}
    for qid, ranked in dump.items():
        out[qid] = [key for key, _ in ranked if kinds.get(key) == wanted_kind]
    return out


def bm25_ranks(report_path):
    out = {}
    if not report_path or not os.path.exists(report_path):
        return out
    for row in json.load(open(report_path))["results"]:
        out[row["id"]] = row["ranks"]["bm25"]
    return out


def evaluate(questions, user_ranks, assistant_ranks, k=5):
    rows = []
    for q in questions:
        if not q["covered"] or not q.get("target_key"):
            continue
        target = q["target_key"]
        if isinstance(user_ranks.get(q["id"]), int):
            a_hit = 0 <= user_ranks.get(q["id"], -1) < k
            b_hit = 0 <= assistant_ranks.get(q["id"], -1) < k
            a_rank = user_ranks.get(q["id"], -1)
            b_rank = assistant_ranks.get(q["id"], -1)
        else:
            a = user_ranks.get(q["id"], [])
            b = assistant_ranks.get(q["id"], [])
            a_hit = target in a[:k]
            b_hit = target in b[:k]
            a_rank = a.index(target) if target in a else -1
            b_rank = b.index(target) if target in b else -1
        two = a_hit or (not a_hit and b_hit)
        rows.append(
            {
                "id": q["id"],
                "category": q.get("category", "?"),
                "a_rank": a_rank,
                "b_rank": b_rank,
                "a_hit": a_hit,
                "b_hit": b_hit,
                "two_stage": two,
            }
        )
    return rows


def summarize(rows, label):
    n = len(rows)
    a = sum(1 for r in rows if r["a_hit"])
    b = sum(1 for r in rows if r["b_hit"])
    t = sum(1 for r in rows if r["two_stage"])
    saved = t - a
    print(f"{label}: n={n} layerA={a} layerB={b} twoStage={t} savedByB={saved}")
    return {"n": n, "layerA": a, "layerB": b, "twoStage": t, "savedByB": saved}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dense-user", default="reports/qol-memory/eval/dense-scores.json")
    ap.add_argument("--dense-ua", default="reports/qol-memory/eval/dense-scores-ua2.json")
    ap.add_argument("--bm25-user", default="")
    ap.add_argument("--bm25-assistant", default="")
    ap.add_argument("--questions", default="docs/research/qol-memory/eval/questions.json")
    ap.add_argument("--snapshot", default="")
    ap.add_argument("--k", type=int, default=5)
    args = ap.parse_args()

    if not args.snapshot and (args.dense_ua or args.bm25_assistant):
        print("error: --snapshot required for layer filtering")
        return
    kinds = load_units(args.snapshot) if args.snapshot else {}

    questions = json.load(open(args.questions))["questions"]

    if args.dense_user and args.dense_ua:
        du = json.load(open(args.dense_user))
        dua = json.load(open(args.dense_ua))
        a = layer_ranks(dua, kinds, "user")
        b = layer_ranks(dua, kinds, "assistant")
        summarize(evaluate(questions, a, b, args.k), f"dense @{args.k}")
        for cat in ["context", "decision", "file"]:
            sub = [q for q in questions if q.get("category") == cat and q["covered"]]
            if sub:
                summarize(
                    evaluate(sub, a, b, args.k), f"dense {cat} @{args.k}"
                )
        rows = evaluate(questions, a, b, args.k)
        for r in rows:
            if r["two_stage"] and not r["a_hit"]:
                q = next(q for q in questions if q["id"] == r["id"])
                print(f"  saved by B: {r['id']} ({r['category']}) :: {q['query'][:60]}")

    if args.bm25_user and args.bm25_assistant:
        a = bm25_ranks(args.bm25_user)
        b = bm25_ranks(args.bm25_assistant)
        summarize(evaluate(questions, a, b, args.k), f"bm25 @{args.k}")


if __name__ == "__main__":
    main()
