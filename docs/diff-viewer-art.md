# The diff viewer art direction

Status: contract draft on the `diff-viewer` branch, awaiting owner verdict.

## Thesis

The diff viewer is not a diff viewer.
It is an instrument that renders a repository's change as motion and heat.
Text is the ground truth the visuals are computed from, and every visual element folds open to the exact text that produced it.
The classic two-column diff remains only as a reading mode, never as the primary experience.

## The mapping contract

Every visual element corresponds one to one with a real artifact: a token, a line, a hunk, a file, or a commit.
Every visual attribute encodes a real property: heat is recency, amplitude is change volume, displacement is the add/remove direction, rhythm is commit cadence, shape is code structure.
Text is one keypress away at every visual locus.
Nothing is decorative: an element that encodes no data does not exist.

## The matter grammar

Heat is the single identity of the product.
The ramp is fixed: deep blue (oldest, coldest) through ember to white-hot (the present).
Heat decays with real time: hot cools to warm after 60 seconds, warm to cool after 300.
Cool never warms.
Change events are physical: an addition ignites, a deletion evaporates, a modification wounds and heals.
At rest, hot matter breathes with a slow ember pulse.
The field is never static.

## The three surfaces

The Timeline: the filmstrip becomes a playable heat field, a piano roll where each note is a commit.
Space plays history; the strikes drive the other surfaces.

The Wave: the file renders as a continuous waveform.
Context is a flat baseline, additions rise above it, deletions hang below, dense edits ripple, heat colors the crests.
A keypress folds the wave open into the exact text at the playhead.
This is the surface where change is a shape, not text.

The Code: the text surface, rendered as living matter.
Tokens ignite, evaporate, and heal; the reader reads the change by watching matter move.
This is the microscope view of the same field.

## The summon script

The user summons and the field breathes.
Press space: history plays, the timeline strikes, the wave dances, the code ignites.
Dive anywhere: fold open the text and read exactly what changed.

## What we stop doing

We stop shipping animations onto the two-column frame and calling it new.
The two-column view stays as the precise reading mode it always was, reachable with one key, unchanged in behavior.

## Build order

R2: the Wave surface.
R3: the Score, play as the primary interaction.
R4: the Panorama garnish, the code's structure as terrain, only if the wave and score hold.
