# qol voice: porting the prototype loop into the plugin

Goal: qol voice runs the whole spoken loop the prototype proved, with no Claude Code hook and no Python.
You speak, the turn lands in the agent pane you were in when you started speaking, the agent's reply is read aloud, and talking over the reply stops it.
The directed-speech gate from [the gate plan](2026-11-21-voice-directed-speech.md) comes after this port, and it plugs into the stage this plan leaves for it.

## What the plugin already has

Capture, recognition, the pure turn coordinator and delivery through terminal-sessions are all in place.
The turn policy already emits playback effects (duck, pause, resume, cancel) and synthesis cancel, but the worker only logs them.
The `sherpa-onnx` crate the plugin already depends on ships Kokoro speech synthesis, Silero voice detection and a CUDA execution provider, so the port needs no new engine dependency.

## What the prototype proved, and the plugin lacks

| Prototype behavior | Plugin today |
|---|---|
| reply read aloud when the agent's turn ends | no speech output at all |
| talking stops playback, also when playback starts mid-speech | effects emitted, never performed |
| turn goes to the agent pane active when speech started | one fixed target chosen in Settings |
| delivery in about 80 ms | a session lookup before every write |
| Silero voice detection, 1.5 s end of turn | loudness threshold, 700 ms |
| Whisper large-v3-turbo on the GPU | Whisper tiny on the CPU, or sherpa on the CPU |
| filler dropped, text shaped like typing | sent as recognized |
| every transcript recorded for labelling | in-memory list only |

## Steps

Each step is its own pull request and leaves the plugin usable.

1. Speech output.
   A synthesis provider registry beside the transcription one, with Kokoro through `sherpa-onnx` as the first local provider.
   Sentence streaming, so the first sentence plays while the rest is synthesized.
   A playback stream in `libs/audio` with a buffer of at least 200 ms, since the prototype crackled on shorter ones.
   Voice and speed are settings; the prototype settled on speed 1.6.
2. Reply detection without an agent hook.
   Voice already subscribes to the target session's screen; it learns the turn ended when the session's status becomes your-turn.
   The reply text comes from the tool's interpretation strategy in cli-sessions, as a new "last reply" capability, so voice never reads an agent's files directly.
   Only sessions that received a voice turn are spoken, and only the most recent reply.
3. Barge-in.
   The worker performs the policy's playback and synthesis effects.
   The policy also cancels playback when it starts while speech is already in progress, which the prototype needed a second fix for.
4. Routing to the active agent pane.
   A routing mode that resolves the target at speech onset: the last-active pane in the terminal that runs an agent, whether or not the terminal has OS focus.
   The fixed target stays as the other mode.
   Delivery drops the per-write session lookup to get under 100 ms.
5. Live transcript indicator.
   When the target pane is not the focused window, a `qol_gpui` toast shows what is being heard, then where it was sent or why it was dropped.
6. Recognition upgrade.
   Silero voice detection through `sherpa-onnx`, 1.5 s end of turn as the default, and Whisper large-v3-turbo on the CUDA provider where a GPU is present.
   A GPU out of memory error is typed and falls back to the CPU model.
7. Text shaping.
   A filler filter (mmm, uh, hmm) and typed style (lowercase first letter, no trailing period), as a stage between recognition and routing.
8. Transcript recording.
   Every transcript and its routing outcome is recorded in the plugin's data folder, with `qol voice review`, `qol voice yes <id>` and `qol voice no <id>` for labelling.
   This is the dataset the gate plan trains on.

## Open questions

- Which terminals beyond kitty can report the last-active pane; the first version supports kitty only and says so in doctor.
- Whether the toast should also offer to resend a dropped turn.
- How replies with code blocks are read aloud; the prototype reads everything.
