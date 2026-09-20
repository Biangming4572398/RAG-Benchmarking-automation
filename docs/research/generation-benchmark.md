# First generated-answer benchmark: HotpotQA

Researched 2026-09-20. The HotpotQA adapter and automatic answer evaluator are now
implemented in this submodule; this note records the benchmark choice and limits.

Choose **HotpotQA, development split, distractor setting** for the first
automatically scored generation experiment. It asks questions requiring reasoning
across Wikipedia passages, with reference answers and sentence-level supporting
facts. This makes the final generated answer a measurable output of the RAG
architecture and generation model pairing. The local embedding model remains
part of the pipeline being tested. [Original paper](https://aclanthology.org/D18-1259/)

The authors supply ten candidate paragraphs per question in the distractor
setting, including the relevant evidence. The development data can be downloaded
from the [official project page](https://hotpotqa.github.io/). This is a practical
first fit for Nebula's existing local Markdown corpus: no live Wikipedia crawl
is needed. Use a pinned, deterministic development subset for early experiments;
label its results as subset results, not the official full benchmark score.

## Data and scoring

The development JSON contains `_id`, `question`, `answer`, `context` as titled
sentence lists, and `supporting_facts` as title/sentence-index pairs. The public
test set omits answers and supporting facts, so use development data for local
automatic scoring. Dataset licensing is CC BY-SA 4.0; the authors' code is
Apache-2.0. Preserve attribution and the downloaded dataset's provenance.
[Author repository and format](https://github.com/hotpotqa/hotpot#json-format)

Start with **answer exact match and token F1**. The official evaluator lowercases,
removes punctuation/articles, normalizes whitespace, and compares the prediction
with the gold answer. It handles yes/no answers explicitly. These scores require
no extra model or paid judge. Supporting-fact and joint scores are also defined,
but require sentence identities, not just document citations.
[Official evaluator](https://raw.githubusercontent.com/hotpotqa/hotpot/master/hotpot_evaluate_v1.py)

These are answer-matching metrics, not general hallucination or semantic-quality
scores. In particular, a correct answer embedded in a long explanation may have
low token F1 or fail exact match. An integration should request and retain a
separate concise final answer, keep evidence/explanations independently, and
version any extraction or normalization policy. Score against all selected cases
and report failed/refused/answered coverage; do not inflate results by averaging
only successful responses. Human groundedness reviews can remain separate.

## Implemented contract

The YAML catalog now includes `hotpotqa` alongside RAGTruth, with a
pinned source checksum and a default 100-case development subset. The JSON loader
stores reference answers and supporting facts outside exported Markdown. It
preserves each question's supplied candidate passages (typically ten) for separate conversation scopes.
The `hotpotqa_answer_v1` evaluator scores the full returned answer using official
normalization; it performs no gold-guided extraction and makes no judge-model call.

Nebula's current strict mode quotes evidence lines. A correct short fact embedded
in a longer quote can therefore receive low EM/F1. This integration measures that
actual output policy; it does not claim the concise-answer protocol suggested
above is implemented. Supporting-fact/joint scoring remains future work.

Summary and per-case scores, reference answers, CSV, and the Generated answers
table carry the automatic results. Averages cover the whole selected subset,
including zero for missing answers; manual reviews remain separate. Fully
processed completed runs can be compared without human review. Snapshot
fingerprints cover references and candidate selections, and comparison also
requires matching top-k, evaluator and source sets.

## Alternative considered

Google's FRAMES also directly targets generated-answer factuality and multi-hop
RAG reasoning: 824 questions with gold answers and relevant Wikipedia links.
Its released fields contain links rather than a bundled passage corpus. Preparing
stable local article snapshots adds work and introduces revision choices, so
HotpotQA is the simpler first integration here. This is an engineering judgment,
not a claim that HotpotQA is a more comprehensive benchmark.
[Google's FRAMES dataset card](https://huggingface.co/datasets/google/frames-benchmark)
