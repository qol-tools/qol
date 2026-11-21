# qol voice: an always-on mic that only sends what was meant for the session

Goal: talk freely with the mic always on, and have only the speech meant for the bound session reach it, within half a second of the end of the sentence.
Delivery keeps today's model: qol voice hands a completed turn to any live terminal session through the shared terminal-sessions library.
What changes is a device-directed speech gate between transcription and routing, a delivery path fast enough for speech, and a dataset of the user's own speech that trains the gate.

A Python prototype proves the loop first: `/media/kmrh47/WD_SN850X/ai/tmp/voice-poc`, started with `voice start` from the Claude Code window it binds to.
It is the place to settle what works; qol voice takes the result once it does.
Every figure below was measured there on 2026-11-21, on Linux Mint with an RTX 3080 (10 GB).

## Where the time went

| Step | Measured | Note |
|---|---|---|
| end of speech, silence wait | 700 ms | `SILENCE_MS` |
| Whisper large-v3-turbo, GPU | 150-350 ms | 2.3 GB of GPU memory |
| delivery via `qol sessions send --submit` | about 3,600 ms | every transcript |
| delivery via `kitten @ send-text` plus `send-key enter` | about 80 ms | what the prototype uses now |
| Kokoro TTS, end of reply to first audio | 140-240 ms | 1.1 GB; speed 1.3 |

Delivery was the whole delay.
The upgrade needs a sessions delivery path that lands a turn in under 100 ms; the 3.6 s path is unusable for speech.

## The gate: options measured

The gate answers one question per transcript: was this said to the assistant, or to someone else, to oneself, or by the room?
The term for this is device-directed speech detection.

| Gate | Per sentence | Memory | Verdict |
|---|---|---|---|
| Claude Haiku, low effort, one `claude -p` per check | 4,200-4,600 ms | none local | too slow |
| Claude Haiku, low effort, one long-lived `claude -p --input-format stream-json` | 1,000-3,000 ms | none local | right answers, too slow |
| strands-decider 2B (Apache-2.0), PyTorch bf16, GPU | 60 ms | 5.9 GB GPU | does not fit beside a game |
| strands-decider 2B ONNX 8-bit, GPU | 27 ms | 2.3 GB GPU | fits only with no game running |
| strands-decider 2B ONNX 4-bit, CPU | about 500 ms | 1.8 GB RAM, no GPU | the everyday target |
| Gemini Decision 1 | not measured | API only, no open weights | ruled out for local use |

Haiku through the subscription is slow because every check is a full Claude Code turn; a direct API call would be about 0.5 s but bills outside the subscription.
The decider is a decision model: one forward pass per question, with a calibrated probability instead of generated text.

Off the shelf the decider is only roughly right.
On 16 real sentences marked by hand, the 4-bit and 8-bit versions both got 14 right at their best cut-off.
It drops short follow-ups such as "Hello?" and "Why is everything being blocked now?" (0.36 to 0.42), and it passes "Hey, Moxie." (0.50), which was said to a pet.
A sentence aimed at the assistant about the gate, such as "Do not send this.", passes by design; a script check for such phrases has to run before the model.

## Plan

1. Collect. The prototype runs with the gate off (`DECIDER_ENABLED = False`), sends every transcript except pure filler, and records each one in `logs/gate.jsonl`.
2. Label. Go through the records together; `voice review` lists unmarked rows and `voice yes <id>` and `voice no <id>` append to `logs/gate-labels.jsonl`. Follow-ups such as repeating a dropped sentence or saying "that wasn't for you" are labels too.
3. Train. Fine-tune the decider's LoRA adapter on the labelled rows at full precision, or with QLoRA in about 4 GB, while the GPU is otherwise free. Shrinking the result back to 4-bit costs about one point of accuracy on the model card (58.4% against 59.4%).
4. Run. Serve the fine-tuned 4-bit model on the CPU, so the gate never competes with a game for GPU memory, and turn the gate back on.
5. Port to qol voice, once steps 1 to 4 settle what works:
   - a directed-speech stage between transcription and routing, registered and described like a transcription provider, local only;
   - its decision as a typed effect of the turn policy, so the turn coordinator stays pure;
   - fast delivery to any live session through the shared terminal-sessions library;
   - transcript recording and the labelling commands as part of the plugin, with the dataset kept on the device;
   - script prefilters for filler and for explicit "don't send this" phrases, ahead of the model.

## Open problems

- Barge-in stops the reply's speech as soon as the mic hears the user, before the gate has decided, so talking to someone else still cuts the reply off.
- Speech starts only when the whole turn ends, tool calls included; speaking each paragraph as it is written needs a hook that fires mid-turn.
- Whisper crashed with CUDA out of memory while a game held 3.8 GB; it needs a typed failure and a fallback to a smaller or int8 model, or the CPU.

## Prototype log, 2026-11-21 (local time, from the prototype's `logs/timings.log` and file times)

| Time | Step | Result |
|---|---|---|
| 09:44-09:46 | Kokoro TTS server, a Claude Code Stop hook that speaks each reply, and a Markdown-to-speech cleaner | first reply spoken at 09:46:49, 87 ms after the turn ended |
| 09:47-09:49 | Mic loop: Silero voice detection, Whisper large-v3-turbo on the GPU, dry runs without delivery | transcripts in 150-350 ms; barge-in stopped playback at 09:48:56 |
| 09:53 | `voice start` bound the loop to the Claude Code window; delivery through `qol sessions send --submit` | loop works end to end; 3.6 s per delivery, so the user repeated "Hello?" before the reply came |
| after 09:54 | Mic crashed with CUDA out of memory while KCD2 held 3.8 GB | the loop went silent when the user switched to another app; the crash, not the focus change, was the cause |
| 10:00-10:03 | Timed Claude Haiku as an intent gate on the subscription | 4.2-4.6 s per `claude -p` call; 1.4-1.6 s per check in one long-lived stream-json process |
| 10:04 | Haiku gate at low effort in the loop; TTS speed 1.3 | right verdicts on test lines, 1-3 s per check |
| 10:06 | Delivery switched to `kitten @ send-text` plus `send-key enter` | 80-86 ms per delivery, first live one at 10:08:09 |
| 10:12-10:20 | Looked for a decision model: Gemini Decision 1 is API only; strands-decider is Apache-2.0 on Hugging Face; installed it | |
| 10:20-10:28 | strands-decider 2B in PyTorch | CPU 1-2 s and 8.2 GB RAM; GPU 60 ms and 5.9 GB, which fit only with the mic stopped |
| 10:30-10:32 | ONNX builds of the decider (8-bit 1.8 GB, 4-bit 1.1 GB) with ONNX Runtime on CUDA 13 | 27 ms per sentence, 2.3 GB of GPU memory |
| 10:33:45 | Decider replaced the Haiku gate as a local server, cut-off 0.45 | 26-42 ms per check; "Hey honey, have you thought about the trailer?" dropped at 0.28 |
| 10:35:46 | Cut-off lowered to 0.33 | four real messages had been dropped at 0.36-0.42 |
| 10:38-10:39 | Transcript records and the `voice review`, `voice yes`, `voice no` commands; 16 earlier verdicts backfilled and marked | the decider had 8 of 16 wrong at its live cut-offs |
| 10:40 | 4-bit decider on the CPU against the 16 marked sentences | about 500 ms, 1.8 GB RAM, no GPU memory; 14 of 16 right at its best cut-off, as for 8-bit |
| 10:41 | Decider turned off; every transcript is sent and recorded | collection for the training set started |
