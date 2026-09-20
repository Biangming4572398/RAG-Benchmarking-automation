# Benchmark catalog expansion

Checked 2026-09-20 against the publishers' repositories, dataset cards, and
evaluation code. These are catalog candidates; adding a catalog entry does not
implement its loader or official scoring protocol. No generation or judge calls
were made for this research. TempRAGEval appears once.

## Sources and versions

The revisions below were resolved using the publishers' public Hugging Face
metadata. Pin the actual downloaded files and record their checksums when
implementing preparation. Licenses describe the dataset cards; bundled or
downloaded third-party material can retain separate terms.

| Benchmark | Dataset repository and verified revision | Dataset license | Access / format |
| --- | --- | --- | --- |
| LongMemEval — cleaned | [xiaowu0162/longmemeval-cleaned](https://huggingface.co/api/datasets/xiaowu0162/longmemeval-cleaned), `98d7416c24c778c2fee6e6f3006e7a073259d48f` | MIT | Public JSON arrays; S, M, oracle variants |
| TempRAGEval | [siyue/TempRAGEval](https://huggingface.co/api/datasets/siyue/TempRAGEval), `7990c282025ae18623b14b906971f1bdb6690096` | Apache-2.0 | Gated `test.csv`; publisher conditions must be accepted |
| QASPER | [allenai/qasper](https://huggingface.co/api/datasets/allenai/qasper), `fdc9d8214fbab5dd782958601db4d678e6934a54` | CC BY 4.0 | Public v0.3 JSON archives referenced by the loader |
| AbstentionBench | [facebook/AbstentionBench](https://huggingface.co/api/datasets/facebook/AbstentionBench), `af06080e2cbfff73f02a7c33135e5567d6f7ce6a` | CC BY-NC 4.0 | Public Python loader; downloads multiple external datasets |
| MultiHop-RAG | [yixuantt/MultiHopRAG](https://huggingface.co/api/datasets/yixuantt/MultiHopRAG), `71ac0d0bd1f951d2d6b70311f7d2ae404e1ffa82` | ODC-BY | Public question JSON and separate corpus JSON |
| RAGBench | [galileo-ai/ragbench](https://huggingface.co/api/datasets/galileo-ai/ragbench), `97808f3e5fd16ede40bbff6c2949af8139b2eb7b` | CC BY 4.0 | Public Parquet, twelve configurations, train/validation/test |

## LongMemEval — cleaned

**The cleaned variant is an official release.** The authors announced the history
cleanup in September 2025 and link the cleaned repository directly. Its card
states that it replaces the original dataset by removing interfering history
sessions. Select **S-cleaned**, not the easier oracle history. The exact HF split
name is `longmemeval_s_cleaned`.
[Author announcement](https://github.com/xiaowu0162/LongMemEval/blob/9e0b455f4ef0e2ab8f2e582289761153549043fc/README.md),
[cleaned dataset card](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned).

Canonical files are
[S-cleaned](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/98d7416c24c778c2fee6e6f3006e7a073259d48f/longmemeval_s_cleaned.json),
[M-cleaned](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/98d7416c24c778c2fee6e6f3006e7a073259d48f/longmemeval_m_cleaned.json), and
[oracle](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned/resolve/98d7416c24c778c2fee6e6f3006e7a073259d48f/longmemeval_oracle.json).
Records contain `question_id`, `question_type`, `question`, `answer`,
`question_date`, aligned `haystack_session_ids` / `haystack_dates` /
`haystack_sessions`, and `answer_session_ids`. A session contains turns with
`role`, `content`, and optional `has_answer`. The raw `answer` column mixes
strings and numbers, which currently breaks the HF viewer's inferred schema.
Read JSON directly and normalize numeric references explicitly.
[Format](https://github.com/xiaowu0162/LongMemEval/blob/9e0b455f4ef0e2ab8f2e582289761153549043fc/README.md#-dataset-format),
[viewer schema error](https://huggingface.co/datasets/xiaowu0162/longmemeval-cleaned).

The official answer evaluator uses question-type-specific correctness judging,
including separate abstention handling for IDs ending `_abs`. Its GPT-4o option
is `gpt-4o-2024-08-06`; it also supports other declared judges. This is not token
F1. Preserve judge identity, prompt/version, and all case outcomes.
[Official evaluator](https://github.com/xiaowu0162/LongMemEval/blob/9e0b455f4ef0e2ab8f2e582289761153549043fc/src/evaluation/evaluate_qa.py).

Integration: isolate each question's complete supplied history, preserve session
dates and the question date, and keep `has_answer` / `answer_session_ids` as
evaluation annotations outside indexed text. Indexing only evidence sessions
would turn the experiment into an oracle setting. A small subset with manual
review must be labeled accordingly.

## TempRAGEval

The canonical file is
[`test.csv`](https://huggingface.co/datasets/siyue/TempRAGEval/blob/7990c282025ae18623b14b906971f1bdb6690096/test.csv).
The card is public, but file access requires agreeing to the publisher's
contact-sharing conditions. The raw CSV was not accessed during this research,
so its complete column serialization remains unverified.
[Access conditions and dataset card](https://huggingface.co/datasets/siyue/TempRAGEval).

The official reader consumes prepared records containing `question`, `answers`,
`time_relation`, and `gold_evidences`, with retrieved contexts carrying `title`
and `text`. It excludes empty `time_relation` records and computes normalized
answer exact match and maximum token F1 over references. The paper also reports
answer recall and gold-evidence recall for retrieval.
[Pinned reader](https://github.com/siyue-zhang/MRAG/blob/19f3bcf9a365f9379e12edc13bec96c9ec557e1a/reader.py),
[paper](https://aclanthology.org/2025.findings-emnlp.167/).

The intended corpus is ATLAS `enwiki-dec2021`, approximately 33.1 million chunks.
Up to two gold sentences label evidence; they are not the candidate corpus.
Preparation needs the accepted dataset and the associated corpus or a clearly
named alternate corpus experiment. Keep this entry integration-required until
that setup exists. The code repository's MIT license is distinct from the
dataset card's Apache-2.0 declaration.
[Official repository](https://github.com/siyue-zhang/MRAG/tree/19f3bcf9a365f9379e12edc13bec96c9ec557e1a).

## QASPER

The publisher's loader names v0.3 and downloads
[train/development](https://qasper-dataset.s3.us-west-2.amazonaws.com/qasper-train-dev-v0.3.tgz)
and [test/evaluator](https://qasper-dataset.s3.us-west-2.amazonaws.com/qasper-test-and-evaluator-v0.3.tgz)
archives. Members are `qasper-train-v0.3.json`, `qasper-dev-v0.3.json`, and
`qasper-test-v0.3.json`. HF maps development to `validation`.
[Pinned official loader](https://huggingface.co/datasets/allenai/qasper/blob/fdc9d8214fbab5dd782958601db4d678e6934a54/qasper.py).

Raw JSON maps paper IDs to title, abstract, section/paragraph `full_text`, figures
and tables, and `qas`. Each question has multiple annotations under
`answers[].answer`: `unanswerable`, `extractive_spans`, nullable `yes_no`,
`free_form_answer`, `evidence`, and `highlighted_evidence`. Candidate material is
the paper, not its gold evidence excerpts.
[Data schema](https://huggingface.co/datasets/allenai/qasper/blob/fdc9d8214fbab5dd782958601db4d678e6934a54/qasper.py).

Official evaluation reports answer token F1 and paragraph evidence F1, taking
the best reference score per question. It joins extractive spans, normalizes
boolean answers, uses `Unanswerable` for that answer type, and counts missing
predictions as zero. Figure/table evidence requires explicit handling; the
official script offers a text-evidence-only option.
[Official evaluator](https://github.com/allenai/qasper-led-baseline/blob/main/scripts/evaluator.py).

Integration: index paper text once and scope each question to its paper; retain
all reference annotations outside the corpus. Declare any exclusion of visual
evidence. Existing HotpotQA scoring cannot be relabeled QASPER scoring.

## AbstentionBench

The publisher releases a suite spanning twenty datasets, rather than one shared
RAG corpus. Its pinned
[`AbstentionBench.py`](https://huggingface.co/datasets/facebook/AbstentionBench/blob/af06080e2cbfff73f02a7c33135e5567d6f7ce6a/AbstentionBench.py)
and [`data.py`](https://huggingface.co/datasets/facebook/AbstentionBench/blob/af06080e2cbfff73f02a7c33135e5567d6f7ce6a/data.py)
produce `question`, nullable `reference_answers`, `should_abstain`, and
`metadata_json` (a JSON-encoded string). Context and instructions can already be
embedded in `question`. Component-specific splits include `qasper`, `squad2`,
`musique`, and `gpqa_abstain`; `suite` is a catalog label, not an upstream split.

The official loading instructions require `datasets <= 3.6.0` and remote-code
loading. The loader fetches external sources; a public suite repository does not
guarantee every component is accessible. Its card explicitly preserves
third-party licensing requirements in addition to CC BY-NC 4.0.
[Dataset card](https://huggingface.co/datasets/facebook/AbstentionBench/blob/af06080e2cbfff73f02a7c33135e5567d6f7ce6a/README.md).

Evaluation separates abstention detection, whether abstention matches the
expected label, and answer correctness. The official pipeline includes both
keyword detection and configured LLM judges; the selected method must be
recorded. Refusing an unanswerable question can be correct.
[Official evaluation code](https://github.com/facebookresearch/AbstentionBench/blob/e29184174c69bc95139b8c33dcca09aacab9442a/recipe/evaluation.py).

Integration: select explicit components and preserve their prompting protocol.
Do not index the question as artificial evidence, invent a retrieval corpus, or
reuse the current “only answered cases can be reviewed” workflow as a complete
abstention evaluator. This requires a distinct evaluation integration.

## MultiHop-RAG

Canonical files are
[`MultiHopRAG.json`](https://huggingface.co/datasets/yixuantt/MultiHopRAG/resolve/71ac0d0bd1f951d2d6b70311f7d2ae404e1ffa82/MultiHopRAG.json)
and [`corpus.json`](https://huggingface.co/datasets/yixuantt/MultiHopRAG/resolve/71ac0d0bd1f951d2d6b70311f7d2ae404e1ffa82/corpus.json).
HF configurations are `MultiHopRAG` and `corpus`; the question configuration is
exposed under the split name `train` even though this is an evaluation corpus.
Questions carry `query`, `answer`, `question_type`, and `evidence_list` with gold
facts and article metadata. Types include inference, comparison, temporal, and
`null_query`; null questions have no supporting evidence.
[Publisher dataset](https://huggingface.co/datasets/yixuantt/MultiHopRAG).

The official retrieval script reports Hits@4/10, MAP@10 and MRR@10, excluding
`null_query` from retrieval evaluation. The current QA script counts a prediction
as successful when its lowercased tokens intersect the gold answer's tokens,
after optional answer extraction. Its printed F1 is therefore not HotpotQA token
F1. Pin and name the evaluator before claiming reproducibility.
[Retrieval evaluator](https://github.com/yixuantt/MultiHop-RAG/blob/c1c1287aa60a94acf9c4d20c891c9cd611a0f6e8/retrieval_evaluate.py),
[QA evaluator](https://github.com/yixuantt/MultiHop-RAG/blob/c1c1287aa60a94acf9c4d20c891c9cd611a0f6e8/qa_evaluate.py).

Integration: index the separate news corpus and preserve publication/source
metadata. Gold facts identify relevant passages; restricting candidates to those
facts would be an oracle experiment. Preserve null questions for generation and
abstention analysis even if retrieval metrics omit them.

## RAGBench

The older `rungalileo/ragbench` dataset URL now redirects to
`galileo-ai/ragbench`. Each configuration has Parquet files such as
[`hotpotqa/test-00000-of-00001.parquet`](https://huggingface.co/datasets/galileo-ai/ragbench/resolve/97808f3e5fd16ede40bbff6c2949af8139b2eb7b/hotpotqa/test-00000-of-00001.parquet).
Configurations include `covidqa`, `cuad`, `delucionqa`, `emanual`, `expertqa`,
`finqa`, `hagrid`, `hotpotqa`, `msmarco`, `pubmedqa`, `tatqa`, and `techqa`.
[Publisher metadata and schema](https://huggingface.co/api/datasets/galileo-ai/ragbench).

Rows contain `id`, `question`, candidate `documents`, historical `response`,
generation/annotation model names, sentence keys/support explanations, and
adherence, relevance, utilization, and completeness labels. These labels apply
to the supplied historical response. `response` is not a guaranteed gold answer.
[Dataset card and examples](https://huggingface.co/datasets/galileo-ai/ragbench).

The paper evaluates RAG evaluators using TRACe annotations. The reproduction
script compares evaluator predictions using hallucination AUROC and relevance /
utilization RMSE; it does not simply score newly generated answers against the
stored response string.
[Paper](https://arxiv.org/abs/2407.11005),
[Metric reproduction script](https://github.com/rungalileo/ragbench/blob/c28e6c22fc858086468eabb274250e27b5a8e9d8/ragbench/calculate_metrics.py).

Integration: select configuration/split explicitly, use only `documents` as
candidate evidence, and create fresh evaluation annotations for new Kimi
outputs. Keep historical labels and model provenance as separate reference
metadata. A manually reviewed generation subset is useful, but it is not an
official RAGBench evaluator score.
